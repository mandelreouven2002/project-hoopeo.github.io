//! The data directory.
//!
//! ```text
//! data.redb      the database file: the catalog, every table's rows and indexes (see
//!                `store`), and the fingerprint of the master key. Protected values are
//!                there only as index entries and ciphertexts.
//! audit.redb     the audit log
//! master.key     the master key, unless --key-file puts it elsewhere
//! spill/         encrypted temporary files of queries larger than their memory (see `spill`)
//! .lock          held while a server uses the directory
//! ```
//!
//! Every commit is one durable redb write transaction. Starting the server reads only the
//! catalog: rows are read from disk when queries need them, so a database does not have to
//! fit in memory, and the server starts in the same time whatever its size.
//!
//! The audit log has a file of its own, with its own writer thread. Records of statements
//! that finish at the same time are written together in one durable commit (group commit),
//! and every statement waits for its records to be on disk before it returns.
//!
//! The master key is what turns the ciphertexts back into values. Keeping it apart from the
//! data (another disk, a secrets manager mounted as a file) means a copy of the data alone
//! reveals nothing. The data file keeps a fingerprint of the key, so a server given the
//! wrong key refuses to start instead of returning garbage.
//!
//! Older data directories are converted the first time they are opened: a 0.4 directory
//! (format 2: rows as JSON, no indexes) in place, inside data.redb; a 0.3 directory
//! (catalog.json and tables/*.arrow) into a new data.redb, with its files kept in legacy-0.3/.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use datafusion::arrow::datatypes::SchemaRef;
use datafusion::arrow::ipc::reader::FileReader;
use datafusion::arrow::record_batch::RecordBatch;
use rand::RngCore;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use crate::audit::AuditRecord;
use crate::catalog::{derive_indexes, parse_type, Catalog, ColumnMeta, IndexMeta, PathMode, Role, TableMeta, TablePriv};
use crate::crypto::Keyring;
use crate::error::{DepError, Result};
use crate::server::{DepServer, ServerOptions};
use crate::store::{db_err, encode_rows, TableInfo};

/// The current format of data.redb.
const FORMAT: u32 = 3;
pub const META: TableDefinition<&str, &[u8]> = TableDefinition::new("dep_meta");
const AUDIT: TableDefinition<u64, &[u8]> = TableDefinition::new("dep_audit");

pub struct Storage {
    pub dir: PathBuf,
    pub key_path: PathBuf,
    pub audit: Option<Arc<AuditStore>>,
    /// Held for as long as the server runs, so two servers never share a directory.
    _lock: File,
}

#[derive(Serialize, Deserialize)]
struct StoredColumn {
    name: String,
    type_name: String,
    domain: Option<String>,
    primary_key: bool,
    references: Option<(String, String)>,
    #[serde(default)]
    not_null: bool,
}

#[derive(Serialize, Deserialize)]
struct StoredTable {
    name: String,
    columns: Vec<StoredColumn>,
    #[serde(default)]
    tid: u64,
    #[serde(default)]
    indexes: Vec<IndexMeta>,
}

