//! Keys, keyed index entries, and authenticated encryption.
//!
//! Each match domain has two keys derived from the server's master key:
//! an index key (HMAC-SHA-256), so equal values in one domain have equal
//! index entries and values in different domains cannot be compared, and an
//! encryption key (XChaCha20-Poly1305), with the domain as associated data.
//! Keys never leave the server process.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// The master key. It can be replaced in place (key rotation): every function and plan
/// that holds the keyring then uses the new key.
pub struct Keyring {
    master: std::sync::RwLock<[u8; 32]>,
}

impl std::fmt::Debug for Keyring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Keyring(..)")
    }
}

impl Keyring {
    /// A keyring with a fresh random master key.
    pub fn random() -> Self {
        let mut master = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut master);
        Keyring { master: std::sync::RwLock::new(master) }
    }

    pub fn from_bytes(master: [u8; 32]) -> Self {
        Keyring { master: std::sync::RwLock::new(master) }
    }

    /// A fresh random key, as bytes.
    pub fn random_bytes() -> [u8; 32] {
        let mut k = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut k);
        k
    }

    /// A copy of the current key, for re-encrypting with another one.
    pub fn snapshot(&self) -> Keyring {
        Keyring::from_bytes(*self.master.read().unwrap())
    }

    pub fn replace(&self, master: [u8; 32]) {
        *self.master.write().unwrap() = master;
    }

    pub fn master_hex(&self) -> String {
        hex::encode(*self.master.read().unwrap())
    }

    /// Identifies the key without revealing it, so a data directory can tell whether it was
    /// given the key its data was written with.
    pub fn fingerprint(&self) -> String {
        hex::encode(&self.derive("fingerprint", "")[..16])
    }

    fn derive(&self, purpose: &str, domain: &str) -> [u8; 32] {
        let master = *self.master.read().unwrap();
        let mut m = <HmacSha256 as Mac>::new_from_slice(&master).expect("key length");
        m.update(purpose.as_bytes());
        m.update(&[0]);
        m.update(domain.as_bytes());
        m.finalize().into_bytes().into()
    }

    /// The index entry of a value in a match domain: `ie1:` + 128 bits of HMAC, hex.
    pub fn index_entry(&self, domain: &str, value: &str) -> String {
        let k = self.derive("index", domain);
        let mut m = <HmacSha256 as Mac>::new_from_slice(&k).expect("key length");
        m.update(value.as_bytes());
        let tag = m.finalize().into_bytes();
        format!("ie1:{}", hex::encode(&tag[..16]))
    }

    /// Encrypts a value: `ct1:` + base64(nonce || ciphertext), with the domain as associated data.
    pub fn encrypt(&self, domain: &str, value: &str) -> String {
        let k = self.derive("aead", domain);
        let cipher = XChaCha20Poly1305::new((&k).into());
        let mut nonce = [0u8; 24];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ct = cipher.encrypt(XNonce::from_slice(&nonce), Payload { msg: value.as_bytes(), aad: domain.as_bytes() }).expect("encryption");
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ct);
        format!("ct1:{}", B64.encode(out))
    }

    pub fn decrypt(&self, domain: &str, stored: &str) -> Option<String> {
        let raw = B64.decode(stored.strip_prefix("ct1:")?).ok()?;
        if raw.len() < 24 {
            return None;
        }
        let k = self.derive("aead", domain);
        let cipher = XChaCha20Poly1305::new((&k).into());
        let pt = cipher.decrypt(XNonce::from_slice(&raw[..24]), Payload { msg: &raw[24..], aad: domain.as_bytes() }).ok()?;
        String::from_utf8(pt).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_and_domains() {
        let k = Keyring::random();
        assert_eq!(k.index_entry("person_id", "123"), k.index_entry("person_id", "123"));
        assert_ne!(k.index_entry("person_id", "123"), k.index_entry("phone_number", "123"));
        let c = k.encrypt("person_id", "123");
        assert_ne!(c, k.encrypt("person_id", "123"));
        assert_eq!(k.decrypt("person_id", &c).as_deref(), Some("123"));
        assert_eq!(k.decrypt("phone_number", &c), None);
    }
}
