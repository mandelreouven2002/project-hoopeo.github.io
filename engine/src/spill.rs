//! Encrypted spill files.
//!
//! When a query needs more memory than the server allows it (a large sort, aggregation or
//! join), DataFusion writes part of its working data to temporary files. Those rows can be
//! anything the query computes, including values a DISCLOSES function returned in plaintext.
//! DEP Engine gives DataFusion this file factory instead of plain temporary files: every
//! file is written in chunks of up to 64 KiB, each encrypted with XChaCha20-Poly1305 under a
//! key that is made at random when the server starts and exists only in its memory. A file
//! left on disk after a crash cannot be read by anyone, and the spill directory is emptied at
//! the next start.

use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use datafusion::common::{DataFusionError, Result};
use datafusion::execution::spill_file::{SpillFile, SpillWriter, TempFileFactory};
use futures::Stream;
use rand::RngCore;

const CHUNK: usize = 64 * 1024;

pub struct EncryptedSpill {
    dir: PathBuf,
    cipher: Arc<XChaCha20Poly1305>,
    next: AtomicU64,
    /// Bytes written to spill files so far (for /metrics).
    pub written: Arc<AtomicU64>,
}

impl EncryptedSpill {
    /// A factory writing into `dir`, which it creates (mode 0700) and empties.
    pub fn new(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                if e.file_name().to_string_lossy().ends_with(".spill") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        let mut key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut key);
        Ok(EncryptedSpill { dir: dir.to_path_buf(), cipher: Arc::new(XChaCha20Poly1305::new((&key).into())), next: AtomicU64::new(1), written: Arc::new(AtomicU64::new(0)) })
    }
}

fn io(e: std::io::Error) -> DataFusionError {
    DataFusionError::IoError(e)
}

impl TempFileFactory for EncryptedSpill {
    fn create_temp_file(&self, _description: &str) -> Result<Arc<dyn SpillFile>> {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        let path = self.dir.join(format!("{}-{n}.spill", std::process::id()));
        File::create(&path).map_err(io)?;
        Ok(Arc::new(EncryptedFile { path, cipher: self.cipher.clone(), size: Arc::new(AtomicU64::new(0)), chunks: Arc::new(AtomicU64::new(0)), total: self.written.clone() }))
    }
}

struct EncryptedFile {
    path: PathBuf,
    cipher: Arc<XChaCha20Poly1305>,
    size: Arc<AtomicU64>,
    /// Chunks written so far: each chunk's number is its associated data, so chunks cannot be reordered.
    chunks: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
}

impl Drop for EncryptedFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl SpillFile for EncryptedFile {
    fn size(&self) -> Option<u64> {
        Some(self.size.load(Ordering::Relaxed))
    }

    fn read_stream(&self) -> Result<Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>>> {
        let file = BufReader::new(File::open(&self.path).map_err(io)?);
        let cipher = self.cipher.clone();
        let s = futures::stream::unfold((file, 0u64, cipher), |(mut f, i, c)| async move {
            let mut len = [0u8; 4];
            match f.read_exact(&mut len) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return None,
                Err(e) => return Some((Err(io(e)), (f, i, c))),
            }
            let n = u32::from_le_bytes(len) as usize;
            let mut nonce = [0u8; 24];
            let mut ct = vec![0u8; n];
            if let Err(e) = f.read_exact(&mut nonce).and_then(|_| f.read_exact(&mut ct)) {
                return Some((Err(io(e)), (f, i, c)));
            }
            let pt = c.decrypt(XNonce::from_slice(&nonce), Payload { msg: &ct, aad: &i.to_le_bytes() });
            let item = pt.map(Bytes::from).map_err(|_| DataFusionError::Execution("a spill file could not be decrypted".into()));
            Some((item, (f, i + 1, c)))
        });
        Ok(Box::pin(s))
    }

    fn open_writer(&self) -> Result<Box<dyn SpillWriter>> {
        let f = OpenOptions::new().append(true).open(&self.path).map_err(io)?;
        Ok(Box::new(EncryptedWriter { out: BufWriter::new(f), buf: Vec::with_capacity(CHUNK), cipher: self.cipher.clone(), chunks: self.chunks.clone(), size: self.size.clone(), total: self.total.clone() }))
    }
}

struct EncryptedWriter {
    out: BufWriter<File>,
    buf: Vec<u8>,
    cipher: Arc<XChaCha20Poly1305>,
    chunks: Arc<AtomicU64>,
    size: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
}

impl EncryptedWriter {
    fn seal(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let mut nonce = [0u8; 24];
        rand::thread_rng().fill_bytes(&mut nonce);
        let chunk = self.chunks.fetch_add(1, Ordering::Relaxed);
        let ct = self
            .cipher
            .encrypt(XNonce::from_slice(&nonce), Payload { msg: &self.buf, aad: &chunk.to_le_bytes() })
            .map_err(|_| std::io::Error::other("spill encryption failed"))?;
        self.out.write_all(&(ct.len() as u32).to_le_bytes())?;
        self.out.write_all(&nonce)?;
        self.out.write_all(&ct)?;
        let n = (4 + 24 + ct.len()) as u64;
        self.size.fetch_add(n, Ordering::Relaxed);
        self.total.fetch_add(n, Ordering::Relaxed);
        self.buf.clear();
        Ok(())
    }
}

impl Write for EncryptedWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let mut rest = data;
        while !rest.is_empty() {
            let room = CHUNK - self.buf.len();
            let n = room.min(rest.len());
            self.buf.extend_from_slice(&rest[..n]);
            rest = &rest[n..];
            if self.buf.len() == CHUNK {
                self.seal()?;
            }
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.seal()?;
        self.out.flush()
    }
}

impl SpillWriter for EncryptedWriter {
    fn finish(&mut self) -> Result<()> {
        self.flush().map_err(io)
    }
}

impl Drop for EncryptedWriter {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn spill_files_hold_no_plaintext() {
        let dir = std::env::temp_dir().join(format!("hoopeo-spill-test-{}", std::process::id()));
        let f = EncryptedSpill::new(&dir).unwrap();
        let file = f.create_temp_file("test").unwrap();
        let secret = b"national id 123456782, disclosed by a function. ".repeat(4000);
        {
            let mut w = file.open_writer().unwrap();
            w.write_all(&secret).unwrap();
            w.finish().unwrap();
        }
        let on_disk = std::fs::read_dir(&dir).unwrap().flatten().map(|e| std::fs::read(e.path()).unwrap()).next().unwrap();
        assert!(!on_disk.windows(9).any(|w| w == b"123456782"));
        let mut back = Vec::new();
        let mut s = file.read_stream().unwrap();
        while let Some(chunk) = s.next().await {
            back.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(back, secret);
        drop(s);
        drop(file);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "a spill file is deleted when dropped");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