#[derive(Serialize, Deserialize)]
struct StoredFunction {
    name: String,
    owner: String,
    source: String,
    comment: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct StoredCatalog {
    format: u32,
    endpoints: Vec<(String, String)>,
    roles: Vec<Role>,
    tables: Vec<StoredTable>,
    table_privs: Vec<(String, String, TablePriv)>,
    domain_privs: Vec<(String, String, bool, bool)>,
    endpoint_grants: Vec<(String, Vec<String>)>,
    functions: Vec<StoredFunction>,
    fn_execute: Vec<(String, Vec<String>)>,
}

fn io(e: impl std::fmt::Display, what: &str) -> DepError {
    DepError::sql("58030", format!("{what}: {e}"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = File::create(&tmp).map_err(|e| io(e, &tmp.display().to_string()))?;
        f.write_all(bytes).map_err(|e| io(e, &tmp.display().to_string()))?;
        f.sync_all().map_err(|e| io(e, &tmp.display().to_string()))?;
    }
    restrict(&tmp);
    fs::rename(&tmp, path).map_err(|e| io(e, &path.display().to_string()))
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn restrict(_: &Path) {}

/// The default data directory: `~/Library/Application Support/Project Hoopeo/data` on macOS,
/// `$XDG_DATA_HOME/project-hoopeo/data` (or `~/.local/share/project-hoopeo/data`) on Linux.
pub fn default_data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join(if cfg!(target_os = "macos") { "Project Hoopeo" } else { "project-hoopeo" }).join("data")
}

fn read_key(path: &Path) -> Result<Keyring> {
    let text = fs::read_to_string(path).map_err(|e| io(e, &path.display().to_string()))?;
    let bytes = hex::decode(text.trim()).map_err(|_| DepError::sql("58030", format!("{} is not a valid key file", path.display())))?;
    let arr: [u8; 32] = bytes.try_into().map_err(|_| DepError::sql("58030", format!("{} must hold 32 bytes", path.display())))?;
    Ok(Keyring::from_bytes(arr))
}

pub fn new_key_path(key_path: &Path) -> PathBuf {
    let mut p = key_path.as_os_str().to_owned();
    p.push(".new");
    PathBuf::from(p)
}

fn meta_get(db: &Database, key: &str) -> Result<Option<Vec<u8>>> {
    let r = db.begin_read().map_err(db_err)?;
    let t = match r.open_table(META) {
        Ok(t) => t,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
        Err(e) => return Err(db_err(e)),
    };
    Ok(t.get(key).map_err(db_err)?.map(|v| v.value().to_vec()))
}

/// Opens a data directory, or creates it, with the default options. Returns the server and whether the directory was new.
pub async fn open(dir: &Path, key_file: Option<&Path>) -> Result<(Arc<DepServer>, bool)> {
    open_with(dir, key_file, &ServerOptions::default()).await
}

/// Opens a data directory, or creates it.
pub async fn open_with(dir: &Path, key_file: Option<&Path>, options: &ServerOptions) -> Result<(Arc<DepServer>, bool)> {
    fs::create_dir_all(dir).map_err(|e| io(e, &dir.display().to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    let lock = File::options().create(true).truncate(false).write(true).open(dir.join(".lock")).map_err(|e| io(e, "lock file"))?;
    if lock.try_lock().is_err() {
        return Err(DepError::sql("55006", format!("the data directory {} is in use by another DEP Engine", dir.display()))
            .with_hint("Connect to that server instead, for example with dep-cli -h 127.0.0.1."));
    }
    let db_path = dir.join("data.redb");
    let legacy = !db_path.exists() && dir.join("catalog.json").exists();
    let fresh = !db_path.exists() && !legacy;
    let key_path = key_file.map(Path::to_path_buf).unwrap_or_else(|| dir.join("master.key"));
    let key_new = new_key_path(&key_path);

    // The key.
    if !key_path.exists() {
        if fresh {
            let mut k = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut k);
            if let Some(parent) = key_path.parent() {
                fs::create_dir_all(parent).map_err(|e| io(e, "key directory"))?;
            }
            write_atomic(&key_path, hex::encode(k).as_bytes())?;
        } else {
            return Err(DepError::sql("58030", format!("the data directory exists but its key file {} is missing", key_path.display()))
                .with_hint("Without the master key the protected values cannot be read. Point --key-file at it."));
        }
    }
    // The builder is not Send: keep it out of the future's state.
    let db = {
        let mut builder = Database::builder();
        builder.set_cache_size(options.cache_bytes());
        builder.create(&db_path).map_err(db_err)?
    };
    restrict(&db_path);
    let mut keys = read_key(&key_path)?;
    match meta_get(&db, "key_fingerprint")? {
        Some(fp) if fp != keys.fingerprint().as_bytes() => {
            // A rotation that committed its data but not yet its key file: finish it.
            let finished = key_new.exists() && read_key(&key_new).map(|k| k.fingerprint().as_bytes() == fp.as_slice()).unwrap_or(false);
            if !finished {
                return Err(DepError::sql("28P01", format!("the master key {} does not match this data directory", key_path.display()))
                    .with_hint("Point --key-file at the key this data was written with."));
            }
            fs::rename(&key_new, &key_path).map_err(|e| io(e, "key file"))?;
            keys = read_key(&key_path)?;
        }
        _ => {
            // A rotation that did not commit: its new key was never used.
            if key_new.exists() {
                let _ = fs::remove_file(&key_new);
            }
        }
    }
    let audit_path = dir.join("audit.redb");
    let audit = AuditStore::open(&audit_path)?;
    restrict(&audit_path);
    let mut opts = options.clone();
    if opts.spill_dir.is_none() {
        opts.spill_dir = Some(dir.join("spill"));
    }
    let server = DepServer::build(keys, db, &opts)?;
    *server.storage.lock().unwrap() = Some(Storage { dir: dir.to_path_buf(), key_path, audit: Some(Arc::new(audit)), _lock: lock });

    let format: u32 = meta_get(server.main_db(), "format")?.and_then(|b| String::from_utf8(b).ok()).and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut notes = Vec::new();
    if legacy {
        notes = migrate_legacy(&server, dir).await?;
        let aside = dir.join("legacy-0.3");
        fs::create_dir_all(&aside).map_err(|e| io(e, "legacy-0.3"))?;
        let _ = fs::rename(dir.join("catalog.json"), aside.join("catalog.json"));
        let _ = fs::rename(dir.join("tables"), aside.join("tables"));
    } else if !fresh && format == 2 {
        notes = migrate_v2(&server).await?;
    } else if !fresh {
        if format > FORMAT {
            return Err(DepError::sql("58030", format!("this data directory was written by a newer DEP Engine (format {format})")));
        }
        load(&server).await?;
    }
    // Every start: the format and key fingerprint, and the catalog when the directory is new.
    let fingerprint = server.keys().fingerprint();
    {
        let mut w = server.writer_now().await?;
        w.put_meta("format", FORMAT.to_string().as_bytes())?;
        w.put_meta("key_fingerprint", fingerprint.as_bytes())?;
        if fresh {
            let cat = server.catalog_now();
            w.put_meta("catalog", &stored_catalog(&cat)?)?;
        }
        crate::store::blocking(move || w.commit())?;
    }
    let last = read_audit(&server, 1)?.first().map(|r| r.seq).unwrap_or(0);
    server.audit_seq.store(last, std::sync::atomic::Ordering::SeqCst);
    *server.startup_notes.lock().unwrap() = notes;
    Ok((server, fresh))
}

/// Kept for callers of earlier versions: every commit is already on disk when a statement returns.
pub async fn flush(_server: &Arc<DepServer>) -> Result<()> {
    Ok(())
}

fn table_meta(t: &StoredTable) -> Result<TableMeta> {
    let columns = t
        .columns
        .iter()
        .map(|c| {
            Ok(ColumnMeta {
                name: c.name.clone(),
                data_type: parse_type(&c.type_name)?,
                type_name: c.type_name.clone(),
                domain: c.domain.clone(),
                primary_key: c.primary_key,
                references: c.references.clone(),
                not_null: c.not_null || c.primary_key,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(TableMeta { name: t.name.clone(), tid: t.tid, columns, indexes: t.indexes.clone(), temporary: false })
}

/// The catalog of a stored catalog, without its functions (they are re-created from their source).
fn catalog_of(stored: &StoredCatalog) -> Result<Catalog> {
    let mut cat = Catalog::new();
    cat.endpoints.clear();
    for (name, mode) in &stored.endpoints {
        cat.endpoints.insert(name.clone(), if mode == "MASKED_READ" { PathMode::MaskedRead } else { PathMode::Operational });
    }
    cat.roles.clear();
    for r in &stored.roles {
        cat.roles.insert(r.name.clone(), r.clone());
    }
    for t in &stored.tables {
        cat.tables.insert(t.name.clone(), table_meta(t)?);
    }
    for (r, t, p) in &stored.table_privs {
        cat.table_privs.insert((r.clone(), t.clone()), p.clone());
    }
    for (r, d, dis, link) in &stored.domain_privs {
        cat.domain_privs.insert((r.clone(), d.clone()), (*dis, *link));
    }
    for (e, roles) in &stored.endpoint_grants {
        cat.endpoint_grants.insert(e.clone(), roles.iter().cloned().collect());
    }
    for (f, roles) in &stored.fn_execute {
        cat.fn_execute.insert(f.clone(), roles.iter().cloned().collect());
    }
    Ok(cat)
}

/// Functions are re-created from their source, in creation order, as their owners.
async fn restore_functions(server: &Arc<DepServer>, stored: &StoredCatalog) -> Result<()> {
    let execute = server.catalog_now().fn_execute.clone();
    for f in &stored.functions {
        let conn = server.restore_connection(&f.owner);
        conn.execute_one(&f.source).await.map_err(|e| DepError::sql("58030", format!("could not re-create function {}: {}", f.name, e.message)))?;
        if let Some(c) = &f.comment {
            server.edit_catalog(|cat| {
                if let Some(m) = cat.functions.get(&f.name).cloned() {
                    let mut copy = crate::server::clone_meta(&m);
                    copy.comment = Some(c.clone());
                    cat.functions.insert(f.name.clone(), Arc::new(copy));
                }
            });
        }
    }
    server.edit_catalog(|cat| cat.fn_execute = execute);
    Ok(())
}

async fn load(server: &Arc<DepServer>) -> Result<()> {
    let bytes = meta_get(server.main_db(), "catalog")?.ok_or_else(|| DepError::sql("58030", "data file: the catalog is missing"))?;
    let stored: StoredCatalog = serde_json::from_slice(&bytes).map_err(|e| io(e, "catalog"))?;
    let cat = catalog_of(&stored)?;
    server.install_catalog(cat);
    restore_functions(server, &stored).await
}

/// A table to import, with its rows (row id and the row as a batch of its physical schema).
struct Import {
    meta: TableMeta,
    batches: Vec<RecordBatch>,
    rowids: Vec<u64>,
    next_rowid: u64,
}

/// Writes imported tables in one commit, builds their indexes, and installs the catalog.
/// A unique index whose rows have duplicate keys is kept, but not enforced; the notes say so.
async fn import(server: &Arc<DepServer>, mut cat: Catalog, imports: Vec<Import>, stored: &StoredCatalog, audit: Vec<AuditRecord>) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    let mut w = server.writer_now().await?;
    for imp in &imports {
        let info = TableInfo::new(&imp.meta);
        let mut at = 0usize;
        for b in &imp.batches {
            for row in encode_rows(b)? {
                w.put_raw(&info, imp.rowids[at], &row)?;
                at += 1;
            }
        }
        w.set_seq(&info, imp.next_rowid.max(imp.rowids.iter().max().map(|m| m + 1).unwrap_or(1)));
        let meta = cat.tables.get_mut(&imp.meta.name).unwrap();
        for ix in info.indexes.clone() {
            let dups = w.build_index(&info, &ix, false)?;
            if dups > 0 {
                if let Some(m) = meta.indexes.iter_mut().find(|i| i.name == ix.meta.name) {
                    m.enforced = false;
                }
                notes.push(format!(
                    "{}: index {} ({}) has {dups} duplicate key(s), so its uniqueness is not enforced. Remove the duplicates, then run REINDEX TABLE {}.",
                    imp.meta.name,
                    ix.meta.name,
                    ix.meta.columns.join(", "),
                    imp.meta.name
                ));
            }
        }
    }
    w.put_meta("catalog", &stored_catalog(&cat)?)?;
    w.put_meta("format", FORMAT.to_string().as_bytes())?;
    crate::store::blocking(move || w.commit())?;
    server.install_catalog(cat);
    if !audit.is_empty() {
        server.audit_store_append(audit).await?;
    }
    restore_functions(server, stored).await?;
    Ok(notes)
}

/// The catalog of an older directory, with table ids and indexes.
fn upgrade_catalog(stored: &StoredCatalog) -> Result<Catalog> {
    let mut cat = catalog_of(stored)?;
    let mut tid = 1u64;
    for t in cat.tables.values_mut() {
        t.tid = tid;
        tid += 1;
        for c in &mut t.columns {
            if c.primary_key {
                c.not_null = true;
            }
        }
        derive_indexes(t);
    }
    // A referenced column needs an index to check references against.
    let refs: Vec<(String, String)> = cat.tables.values().flat_map(|t| t.columns.iter().filter_map(|c| c.references.clone())).collect();
    for (rt, rc) in refs {
        if let Some(t) = cat.tables.get_mut(&rt) {
            crate::catalog::ensure_index(t, &rc);
        }
    }
    Ok(cat)
}

/// Converts a 0.4 data file (format 2: rows as JSON under rows/{table}, no indexes, the audit
/// log inside) to the current format, in one commit.
async fn migrate_v2(server: &Arc<DepServer>) -> Result<Vec<String>> {
    let bytes = meta_get(server.main_db(), "catalog")?.ok_or_else(|| DepError::sql("58030", "data file: the catalog is missing"))?;
    let stored: StoredCatalog = serde_json::from_slice(&bytes).map_err(|e| io(e, "catalog"))?;
    let cat = upgrade_catalog(&stored)?;
    // Read every old table (a 0.4 database fits in memory: 0.4 kept all of it there), then delete it.
    let mut imports = Vec::new();
    let mut audit = Vec::new();
    {
        let db = server.main_db();
        let w = db.begin_write().map_err(db_err)?;
        for meta in cat.tables.values() {
            let name = format!("rows/{}", meta.name);
            let def: TableDefinition<u64, &[u8]> = TableDefinition::new(&name);
            let mut rowids = Vec::new();
            let mut rows = Vec::new();
            match w.open_table(def) {
                Ok(t) => {
                    for item in t.iter().map_err(db_err)? {
                        let (k, v) = item.map_err(db_err)?;
                        rowids.push(k.value());
                        rows.push(v.value().to_vec());
                    }
                }
                Err(e) => return Err(db_err(e)),
            }
            w.delete_table(def).map_err(db_err)?;
            let next = {
                let m = w.open_table(META).map_err(db_err)?;
                let v = m.get(format!("next_rowid/{}", meta.name).as_str()).map_err(db_err)?.and_then(|v| String::from_utf8(v.value().to_vec()).ok()).and_then(|s| s.parse().ok());
                v
            };
            w.open_table(META).map_err(db_err)?.remove(format!("next_rowid/{}", meta.name).as_str()).map_err(db_err)?;
            let batches = decode_json_rows(&meta.physical_schema(), &rows)?;
            imports.push(Import { meta: meta.clone(), batches, next_rowid: next.unwrap_or(rowids.last().copied().unwrap_or(0) + 1), rowids });
        }
        match w.open_table(AUDIT) {
            Ok(t) => {
                for item in t.iter().map_err(db_err)? {
                    let (_, v) = item.map_err(db_err)?;
                    if let Ok(r) = serde_json::from_slice::<AuditRecord>(v.value()) {
                        audit.push(r);
                    }
                }
            }
            Err(e) => return Err(db_err(e)),
        }
        w.delete_table(AUDIT).map_err(db_err)?;
        w.commit().map_err(db_err)?;
    }
    import(server, cat, imports, &stored, audit).await
}

/// Reads a data directory written by DEP Engine 0.3.
async fn migrate_legacy(server: &Arc<DepServer>, dir: &Path) -> Result<Vec<String>> {
    let text = fs::read_to_string(dir.join("catalog.json")).map_err(|e| io(e, "catalog.json"))?;
    let stored: StoredCatalog = serde_json::from_str(&text).map_err(|e| io(e, "catalog.json"))?;
    let cat = upgrade_catalog(&stored)?;
    let mut imports = Vec::new();
    for meta in cat.tables.values() {
        let schema = meta.physical_schema();
        let path = dir.join("tables").join(format!("{}.arrow", meta.name));
        let mut batches = Vec::new();
        if path.exists() {
            let f = File::open(&path).map_err(|e| io(e, &path.display().to_string()))?;
            for b in FileReader::try_new(f, None)? {
                let b = b?;
                batches.push(RecordBatch::try_new(schema.clone(), b.columns().to_vec())?);
            }
        }
        let n: usize = batches.iter().map(|b| b.num_rows()).sum();
        imports.push(Import { meta: meta.clone(), batches, rowids: (1..=n as u64).collect(), next_rowid: n as u64 + 1 });
    }
    import(server, cat, imports, &stored, vec![]).await
}

/// Rows as DEP Engine 0.4 stored them: one JSON object per row.
fn decode_json_rows(schema: &SchemaRef, rows: &[Vec<u8>]) -> Result<Vec<RecordBatch>> {
    use datafusion::arrow::json::ReaderBuilder;
    if rows.is_empty() {
        return Ok(vec![]);
    }
    let mut text = Vec::new();
    for r in rows {
        text.extend_from_slice(r);
        text.push(b'\n');
    }
    let reader = ReaderBuilder::new(schema.clone()).with_batch_size(8192).build(std::io::Cursor::new(text))?;
    let mut out = Vec::new();
    for b in reader {
        out.push(b?);
    }
    Ok(out)
}

/// The catalog as it is stored (temporary tables are not).
pub fn stored_catalog(cat: &Catalog) -> Result<Vec<u8>> {
    let stored = StoredCatalog {
        format: FORMAT,
        endpoints: cat.endpoints.iter().map(|(k, v)| (k.clone(), v.as_str().to_string())).collect(),
        roles: cat.roles.values().cloned().collect(),
        tables: cat
            .tables
            .values()
            .filter(|t| !t.temporary)
            .map(|t| StoredTable {
                name: t.name.clone(),
                tid: t.tid,
                indexes: t.indexes.clone(),
                columns: t
                    .columns
                    .iter()
                    .map(|c| StoredColumn {
                        name: c.name.clone(),
                        type_name: c.type_name.clone(),
                        domain: c.domain.clone(),
                        primary_key: c.primary_key,
                        references: c.references.clone(),
                        not_null: c.not_null,
                    })
                    .collect(),
            })
            .collect(),
        table_privs: cat.table_privs.iter().filter(|((_, t), _)| !cat.tables.get(t).map(|m| m.temporary).unwrap_or(false)).map(|((r, t), p)| (r.clone(), t.clone(), p.clone())).collect(),
        domain_privs: cat.domain_privs.iter().map(|((r, d), (a, b))| (r.clone(), d.clone(), *a, *b)).collect(),
        endpoint_grants: cat.endpoint_grants.iter().map(|(e, s)| (e.clone(), s.iter().cloned().collect())).collect(),
        functions: cat
            .function_order
            .iter()
            .filter_map(|n| cat.functions.get(n))
            .map(|f| StoredFunction { name: f.name.clone(), owner: f.owner.clone(), source: f.source.clone(), comment: f.comment.clone() })
            .collect(),
        fn_execute: cat.fn_execute.iter().map(|(f, s)| (f.clone(), s.iter().cloned().collect())).collect(),
    };
    serde_json::to_vec(&stored).map_err(|e| io(e, "catalog"))
}

// ------------------------------------------------------------------ the audit log

type AuditJob = (Vec<AuditRecord>, tokio::sync::oneshot::Sender<Result<()>>);

/// The audit log file and its writer thread.
pub struct AuditStore {
    db: Arc<Database>,
    tx: std::sync::Mutex<Option<std::sync::mpsc::Sender<AuditJob>>>,
    thread: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Drop for AuditStore {
    /// Stops the writer thread and waits for it, so the file is closed when the server is gone.
    fn drop(&mut self) {
        self.tx.lock().unwrap().take();
        if let Some(t) = self.thread.lock().unwrap().take() {
            let _ = t.join();
        }
    }
}

impl AuditStore {
    fn open(path: &Path) -> Result<AuditStore> {
        let db = Arc::new(Database::create(path).map_err(db_err)?);
        {
            let w = db.begin_write().map_err(db_err)?;
            w.open_table(AUDIT).map_err(db_err)?;
            w.commit().map_err(db_err)?;
        }
        let (tx, rx) = std::sync::mpsc::channel::<AuditJob>();
        let wdb = db.clone();
        let thread = std::thread::Builder::new()
            .name("dep-audit".into())
            .spawn(move || {
                while let Ok(first) = rx.recv() {
                    let mut jobs = vec![first];
                    while jobs.len() < 1024 {
                        match rx.try_recv() {
                            Ok(j) => jobs.push(j),
                            Err(_) => break,
                        }
                    }
                    let r = (|| -> Result<()> {
                        let w = wdb.begin_write().map_err(db_err)?;
                        {
                            let mut t = w.open_table(AUDIT).map_err(db_err)?;
                            for (recs, _) in &jobs {
                                for r in recs {
                                    let bytes = serde_json::to_vec(r).map_err(|e| io(e, "audit"))?;
                                    t.insert(r.seq, bytes.as_slice()).map_err(db_err)?;
                                }
                            }
                        }
                        w.commit().map_err(db_err)
                    })();
                    for (_, done) in jobs {
                        let _ = done.send(r.clone());
                    }
                }
            })
            .map_err(|e| io(e, "audit writer"))?;
        Ok(AuditStore { db, tx: std::sync::Mutex::new(Some(tx)), thread: std::sync::Mutex::new(Some(thread)) })
    }

    /// Appends records; returns when they are on disk.
    pub async fn append(&self, records: Vec<AuditRecord>) -> Result<()> {
        let (done, wait) = tokio::sync::oneshot::channel();
        self.tx.lock().unwrap().as_ref().ok_or_else(|| DepError::sql("58030", "the audit log is closed"))?.send((records, done)).map_err(|_| DepError::sql("58030", "the audit log writer stopped"))?;
        wait.await.map_err(|_| DepError::sql("58030", "the audit log writer stopped"))?
    }

    fn latest(&self, limit: usize) -> Result<Vec<AuditRecord>> {
        let r = self.db.begin_read().map_err(db_err)?;
        let t = r.open_table(AUDIT).map_err(db_err)?;
        let mut out = Vec::new();
        for item in t.iter().map_err(db_err)?.rev().take(limit) {
            let (_, v) = item.map_err(db_err)?;
            if let Ok(rec) = serde_json::from_slice::<AuditRecord>(v.value()) {
                out.push(rec);
            }
        }
        Ok(out)
    }
}

/// The latest `limit` audit records, newest first.
pub fn read_audit(server: &DepServer, limit: usize) -> Result<Vec<AuditRecord>> {
    if let Some(a) = server.storage.lock().unwrap().as_ref().and_then(|s| s.audit.as_ref()) {
        return a.latest(limit);
    }
    Ok(server.audit_mem.lock().unwrap().iter().rev().take(limit).cloned().collect())
}

// ------------------------------------------------------------------ key rotation

/// Writes the new key beside the current one, before the data encrypted with it is committed.
pub fn write_new_key(server: &DepServer, new: &Keyring) -> Result<()> {
    let Some(key_path) = server.storage.lock().unwrap().as_ref().map(|s| s.key_path.clone()) else { return Ok(()) };
    write_atomic(&new_key_path(&key_path), new.master_hex().as_bytes())
}

/// After the commit: the new key file takes the place of the old one. A crash before this is
/// finished by the next start, which finds the new key's fingerprint in the data file.
pub fn finish_new_key(server: &DepServer) -> Result<()> {
    let Some(key_path) = server.storage.lock().unwrap().as_ref().map(|s| s.key_path.clone()) else { return Ok(()) };
    fs::rename(new_key_path(&key_path), &key_path).map_err(|e| io(e, "key file"))
}
