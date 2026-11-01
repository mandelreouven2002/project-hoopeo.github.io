//! The DEP server: the catalog, the keys, the data (see `store`), and connections that each
//! have a fixed access path.
//!
//! **Transactions.** Every statement runs in a transaction: its own (autocommit), or one
//! opened with BEGIN. A transaction reads a snapshot taken when it began. The first write or
//! schema change takes the server's write lock and opens a redb write transaction; COMMIT
//! makes all of it durable at once and publishes the new catalog, ROLLBACK discards it.
//! Schema changes are transactional: a transaction works on its own copy of the catalog.
//! SAVEPOINT, ROLLBACK TO SAVEPOINT and RELEASE SAVEPOINT work as in PostgreSQL.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use datafusion::arrow::array::{Array, RecordBatch, StringArray, UInt64Array};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use datafusion::catalog::TableFunction;
use datafusion::common::{Column, ScalarValue};
use datafusion::dataframe::DataFrame;
use datafusion::execution::disk_manager::DiskManagerBuilder;
use datafusion::execution::memory_pool::FairSpillPool;
use datafusion::execution::runtime_env::RuntimeEnvBuilder;
use datafusion::execution::session_state::SessionStateBuilder;
use datafusion::execution::{SendableRecordBatchStream, SessionState};
use datafusion::logical_expr::{cast, lit, Expr, Filter, LogicalPlan, Projection, ScalarUDF};
use datafusion::physical_plan::stream::RecordBatchStreamAdapter;
use datafusion::prelude::{SessionConfig, SessionContext};
use datafusion::sql::parser::{DFParser, Statement as DFStatement};
use datafusion::sql::sqlparser::ast::{self as ast, AssignmentTarget, ColumnOption, FromTable, ObjectName, Statement as SqlStatement, TableConstraint, TableObject};
use datafusion::sql::sqlparser::dialect::PostgreSqlDialect;
use futures::StreamExt;
use datafusion::execution::FunctionRegistry;
use redb::{Database, ReadTransaction, ReadableDatabase};

use crate::analyze::{Analyzer, Region, Taint};
use crate::audit::{AuditRecord, Event, MatchLimit, Metrics, RateLimiter, RoleLimit};
use crate::catalog::{derive_indexes, ensure_index, parse_type, Catalog, ColumnMeta, DomainPriv, FunctionMeta, IndexMeta, PathMode, Role, TableMeta, TablePriv, ADMIN, CT_SUFFIX, PUBLIC};
use crate::crypto::Keyring;
use crate::error::{DepError, Result};
use crate::functions::DepTableFunction;
use crate::privileges::{self, Uses};
use crate::sqltext;
use crate::sqltext::{GrantStmt, Object, Privilege};
use crate::store::{self, blocking, check_not_null, db_err, encode_rows, DepCatalog, DepCatalogList, DepSchema, ReadView, Source, TableInfo, TxWriter, ROWID};
use crate::udf::{DepFn, Kind, Reveal};

/// The result of one statement, collected.
#[derive(Debug)]
pub enum Output {
    Rows { schema: SchemaRef, batches: Vec<RecordBatch> },
    Command(String),
}

impl Output {
    pub fn row_count(&self) -> usize {
        match self {
            Output::Rows { batches, .. } => batches.iter().map(|b| b.num_rows()).sum(),
            Output::Command(_) => 0,
        }
    }
}

/// The result of one statement, with its rows as a stream (read from disk as it is consumed).
pub enum Streamed {
    Rows { schema: SchemaRef, stream: SendableRecordBatchStream },
    Command(String),
}

impl Streamed {
    fn rows(schema: SchemaRef, batches: Vec<RecordBatch>) -> Streamed {
        let s = futures::stream::iter(batches.into_iter().map(Ok));
        Streamed::Rows { schema: schema.clone(), stream: Box::pin(RecordBatchStreamAdapter::new(schema, s)) }
    }

    /// Collects the rows.
    pub async fn collect(self) -> Result<Output> {
        match self {
            Streamed::Command(t) => Ok(Output::Command(t)),
            Streamed::Rows { schema, mut stream } => {
                let mut batches = Vec::new();
                while let Some(b) = stream.next().await {
                    batches.push(b?);
                }
                Ok(Output::Rows { schema, batches })
            }
        }
    }
}

impl From<Output> for Streamed {
    fn from(o: Output) -> Streamed {
        match o {
            Output::Command(t) => Streamed::Command(t),
            Output::Rows { schema, batches } => Streamed::rows(schema, batches),
        }
    }
}

/// Resource settings of a server.
#[derive(Debug, Clone, Default)]
pub struct ServerOptions {
    /// Memory for query execution (sorts, aggregations, joins). Beyond it, queries spill to
    /// encrypted temporary files. Default: a quarter of the machine's memory.
    pub memory_limit: Option<usize>,
    /// The data file's page cache. Default: an eighth of the machine's memory, at most 4 GiB.
    pub cache_size: Option<usize>,
    /// Where spill files go. Default: DATA_DIR/spill, or the system's temporary directory.
    pub spill_dir: Option<PathBuf>,
    /// Partitions a query runs in parallel. Default: the number of CPUs.
    pub target_partitions: Option<usize>,
}

/// The memory available to the server, in bytes: the machine's memory, or the memory limit of
/// its container (cgroup v2 or v1) when that is lower.
pub fn total_memory() -> Option<u64> {
    let mut total = None;
    #[cfg(unix)]
    unsafe {
        let pages = libc::sysconf(libc::_SC_PHYS_PAGES);
        let size = libc::sysconf(libc::_SC_PAGESIZE);
        if pages > 0 && size > 0 {
            total = Some(pages as u64 * size as u64);
        }
    }
    #[cfg(target_os = "linux")]
    for f in ["/sys/fs/cgroup/memory.max", "/sys/fs/cgroup/memory/memory.limit_in_bytes"] {
        if let Some(limit) = std::fs::read_to_string(f).ok().and_then(|t| t.trim().parse::<u64>().ok()) {
            // cgroup v1 reports "no limit" as a huge number.
            if limit < (1u64 << 60) {
                total = Some(total.map(|t: u64| t.min(limit)).unwrap_or(limit));
            }
            break;
        }
    }
    total
}

/// Parses a size: `512MB`, `4GB`, `2GiB`, `1048576`.
pub fn parse_size(s: &str) -> Option<usize> {
    let t = s.trim().to_uppercase();
    let split = t.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(t.len());
    let (n, unit) = t.split_at(split);
    let n: f64 = n.parse().ok()?;
    let mult: f64 = match unit.trim() {
        "" | "B" => 1.0,
        "K" | "KB" | "KIB" => 1024.0,
        "M" | "MB" | "MIB" => 1024.0 * 1024.0,
        "G" | "GB" | "GIB" => 1024.0 * 1024.0 * 1024.0,
        "T" | "TB" | "TIB" => 1024.0f64.powi(4),
        _ => return None,
    };
    Some((n * mult) as usize)
}

impl ServerOptions {
    pub fn cache_bytes(&self) -> usize {
        self.cache_size.unwrap_or_else(|| total_memory().map(|m| (m / 8).min(4 << 30) as usize).unwrap_or(1 << 30).max(64 << 20))
    }
    pub fn memory_bytes(&self) -> usize {
        self.memory_limit.unwrap_or_else(|| total_memory().map(|m| (m / 4) as usize).unwrap_or(2 << 30).max(256 << 20))
    }
    pub fn partitions(&self) -> usize {
        self.target_partitions.unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2)).max(1)
    }
}

pub struct DepServer {
    /// The DataFusion session every statement's session is made from: settings, functions,
    /// the memory pool and the spill files. Tables and declared functions come from the catalog.
    base: SessionState,
    /// The committed catalog.
    pub(crate) catalog: RwLock<Arc<Catalog>>,
    keys: Arc<Keyring>,
    index_udf: Arc<ScalarUDF>,
    encrypt_udf: Arc<ScalarUDF>,
    decrypt_udf: Arc<ScalarUDF>,
    main: Arc<Database>,
    temp: Arc<Database>,
    /// The data directory, when the server keeps its state on disk.
    pub(crate) storage: Mutex<Option<crate::persist::Storage>>,
    next_tid: AtomicU64,
    /// Counts commits; with `table_seq`, tells a transaction whether a table changed after it began.
    commit_seq: AtomicU64,
    table_seq: Mutex<HashMap<u64, u64>>,
    /// One writer at a time: a transaction holds it from its first write until it ends.
    writer: Arc<tokio::sync::Mutex<()>>,
    /// How long a writer waits for the write lock.
    pub lock_timeout: RwLock<Duration>,
    /// A transaction idle for longer is ended (None: never).
    pub idle_timeout: RwLock<Option<Duration>>,
    /// The latest audit records (all of them for a server in memory; audit.redb keeps them on disk).
    pub(crate) audit_mem: Mutex<std::collections::VecDeque<AuditRecord>>,
    pub(crate) audit_seq: AtomicU64,
    pub(crate) rate: RateLimiter,
    /// The match limit of every role that is not a superuser and has none of its own.
    pub default_match_limit: RwLock<Option<MatchLimit>>,
    pub metrics: Arc<Metrics>,
    /// Messages for the log when the server started (for example, about a converted data directory).
    pub startup_notes: Mutex<Vec<String>>,
    slots: Mutex<Vec<Weak<TxnSlot>>>,
    reaper: AtomicBool,
    pub spill: Option<Arc<crate::spill::EncryptedSpill>>,
    pub memory_limit: usize,
    /// DataFusion sessions made for recent catalogs (tables and declared functions registered),
    /// cloned for each statement instead of being made again.
    sessions: Mutex<Vec<(Arc<Catalog>, SessionState)>>,
}

/// An open transaction of one connection.
struct TxnState {
    /// Opened with BEGIN (otherwise the transaction of one statement).
    explicit: bool,
    begin_seq: u64,
    main: Arc<ReadTransaction>,
    temp: Arc<ReadTransaction>,
    writer: Option<Arc<Mutex<TxWriter>>>,
    /// The transaction's own catalog, after its first schema change.
    catalog: Option<Arc<Catalog>>,
    savepoints: Vec<Savepoint>,
    /// A statement failed: the rest of the transaction is ignored until ROLLBACK (or ROLLBACK TO).
    failed: bool,
}

struct Savepoint {
    name: String,
    undo: usize,
    catalog: Option<Arc<Catalog>>,
}

/// A connection's transaction, as the idle-transaction reaper sees it.
pub(crate) struct TxnSlot {
    txn: Mutex<Option<TxnState>>,
    busy: AtomicBool,
    killed: AtomicBool,
    last: Mutex<Instant>,
}

pub struct Connection {
    server: Arc<DepServer>,
    pub endpoint: String,
    pub path: PathMode,
    /// The role that logged in.
    pub session_user: String,
    /// The role whose privileges apply now (changed by SET ROLE).
    role: Mutex<String>,
    /// Set only while the server re-creates its functions at startup.
    restore: bool,
    /// Notes about how the DEP rules applied to the last statement (SHOW DEP NOTES).
    notes: Mutex<Vec<String>>,
    slot: Arc<TxnSlot>,
    /// Uses of protected data by the statement running now, for the audit log.
    pending_audit: Mutex<Vec<Event>>,
    /// Statements described for the extended protocol, by text, for the catalog they were described against.
    described: Mutex<HashMap<String, (usize, Described)>>,
}

type Described = (Vec<Option<DataType>>, Option<SchemaRef>);

impl Drop for Connection {
    fn drop(&mut self) {
        // An open transaction is rolled back (dropping its writer releases the write lock).
        if let Some(t) = self.slot.txn.lock().unwrap().take() {
            if let Some(w) = t.writer {
                w.lock().unwrap().abort();
            }
        }
    }
}

fn md5_hex(s: &str) -> String {
    format!("{:x}", md5::compute(s.as_bytes()))
}

fn denied(msg: impl Into<String>) -> DepError {
    DepError::sql("42501", msg)
}

fn internal(msg: &str) -> DepError {
    DepError::sql("XX000", format!("internal: {msg}"))
}

fn ident_name(i: &ast::Ident) -> String {
    if i.quote_style.is_some() {
        i.value.clone()
    } else {
        i.value.to_lowercase()
    }
}

fn object_name(n: &ObjectName) -> String {
    let s = n.to_string();
    let last = s.rsplit('.').next().unwrap_or(&s);
    if last.starts_with('"') {
        last.trim_matches('"').to_string()
    } else {
        last.to_lowercase()
    }
}

fn index_columns(cols: &[ast::IndexColumn]) -> Result<Vec<String>> {
    cols.iter()
        .map(|c| match &c.column.expr {
            ast::Expr::Identifier(i) => Ok(ident_name(i)),
            ast::Expr::CompoundIdentifier(v) => Ok(v.last().map(ident_name).unwrap_or_default()),
            e => Err(DepError::unsupported(format!("Index on an expression ({e}) is not supported: index columns."))),
        })
        .collect()
}

fn in_memory_db() -> Result<Database> {
    Database::builder().create_with_backend(redb::backends::InMemoryBackend::new()).map_err(db_err)
}

impl DepServer {
    /// A server that keeps everything in memory.
    pub fn new() -> Arc<Self> {
        Self::in_memory(&ServerOptions { memory_limit: Some(1 << 30), ..Default::default() }).expect("server")
    }

    /// A server that keeps everything in memory, with resource settings.
    pub fn in_memory(opts: &ServerOptions) -> Result<Arc<Self>> {
        let db = in_memory_db()?;
        let s = Self::build(Keyring::random(), db, opts)?;
        // The catalog of a new database, so the first transaction starts from it.
        let w = s.main.begin_write().map_err(db_err)?;
        w.open_table(crate::persist::META).map_err(db_err)?.insert("catalog", crate::persist::stored_catalog(&s.catalog_now())?.as_slice()).map_err(db_err)?;
        w.commit().map_err(db_err)?;
        Ok(s)
    }

    /// A server on a database (a file, or in memory).
    pub(crate) fn build(keys: Keyring, main: Database, opts: &ServerOptions) -> Result<Arc<Self>> {
        let keys = Arc::new(keys);
        let memory_limit = opts.memory_bytes();
        let spill_dir = opts.spill_dir.clone().unwrap_or_else(|| std::env::temp_dir().join(format!("hoopeo-spill-{}", std::process::id())));
        let spill = crate::spill::EncryptedSpill::new(&spill_dir).ok().map(Arc::new);
        let mut rt = RuntimeEnvBuilder::new().with_memory_pool(Arc::new(FairSpillPool::new(memory_limit)));
        rt = match &spill {
            Some(s) => rt.with_disk_manager_builder(DiskManagerBuilder::default().with_temp_file_factory(s.clone())),
            None => rt.with_disk_manager_builder(DiskManagerBuilder::default().with_mode(datafusion::execution::disk_manager::DiskManagerMode::Disabled)),
        };
        let runtime = rt.build_arc()?;
        let mut config = SessionConfig::new().with_information_schema(false).with_target_partitions(opts.partitions());
        // Each sort sets memory aside for merging what it spilled. Scale it to the memory a query
        // may use, so that concurrent sorts on a server with a modest limit can still spill.
        let per_sort = memory_limit / (8 * opts.partitions().max(1));
        let ex = &mut config.options_mut().execution;
        ex.sort_spill_reservation_bytes = ex.sort_spill_reservation_bytes.min(per_sort.max(256 << 10));
        let mut base = SessionStateBuilder::new().with_config(config).with_runtime_env(runtime).with_default_features().build();
        base.register_udf(Arc::new(Reveal::udf()))?;
        Ok(Arc::new(DepServer {
            base,
            catalog: RwLock::new(Arc::new(Catalog::new())),
            index_udf: DepFn::udf(Kind::Index, keys.clone()),
            encrypt_udf: DepFn::udf(Kind::Encrypt, keys.clone()),
            decrypt_udf: DepFn::udf(Kind::Decrypt, keys.clone()),
            keys,
            main: Arc::new(main),
            temp: Arc::new(in_memory_db()?),
            storage: Mutex::new(None),
            next_tid: AtomicU64::new(1),
            commit_seq: AtomicU64::new(0),
            table_seq: Mutex::new(HashMap::new()),
            writer: Arc::new(tokio::sync::Mutex::new(())),
            lock_timeout: RwLock::new(Duration::from_secs(30)),
            idle_timeout: RwLock::new(None),
            audit_mem: Mutex::new(Default::default()),
            audit_seq: AtomicU64::new(0),
            rate: RateLimiter::default(),
            default_match_limit: RwLock::new(None),
            metrics: Arc::new(Metrics::default()),
            startup_notes: Mutex::new(vec![]),
            slots: Mutex::new(vec![]),
            reaper: AtomicBool::new(false),
            spill,
            memory_limit,
            sessions: Mutex::new(vec![]),
        }))
    }

    /// A DataFusion session for a catalog: its tables, and its declared functions as table functions.
    fn session_for(&self, catalog: &Arc<Catalog>, with_rowid: Option<&str>) -> SessionState {
        if with_rowid.is_none() {
            if let Some((_, st)) = self.sessions.lock().unwrap().iter().find(|(c, _)| Arc::ptr_eq(c, catalog)) {
                return st.clone();
            }
        }
        let schema = Arc::new(DepSchema { catalog: catalog.clone(), with_rowid: with_rowid.map(str::to_string) });
        let list = Arc::new(DepCatalogList(Arc::new(DepCatalog(schema))));
        let mut fns = self.base.table_functions().clone();
        for f in catalog.functions.values() {
            fns.insert(f.name.clone(), Arc::new(TableFunction::new(f.name.clone(), Arc::new(DepTableFunction { meta: f.clone() }))));
        }
        let st = SessionStateBuilder::new_from_existing(self.base.clone()).with_catalog_list(list).with_table_functions(fns).build();
        if with_rowid.is_none() {
            let mut cache = self.sessions.lock().unwrap();
            if cache.len() >= 8 {
                cache.remove(0);
            }
            cache.push((catalog.clone(), st.clone()));
        }
        st
    }

    pub(crate) fn main_db(&self) -> &Database {
        &self.main
    }

    /// The committed catalog.
    pub fn catalog_now(&self) -> Arc<Catalog> {
        self.catalog.read().unwrap().clone()
    }

    /// Replaces the committed catalog (at startup, and when a transaction commits).
    pub(crate) fn install_catalog(&self, cat: Catalog) {
        self.install_catalog_arc(Arc::new(cat));
    }

    fn install_catalog_arc(&self, cat: Arc<Catalog>) {
        let max = cat.tables.values().map(|t| t.tid).max().unwrap_or(0);
        self.next_tid.fetch_max(max + 1, Ordering::SeqCst);
        *self.catalog.write().unwrap() = cat;
    }

    /// Changes the committed catalog in memory (startup only).
    pub(crate) fn edit_catalog(&self, f: impl FnOnce(&mut Catalog)) {
        let mut c = (*self.catalog_now()).clone();
        f(&mut c);
        self.install_catalog(c);
    }

    /// Takes the write lock and opens a writer, for work outside any connection (startup, imports).
    pub(crate) async fn writer_now(&self) -> Result<TxWriter> {
        let guard = self.lock_writer().await?;
        Ok(TxWriter::new(guard, self.main.clone(), self.temp.clone()))
    }

    /// Writes records to the audit log of the data directory (imports).
    pub(crate) async fn audit_store_append(&self, records: Vec<AuditRecord>) -> Result<()> {
        let store = self.storage.lock().unwrap().as_ref().and_then(|s| s.audit.clone());
        match store {
            Some(a) => a.append(records).await,
            None => Ok(()),
        }
    }

    /// Waits for the write lock, up to the lock timeout.
    pub(crate) async fn lock_writer(&self) -> Result<tokio::sync::OwnedMutexGuard<()>> {
        let timeout = *self.lock_timeout.read().unwrap();
        match tokio::time::timeout(timeout, self.writer.clone().lock_owned()).await {
            Ok(g) => Ok(g),
            Err(_) => Err(DepError::sql("55P03", format!("could not obtain the write lock within {} s: another transaction is writing", timeout.as_secs()))
                .with_hint("Commit or roll back the other transaction, or retry.")),
        }
    }

    fn snapshot(&self) -> Result<(u64, Arc<ReadTransaction>, Arc<ReadTransaction>)> {
        let seq = self.commit_seq.load(Ordering::SeqCst);
        let main = Arc::new(self.main.begin_read().map_err(db_err)?);
        let temp = Arc::new(self.temp.begin_read().map_err(db_err)?);
        Ok((seq, main, temp))
    }

    /// Adds records to the audit log; returns when they are on disk.
    pub(crate) async fn audit(&self, mut records: Vec<AuditRecord>) {
        if records.is_empty() {
            return;
        }
        for r in &mut records {
            r.seq = self.audit_seq.fetch_add(1, Ordering::SeqCst) + 1;
        }
        {
            let mut mem = self.audit_mem.lock().unwrap();
            for r in &records {
                mem.push_back(r.clone());
            }
            while mem.len() > 10_000 {
                mem.pop_front();
            }
        }
        if let Err(e) = self.audit_store_append(records).await {
            eprintln!("DEP Engine: could not write the audit log: {}", e.message);
        }
    }

    /// The match limit that applies to a role now.
    fn match_limit(&self, cat: &Catalog, role: &str) -> Option<MatchLimit> {
        match cat.roles.get(role).map(|r| r.match_limit).unwrap_or_default() {
            RoleLimit::Limit(l) => Some(l),
            RoleLimit::Unlimited => None,
            RoleLimit::Default => {
                if cat.is_superuser(role) {
                    None
                } else {
                    *self.default_match_limit.read().unwrap()
                }
            }
        }
    }

    fn new_slot(&self) -> Arc<TxnSlot> {
        let s = Arc::new(TxnSlot { txn: Mutex::new(None), busy: AtomicBool::new(false), killed: AtomicBool::new(false), last: Mutex::new(Instant::now()) });
        let mut slots = self.slots.lock().unwrap();
        slots.retain(|w| w.strong_count() > 0);
        slots.push(Arc::downgrade(&s));
        s
    }

    /// Ends transactions left idle for longer than the idle timeout, so a client that went
    /// away in the middle of one does not hold the write lock. Started by the network front ends.
    pub fn start_reaper(self: &Arc<Self>) {
        if self.reaper.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let Some(s) = weak.upgrade() else { return };
                let Some(limit) = *s.idle_timeout.read().unwrap() else { continue };
                let slots: Vec<Arc<TxnSlot>> = s.slots.lock().unwrap().iter().filter_map(|w| w.upgrade()).collect();
                for slot in slots {
                    if slot.busy.load(Ordering::SeqCst) || slot.last.lock().unwrap().elapsed() < limit {
                        continue;
                    }
                    let Ok(mut g) = slot.txn.try_lock() else { continue };
                    if g.as_ref().map(|t| t.explicit).unwrap_or(false) {
                        if let Some(t) = g.take() {
                            if let Some(w) = t.writer {
                                w.lock().unwrap().abort();
                            }
                        }
                        slot.killed.store(true, Ordering::SeqCst);
                        Metrics::inc(&s.metrics.idle_terminated, 1);
                    }
                }
            }
        });
    }

    /// Opens a connection as the superuser `admin`, through an endpoint. The endpoint fixes the access path.
    pub fn connect(self: &Arc<Self>, endpoint: &str) -> Result<Connection> {
        self.connect_as(ADMIN, endpoint)
    }

    /// Opens a connection as a role, through an endpoint. The role must be allowed to log in and to
    /// connect through the endpoint. The password, if any, is checked by the caller (see [`Self::check_password`]).
    pub fn connect_as(self: &Arc<Self>, user: &str, endpoint: &str) -> Result<Connection> {
        let ep = endpoint.to_lowercase();
        let user = user.to_lowercase();
        let cat = self.catalog_now();
        let path = *cat.endpoints.get(&ep).ok_or_else(|| {
            DepError::sql("08001", format!("endpoint \"{ep}\" does not exist"))
                .with_hint("The endpoints are operational (OPERATIONAL) and analytics (MASKED_READ), plus any made with CREATE ENDPOINT.")
        })?;
        let role = cat.roles.get(&user).ok_or_else(|| DepError::sql("28000", format!("role \"{user}\" does not exist")))?;
        if !role.login {
            return Err(DepError::sql("28000", format!("role \"{user}\" is not permitted to log in")));
        }
        if !cat.can_connect(&user, &ep) {
            return Err(DepError::sql("28000", format!("role \"{user}\" is not allowed to connect through endpoint \"{ep}\""))
                .with_hint(format!("A superuser can allow it: GRANT CONNECT ON ENDPOINT {ep} TO {user}.")));
        }
        Ok(Connection {
            server: self.clone(),
            endpoint: ep,
            path,
            session_user: user.clone(),
            role: Mutex::new(user),
            restore: false,
            notes: Default::default(),
            slot: self.new_slot(),
            pending_audit: Default::default(),
            described: Default::default(),
        })
    }

    pub fn is_superuser(&self, role: &str) -> bool {
        self.catalog_now().is_superuser(role)
    }

    /// A connection used while restoring state: it acts as `role` and skips privilege checks
    /// that already passed when the objects were first created.
    pub(crate) fn restore_connection(self: &Arc<Self>, role: &str) -> Connection {
        Connection {
            server: self.clone(),
            endpoint: "operational".into(),
            path: PathMode::Operational,
            session_user: role.into(),
            role: Mutex::new(role.into()),
            restore: true,
            notes: Default::default(),
            slot: self.new_slot(),
            pending_audit: Default::default(),
            described: Default::default(),
        }
    }

    /// Whether this server keeps its state on disk.
    pub fn is_persistent(&self) -> bool {
        self.storage.lock().unwrap().is_some()
    }

    /// Sets a role's password, and writes the catalog at once. Fails if a transaction is writing.
    pub fn set_password(&self, role: &str, password: &str) -> Result<()> {
        let guard = self.writer.clone().try_lock_owned().map_err(|_| DepError::sql("55P03", "another transaction is writing; try again"))?;
        let mut c = (*self.catalog_now()).clone();
        let r = c.roles.get_mut(role).ok_or_else(|| DepError::sql("42704", format!("role \"{role}\" does not exist")))?;
        r.password_md5 = Some(md5_hex(&format!("{password}{role}")));
        let mut w = TxWriter::new(guard, self.main.clone(), self.temp.clone());
        w.put_meta("catalog", &crate::persist::stored_catalog(&c)?)?;
        blocking(|| w.commit())?;
        self.install_catalog(c);
        w.release();
        Ok(())
    }

    /// The stored password of a role, `hex(md5(password || role))`, as PostgreSQL keeps it.
    pub fn password_md5(&self, role: &str) -> Option<String> {
        self.catalog_now().roles.get(role).and_then(|r| r.password_md5.clone())
    }

    pub fn check_password(&self, role: &str, password: &str) -> bool {
        self.password_md5(role).map(|h| h == md5_hex(&format!("{password}{role}"))).unwrap_or(false)
    }

    pub fn describe_roles(&self) -> Vec<(Role, Vec<String>)> {
        describe_roles(&self.catalog_now())
    }

    pub fn describe_privileges(&self) -> Vec<String> {
        describe_privileges(&self.catalog_now())
    }

    pub fn endpoints(&self) -> Vec<(String, PathMode)> {
        self.catalog_now().endpoints.iter().map(|(k, v)| (k.clone(), *v)).collect()
    }

    pub fn describe_tables(&self) -> Vec<TableMeta> {
        self.catalog_now().tables.values().cloned().collect()
    }

    pub fn describe_functions(&self) -> Vec<Arc<FunctionMeta>> {
        self.catalog_now().functions.values().cloned().collect()
    }

    /// The rows of every table, from the latest commit.
    pub fn row_counts(&self) -> Result<Vec<(String, u64)>> {
        let (_, main, temp) = self.snapshot()?;
        let cat = self.catalog_now();
        let mut out = Vec::new();
        for t in cat.tables.values() {
            let info = TableInfo::new(t);
            let src = Source::Snap(if t.temporary { temp.clone() } else { main.clone() });
            out.push((t.name.clone(), src.count(&info)?));
        }
        Ok(out)
    }

    /// The table as the server stores it: index entries and ciphertexts in place of protected values.
    pub async fn storage(self: &Arc<Self>, table: &str) -> Result<Output> {
        let conn = self.restore_connection(ADMIN);
        conn.stored_rows(table, None).await
    }

    pub fn keys(&self) -> &Keyring {
        &self.keys
    }
}

fn describe_roles(cat: &Catalog) -> Vec<(Role, Vec<String>)> {
    cat.roles.values().map(|r| (r.clone(), cat.endpoints.keys().filter(|e| cat.can_connect(&r.name, e)).cloned().collect())).collect()
}

/// Human-readable privileges: one line per grant.
fn describe_privileges(cat: &Catalog) -> Vec<String> {
    let mut v = Vec::new();
    for ((role, table), p) in &cat.table_privs {
        if !p.is_empty() {
            v.push(format!("{role}: {} ON {table}", p.describe()));
        }
    }
    for ((role, d), (dis, link)) in &cat.domain_privs {
        let k: Vec<&str> = [(*dis, "DISCLOSE"), (*link, "LINK")].iter().filter(|x| x.0).map(|x| x.1).collect();
        if !k.is_empty() {
            v.push(format!("{role}: {} ON MATCH DOMAIN {d}", k.join(", ")));
        }
    }
    for (f, roles) in &cat.fn_execute {
        for r in roles {
            v.push(format!("{r}: EXECUTE ON FUNCTION {f}"));
        }
    }
    for (e, roles) in &cat.endpoint_grants {
        for r in roles {
            v.push(format!("{r}: CONNECT ON ENDPOINT {e}"));
        }
    }
    v
}

/// A single-column result of text values.
fn text_rows(cols: &[&str], rows: Vec<Vec<Option<String>>>) -> Result<Output> {
    let schema = Arc::new(Schema::new(cols.iter().map(|c| Field::new(*c, DataType::Utf8, true)).collect::<Vec<_>>()));
    let arrays: Vec<datafusion::arrow::array::ArrayRef> =
        (0..cols.len()).map(|i| Arc::new(StringArray::from(rows.iter().map(|r| r[i].clone()).collect::<Vec<_>>())) as datafusion::arrow::array::ArrayRef).collect();
    let b = RecordBatch::try_new(schema.clone(), arrays)?;
    Ok(Output::Rows { schema, batches: vec![b] })
}

/// Whether a plan reads a table.
fn plan_reads(plan: &LogicalPlan, table: &str) -> bool {
    use datafusion::common::tree_node::TreeNodeRecursion;
    let mut found = false;
    let _ = plan.apply_with_subqueries(|p| {
        if let LogicalPlan::TableScan(ts) = p {
            if ts.table_name.table() == table {
                found = true;
                return Ok(TreeNodeRecursion::Stop);
            }
        }
        Ok(TreeNodeRecursion::Continue)
    });
    found
}

impl Connection {
    /// The server this connection belongs to.
    pub fn server(&self) -> &Arc<DepServer> {
        &self.server
    }

    /// The role whose privileges apply now.
    pub fn role(&self) -> String {
        self.role.lock().unwrap().clone()
    }

    /// The DEP notes of the last statement: how the rules applied (lookups, anchored joins,
    /// disclosing or linking functions, protected outputs). Empty for plain SQL.
    pub fn notes(&self) -> Vec<String> {
        self.notes.lock().unwrap().clone()
    }

    /// Restores notes kept by a front end between statements.
    pub fn set_notes(&self, notes: Vec<String>) {
        *self.notes.lock().unwrap() = notes;
    }

    fn add_notes(&self, more: Vec<String>) {
        let mut n = self.notes.lock().unwrap();
        for m in more {
            if !n.contains(&m) {
                n.push(m);
            }
        }
    }

    /// The catalog this connection sees now: its transaction's own, or the committed one.
    fn cat(&self) -> Arc<Catalog> {
        if let Some(c) = self.slot.txn.lock().unwrap().as_ref().and_then(|t| t.catalog.clone()) {
            return c;
        }
        self.server.catalog_now()
    }

    /// Changes the catalog, inside the transaction (which must hold the writer). On error nothing changes.
    fn cat_mut<R>(&self, f: impl FnOnce(&mut Catalog) -> Result<R>) -> Result<R> {
        let mut c = (*self.cat()).clone();
        let r = f(&mut c)?;
        if self.restore {
            self.server.install_catalog(c);
            return Ok(r);
        }
        let mut g = self.slot.txn.lock().unwrap();
        let t = g.as_mut().ok_or_else(|| internal("a schema change outside a transaction"))?;
        if t.writer.is_none() {
            return Err(internal("a schema change without the write lock"));
        }
        t.catalog = Some(Arc::new(c));
        Ok(r)
    }

    fn require_superuser(&self, what: &str) -> Result<()> {
        if !self.cat().is_superuser(&self.role()) {
            return Err(denied(format!("permission denied to {what}")).with_hint("Only a superuser can do this."));
        }
        Ok(())
    }

    /// Checks the privileges a statement needs to read what it reads.
    fn check_uses(&self, cat: &Catalog, uses: &Uses) -> Result<()> {
        let role = self.role();
        for (t, c) in &uses.columns {
            if !cat.can_select(&role, t, c) {
                let mut e = denied(format!("permission denied for column {c} of table {t}"));
                e.hint = Some(if cat.protected_column(t, c).is_some() {
                    format!("In DEP, the privilege on a PROTECTED column is the privilege to match on it. GRANT SELECT ({c}) ON {t} TO {role} would allow it.")
                } else {
                    format!("GRANT SELECT ({c}) ON {t} TO {role} would allow it.")
                });
                return Err(e);
            }
        }
        for t in &uses.tables {
            if !uses.columns.iter().any(|(tt, _)| tt == t) && !cat.can_select_any(&role, t) {
                return Err(denied(format!("permission denied for table {t}")));
            }
        }
        for f in &uses.functions {
            let Some(m) = cat.functions.get(f) else { continue };
            if !cat.can_execute(&role, m) {
                return Err(denied(format!("permission denied for function {f}"))
                    .with_hint(format!("The owner, {}, can GRANT EXECUTE ON FUNCTION {f} TO {role}.", m.owner)));
            }
            Self::check_owner(cat, m)?;
        }
        Ok(())
    }

    /// A function body runs with its owner's privileges, checked again at each call.
    fn check_owner(cat: &Catalog, m: &FunctionMeta) -> Result<()> {
        for (t, c) in &m.reads {
            if !cat.tables.contains_key(t) || !cat.can_select(&m.owner, t, c) {
                return Err(denied(format!("permission denied for function {}: its owner {} can no longer read {t}.{c}", m.name, m.owner)));
            }
        }
        for t in &m.scans {
            if !cat.tables.contains_key(t) || !cat.can_select_any(&m.owner, t) {
                return Err(denied(format!("permission denied for function {}: its owner {} can no longer read {t}", m.name, m.owner)));
            }
        }
        for g in &m.calls {
            match cat.functions.get(g) {
                Some(gm) if cat.can_execute(&m.owner, gm) => Self::check_owner(cat, gm)?,
                _ => return Err(denied(format!("permission denied for function {}: its owner {} can no longer execute {g}", m.name, m.owner))),
            }
        }
        Ok(())
    }

    /// Runs a script. Like psql, a failed statement does not stop the ones after it.
    pub async fn execute(&self, sql: &str) -> Vec<Result<Output>> {
        let stmts = match sqltext::split(sql) {
            Ok(s) => s,
            Err(e) => return vec![Err(e)],
        };
        let mut out = Vec::new();
        for s in stmts {
            out.push(self.execute_one(&s).await);
        }
        out
    }

    /// Runs a script and stops at the first error.
    pub async fn execute_strict(&self, sql: &str) -> Result<Vec<Output>> {
        let mut out = Vec::new();
        for s in sqltext::split(sql)? {
            out.push(self.execute_one(&s).await?);
        }
        Ok(out)
    }

    fn analyzer<'c>(&self, catalog: &'c Catalog, region: Option<Region>) -> Analyzer<'c> {
        let s = &self.server;
        Analyzer::new(self.path, region, catalog, s.index_udf.clone(), s.decrypt_udf.clone())
    }

    fn require_operational(&self, what: &str) -> Result<()> {
        if self.path == PathMode::MaskedRead {
            return Err(DepError::masked(what));
        }
        Ok(())
    }

    /// Runs one statement and collects its rows.
    pub async fn execute_one(&self, sql: &str) -> Result<Output> {
        let s = self.execute_stream(sql).await?;
        match s.collect().await {
            Ok(o) => Ok(o),
            Err(e) => {
                self.stream_failed(&e);
                Err(e)
            }
        }
    }

    /// A statement's row stream failed after it started: its transaction fails too.
    pub fn stream_failed(&self, e: &DepError) {
        self.server.metrics.error(e.code.unwrap_or(e.sqlstate.as_str()));
        self.fail_txn();
    }

    /// Runs one statement. A query's rows come as a stream, read from disk as it is consumed.
    pub async fn execute_stream(&self, sql: &str) -> Result<Streamed> {
        self.slot.busy.store(true, Ordering::SeqCst);
        let out = self.execute_stream_inner(sql).await;
        *self.slot.last.lock().unwrap() = Instant::now();
        self.slot.busy.store(false, Ordering::SeqCst);
        out
    }

    async fn execute_stream_inner(&self, sql: &str) -> Result<Streamed> {
        let w = sqltext::leading_words(sql, 3);
        let first = w.first().cloned().unwrap_or_default();
        if w.len() < 3 || w[0] != "SHOW" || w[1] != "DEP" || w[2] != "NOTES" {
            self.notes.lock().unwrap().clear();
        }
        self.pending_audit.lock().unwrap().clear();
        Metrics::inc(&self.server.metrics.statements, 1);
        if self.slot.killed.swap(false, Ordering::SeqCst) {
            return Err(DepError::sql("25P03", "the transaction was ended because it was idle for longer than the idle-in-transaction timeout")
                .with_hint("Its changes were rolled back. Run the transaction again."));
        }
        let control = matches!(first.as_str(), "BEGIN" | "START" | "COMMIT" | "END" | "ROLLBACK" | "ABORT" | "SAVEPOINT" | "RELEASE");
        let rollback_to = first == "ROLLBACK" && w.get(1).map(|x| x == "TO").unwrap_or(false);
        if self.txn_failed() && !matches!(first.as_str(), "COMMIT" | "END" | "ROLLBACK" | "ABORT") {
            return Err(DepError::sql("25P02", "current transaction is aborted, commands ignored until end of transaction block")
                .with_hint("ROLLBACK (or ROLLBACK TO a savepoint), then run the statements again."));
        }
        let implicit = !control && !self.in_txn();
        if implicit {
            self.begin_txn(false)?;
        }
        let mut out = self.execute_inner(sql).await;
        if implicit {
            match &out {
                Ok(_) => {
                    if let Err(e) = self.commit_txn().await {
                        out = Err(e);
                    }
                }
                Err(_) => self.rollback_txn(),
            }
        }
        let err = out.as_ref().err().cloned();
        self.flush_audit(err.as_ref(), &first).await;
        if let Err(e) = &out {
            self.server.metrics.error(e.code.unwrap_or(e.sqlstate.as_str()));
            // In a transaction, a failed statement fails the transaction (except COMMIT and ROLLBACK themselves).
            let ends = matches!(first.as_str(), "COMMIT" | "END" | "ABORT") || (first == "ROLLBACK" && !rollback_to);
            if !ends {
                self.fail_txn();
            }
        }
        out
    }

    /// For the extended query protocol: the parameter types and the result columns of a
    /// statement, worked out without running it (no rows are read, nothing is audited).
    pub async fn describe(&self, sql: &str) -> Result<(Vec<Option<DataType>>, Option<SchemaRef>)> {
        // A prepared statement is described once per catalog (a driver asks at every execution).
        let key = Arc::as_ptr(&self.cat()) as usize;
        if let Some((k, d)) = self.described.lock().unwrap().get(sql) {
            if *k == key {
                return Ok(d.clone());
            }
        }
        let d = self.describe_uncached(sql).await?;
        let mut cache = self.described.lock().unwrap();
        if cache.len() > 512 {
            cache.clear();
        }
        cache.insert(sql.to_string(), (key, d.clone()));
        Ok(d)
    }

    async fn describe_uncached(&self, sql: &str) -> Result<(Vec<Option<DataType>>, Option<SchemaRef>)> {
        let n = sqltext::placeholder_count(sql)?;
        let w = sqltext::leading_words(sql, 2);
        let first = w.first().map(|s| s.as_str()).unwrap_or("");
        match first {
            "SELECT" | "WITH" | "VALUES" | "TABLE" | "(" => {
                sqltext::check_reserved(sql)?;
                let mut stmts = DFParser::parse_sql_with_dialect(sql, &PostgreSqlDialect {})?;
                let stmt = stmts.pop_front().ok_or_else(|| DepError::sql("42601", "empty statement"))?;
                let plan = self.state()?.statement_to_plan(stmt).await?;
                let types = plan.get_parameter_types().unwrap_or_default();
                let params = (1..=n).map(|i| types.get(&format!("${i}")).cloned().flatten()).collect();
                let finalized = {
                    let cat = self.cat();
                    let mut an = self.analyzer(&cat, None);
                    self.query_plan_inner(&cat, &mut an, &plan)
                };
                self.notes.lock().unwrap().clear();
                let finalized = finalized?;
                Ok((params, Some(Arc::new(finalized.schema().as_arrow().clone()))))
            }
            // Writes: DataFusion infers each parameter's type from the column it is written to
            // or compared with (a PROTECTED column takes TEXT), so drivers can send typed values.
            "INSERT" | "UPDATE" | "DELETE" => {
                let inferred = async {
                    sqltext::check_reserved(sql).ok()?;
                    let mut stmts = DFParser::parse_sql_with_dialect(sql, &PostgreSqlDialect {}).ok()?;
                    let stmt = stmts.pop_front()?;
                    let plan = self.state().ok()?.statement_to_plan(stmt).await.ok()?;
                    plan.get_parameter_types().ok()
                }
                .await
                .unwrap_or_default();
                Ok(((1..=n).map(|i| inferred.get(&format!("${i}")).cloned().flatten()).collect(), None))
            }
            "EXPLAIN" => Ok((vec![None; n], Some(Arc::new(Schema::new(vec![Field::new("dep_plan", DataType::Utf8, false)]))))),
            "SHOW" if n == 0 => match self.execute_one(sql).await? {
                Output::Rows { schema, .. } => Ok((vec![], Some(schema))),
                Output::Command(_) => Ok((vec![], None)),
            },
            _ => Ok((vec![None; n], None)),
        }
    }

    // ------------------------------------------------------------------ transactions

    /// Whether a transaction is open (BEGIN without COMMIT or ROLLBACK yet).
    pub fn in_txn(&self) -> bool {
        self.slot.txn.lock().unwrap().as_ref().map(|t| t.explicit).unwrap_or(false)
    }

    /// Whether the open transaction failed, and waits for ROLLBACK.
    pub fn txn_failed(&self) -> bool {
        self.slot.txn.lock().unwrap().as_ref().map(|t| t.failed).unwrap_or(false)
    }

    fn fail_txn(&self) {
        if let Some(t) = self.slot.txn.lock().unwrap().as_mut() {
            if t.explicit {
                t.failed = true;
            }
        }
    }

    fn begin_txn(&self, explicit: bool) -> Result<()> {
        let (begin_seq, main, temp) = self.server.snapshot()?;
        *self.slot.txn.lock().unwrap() = Some(TxnState { explicit, begin_seq, main, temp, writer: None, catalog: None, savepoints: vec![], failed: false });
        Ok(())
    }

    /// What this connection reads now.
    fn view(&self) -> Result<ReadView> {
        let metrics = self.server.metrics.clone();
        let open = {
            let g = self.slot.txn.lock().unwrap();
            g.as_ref().map(|t| (t.catalog.clone(), t.main.clone(), t.temp.clone(), t.writer.clone()))
        };
        Ok(match open {
            Some((catalog, main, temp, writer)) => ReadView { catalog: catalog.unwrap_or_else(|| self.server.catalog_now()), main, temp, writer, metrics, scans: Default::default() },
            None => {
                let (_, main, temp) = self.server.snapshot()?;
                ReadView { catalog: self.server.catalog_now(), main, temp, writer: None, metrics, scans: Default::default() }
            }
        })
    }

    /// A DataFusion session that reads through this connection's view. `with_rowid` gives
    /// one table the row id column (the target of an UPDATE or DELETE).
    fn state_with(&self, with_rowid: Option<&str>) -> Result<SessionState> {
        let view = self.view()?;
        let mut st = self.server.session_for(&view.catalog, with_rowid);
        // The statement's start time: current_date, now() and the like are folded to it.
        st.mark_start_execution();
        st.config_mut().set_extension(Arc::new(view));
        Ok(st)
    }

    fn state(&self) -> Result<SessionState> {
        self.state_with(None)
    }

    /// The transaction's writer: takes the write lock (waiting up to the lock timeout) the first time.
    async fn ensure_writer(&self) -> Result<Arc<Mutex<TxWriter>>> {
        let explicit = {
            let g = self.slot.txn.lock().unwrap();
            let t = g.as_ref().ok_or_else(|| internal("no transaction"))?;
            if let Some(w) = &t.writer {
                return Ok(w.clone());
            }
            t.explicit
        };
        let guard = self.server.lock_writer().await?;
        let mut w = TxWriter::new(guard, self.server.main.clone(), self.server.temp.clone());
        // The transaction of one statement has read nothing yet: it reads the latest commit.
        let fresh = if explicit { None } else { Some(self.server.snapshot()?) };
        let mut g = self.slot.txn.lock().unwrap();
        let t = g.as_mut().ok_or_else(|| internal("no transaction"))?;
        if let Some((seq, main, temp)) = fresh {
            t.begin_seq = seq;
            t.main = main;
            t.temp = temp;
        }
        if !t.savepoints.is_empty() {
            w.mark();
        }
        let w = Arc::new(Mutex::new(w));
        t.writer = Some(w.clone());
        Ok(w)
    }

    /// Prepares to write a table: the writer, and a check that no one committed a change to
    /// the table after this transaction began.
    async fn write_table(&self, name: &str) -> Result<(Arc<TableInfo>, Arc<Mutex<TxWriter>>)> {
        let w = self.ensure_writer().await?;
        let cat = self.cat();
        let meta = cat.tables.get(name).ok_or_else(|| DepError::sql("42P01", format!("relation \"{name}\" does not exist")))?;
        let info = TableInfo::new(meta);
        let (explicit, begin_seq) = self.slot.txn.lock().unwrap().as_ref().map(|t| (t.explicit, t.begin_seq)).unwrap_or((false, 0));
        let first = !w.lock().unwrap().written.contains(&meta.tid);
        if first && explicit {
            let changed = self.server.table_seq.lock().unwrap().get(&meta.tid).copied().unwrap_or(0);
            if changed > begin_seq {
                return Err(DepError::sql("40001", format!("could not serialize access due to a concurrent update of {name}"))
                    .with_hint("Another transaction committed a change to this table after this transaction began. Roll back and run it again."));
            }
        }
        w.lock().unwrap().written.insert(meta.tid);
        Ok((info, w))
    }

    fn begin(&self) -> Result<Output> {
        if self.in_txn() {
            self.add_notes(vec!["There is already a transaction in progress.".into()]);
            return Ok(Output::Command("BEGIN".into()));
        }
        self.begin_txn(true)?;
        Ok(Output::Command("BEGIN".into()))
    }

    /// Commits the open transaction (explicit or not): one durable write to disk, then the new
    /// catalog is published, then the write lock is released.
    async fn commit_txn(&self) -> Result<()> {
        let Some(t) = self.slot.txn.lock().unwrap().take() else { return Ok(()) };
        let Some(w) = t.writer else { return Ok(()) };
        let new_cat = t.catalog;
        let old_cat = self.server.catalog_now();
        let written = {
            let mut w = w.lock().unwrap();
            let r = (|| -> Result<HashSet<u64>> {
                if let Some(c) = &new_cat {
                    // Remove the stored data of tables and indexes that are gone.
                    let final_tids: HashMap<u64, &TableMeta> = c.tables.values().map(|t| (t.tid, t)).collect();
                    let mut touched: HashSet<(bool, u64)> = w.created.clone();
                    for t in old_cat.tables.values() {
                        touched.insert((t.temporary, t.tid));
                    }
                    for (temp, tid, _) in w.built_indexes.clone() {
                        touched.insert((temp, tid));
                    }
                    for tid in w.written.clone() {
                        if let Some(m) = final_tids.get(&tid) {
                            touched.insert((m.temporary, tid));
                        }
                    }
                    for (temp, tid) in touched {
                        match final_tids.get(&tid) {
                            None => w.drop_table_data(temp, tid)?,
                            Some(m) => {
                                for name in w.stored_indexes(temp, tid)? {
                                    if m.index(&name).is_none() {
                                        w.drop_index_data(temp, tid, &name)?;
                                    }
                                }
                            }
                        }
                    }
                    w.put_meta("catalog", &crate::persist::stored_catalog(c)?)?;
                }
                let written = w.written.clone();
                blocking(|| w.commit())?;
                Ok(written)
            })();
            match r {
                Ok(x) => x,
                Err(e) => {
                    w.abort();
                    return Err(DepError::sql(&e.sqlstate, format!("COMMIT failed, and the transaction was rolled back: {}", e.message)));
                }
            }
        };
        let seq = self.server.commit_seq.load(Ordering::SeqCst) + 1;
        {
            let mut ts = self.server.table_seq.lock().unwrap();
            for tid in written {
                ts.insert(tid, seq);
            }
        }
        self.server.commit_seq.store(seq, Ordering::SeqCst);
        if let Some(c) = new_cat {
            self.server.install_catalog_arc(c);
        }
        w.lock().unwrap().release();
        Metrics::inc(&self.server.metrics.commits, 1);
        Ok(())
    }

    fn rollback_txn(&self) {
        if let Some(t) = self.slot.txn.lock().unwrap().take() {
            if let Some(w) = t.writer {
                w.lock().unwrap().abort();
            }
        }
    }

    async fn commit(&self) -> Result<Output> {
        let (open, failed) = {
            let g = self.slot.txn.lock().unwrap();
            (g.is_some(), g.as_ref().map(|t| t.failed).unwrap_or(false))
        };
        if !open {
            self.add_notes(vec!["There is no transaction in progress.".into()]);
            return Ok(Output::Command("COMMIT".into()));
        }
        if failed {
            self.rollback_txn();
            Metrics::inc(&self.server.metrics.rollbacks, 1);
            self.add_notes(vec!["The transaction had failed, so it was rolled back.".into()]);
            return Ok(Output::Command("ROLLBACK".into()));
        }
        self.commit_txn().await?;
        Ok(Output::Command("COMMIT".into()))
    }

    fn rollback(&self) -> Output {
        if self.slot.txn.lock().unwrap().is_none() {
            self.add_notes(vec!["There is no transaction in progress.".into()]);
        } else {
            self.rollback_txn();
            Metrics::inc(&self.server.metrics.rollbacks, 1);
        }
        Output::Command("ROLLBACK".into())
    }

    fn savepoint_name(sql: &str) -> Result<String> {
        let words: Vec<String> = sql.trim().trim_end_matches(';').split_whitespace().map(|w| w.to_string()).collect();
        let last = words.last().ok_or_else(|| DepError::sql("42601", "a savepoint name is needed"))?;
        let up = last.to_uppercase();
        if matches!(up.as_str(), "SAVEPOINT" | "TO" | "RELEASE" | "ROLLBACK") {
            return Err(DepError::sql("42601", "a savepoint name is needed"));
        }
        Ok(if last.starts_with('"') { last.trim_matches('"').to_string() } else { last.to_lowercase() })
    }

    fn savepoint(&self, sql: &str) -> Result<Output> {
        let name = Self::savepoint_name(sql)?;
        let mut g = self.slot.txn.lock().unwrap();
        let t = g.as_mut().filter(|t| t.explicit).ok_or_else(|| DepError::sql("25P01", "SAVEPOINT can only be used in transaction blocks"))?;
        let undo = t.writer.as_ref().map(|w| w.lock().unwrap().mark()).unwrap_or(0);
        t.savepoints.push(Savepoint { name, undo, catalog: t.catalog.clone() });
        Ok(Output::Command("SAVEPOINT".into()))
    }

    fn rollback_to(&self, sql: &str) -> Result<Output> {
        let name = Self::savepoint_name(sql)?;
        let mut g = self.slot.txn.lock().unwrap();
        let t = g.as_mut().filter(|t| t.explicit).ok_or_else(|| DepError::sql("25P01", "ROLLBACK TO SAVEPOINT can only be used in transaction blocks"))?;
        let at = t.savepoints.iter().rposition(|s| s.name == name).ok_or_else(|| DepError::sql("3B001", format!("savepoint \"{name}\" does not exist")))?;
        t.savepoints.truncate(at + 1);
        let sp = &t.savepoints[at];
        if let Some(w) = &t.writer {
            w.lock().unwrap().undo_to(sp.undo)?;
        }
        t.catalog = sp.catalog.clone();
        t.failed = false;
        Ok(Output::Command("ROLLBACK".into()))
    }

    fn release(&self, sql: &str) -> Result<Output> {
        let name = Self::savepoint_name(sql)?;
        let mut g = self.slot.txn.lock().unwrap();
        let t = g.as_mut().filter(|t| t.explicit).ok_or_else(|| DepError::sql("25P01", "RELEASE SAVEPOINT can only be used in transaction blocks"))?;
        let at = t.savepoints.iter().rposition(|s| s.name == name).ok_or_else(|| DepError::sql("3B001", format!("savepoint \"{name}\" does not exist")))?;
        t.savepoints.truncate(at);
        if t.savepoints.is_empty() {
            if let Some(w) = &t.writer {
                w.lock().unwrap().release_marks();
            }
        }
        Ok(Output::Command("RELEASE".into()))
    }

    // ------------------------------------------------------------------ audit and limits

    /// Applies the match limit of the role to the uses of protected data a statement is about
    /// to make, and keeps them for the audit log.
    fn admit_events(&self, events: Vec<Event>) -> Result<()> {
        if events.is_empty() || self.restore {
            return Ok(());
        }
        let role = self.role();
        let matches: u32 = events.iter().filter(|e| e.kind == "match").map(|e| e.count).sum();
        if matches > 0 {
            let limit = self.server.match_limit(&self.cat(), &role);
            if let Some(l) = limit {
                if let Err(used) = self.server.rate.admit(&role, l, matches) {
                    Metrics::inc(&self.server.metrics.rate_limited, 1);
                    let objects: Vec<String> = events.iter().filter(|e| e.kind == "match").map(|e| e.object.clone()).collect();
                    self.pending_audit.lock().unwrap().push(Event { kind: "rate_limited", object: objects.join(", "), domain: None, count: matches });
                    return Err(DepError::dep(
                        "DEP_MATCH_RATE_LIMITED",
                        format!("Role {role} has made {used} protected matches of the {} it is allowed; this statement would make {matches} more.", l.describe()),
                        Some("Wait for the period to pass, or ask a superuser to change the limit (ALTER ROLE ... MATCH LIMIT n PER MINUTE)."),
                    ));
                }
            }
        }
        let m = &self.server.metrics;
        Metrics::inc(&m.protected_matches, matches as u64);
        Metrics::inc(&m.disclosures, events.iter().filter(|e| e.kind == "disclose").count() as u64);
        Metrics::inc(&m.links, events.iter().filter(|e| e.kind == "link").count() as u64);
        self.pending_audit.lock().unwrap().extend(events);
        Ok(())
    }

    /// Writes the statement's audit records: its uses of protected data, and the refusal, if
    /// the DEP rules refused it.
    async fn flush_audit(&self, out: Option<&DepError>, first_word: &str) {
        if self.restore {
            return;
        }
        let events = std::mem::take(&mut *self.pending_audit.lock().unwrap());
        let role = self.role();
        let mk = |kind: &str, object: String, domain: Option<String>, count: u32, detail: Option<String>| AuditRecord {
            seq: 0,
            at: crate::audit::now_utc(),
            session_user: self.session_user.clone(),
            role: role.clone(),
            endpoint: self.endpoint.clone(),
            path: self.path.as_str().to_string(),
            kind: kind.to_string(),
            object,
            domain,
            count,
            detail,
        };
        let failed = out.is_some();
        let mut recs: Vec<AuditRecord> =
            events.into_iter().map(|e| mk(e.kind, e.object, e.domain, e.count, if failed && e.kind != "rate_limited" { Some("statement failed".into()) } else { None })).collect();
        if let Some(e) = out {
            if let Some(code) = e.code {
                if code != "DEP_MATCH_RATE_LIMITED" {
                    recs.push(mk("denied", first_word.to_string(), None, 0, Some(code.to_string())));
                }
            }
        }
        self.server.audit(recs).await;
    }

    fn show_audit(&self, sql: &str) -> Result<Output> {
        self.require_superuser("read the audit log")?;
        let limit = sql.split_whitespace().filter_map(|w| w.trim_end_matches(';').parse::<usize>().ok()).next().unwrap_or(100);
        let recs = crate::persist::read_audit(&self.server, limit)?;
        let names = ["seq", "at", "session_user", "role", "endpoint", "path", "kind", "object", "domain", "count", "detail"];
        let rows = recs
            .iter()
            .map(|r| {
                vec![
                    Some(r.seq.to_string()),
                    Some(r.at.clone()),
                    Some(r.session_user.clone()),
                    Some(r.role.clone()),
                    Some(r.endpoint.clone()),
                    Some(r.path.clone()),
                    Some(r.kind.clone()),
                    Some(r.object.clone()),
                    r.domain.clone(),
                    if r.count > 0 { Some(r.count.to_string()) } else { None },
                    r.detail.clone(),
                ]
            })
            .collect();
        text_rows(&names, rows)
    }

    // ------------------------------------------------------------------ key rotation

    /// `ALTER SYSTEM ROTATE MASTER KEY`: a new master key; every protected value is decrypted
    /// with the old key, then indexed and encrypted again with the new one, and every index
    /// is rebuilt, in one commit.
    async fn rotate_key(&self) -> Result<Output> {
        self.require_operational("Key rotation is")?;
        self.require_superuser("rotate the master key")?;
        if self.in_txn() {
            return Err(DepError::sql("25001", "ALTER SYSTEM ROTATE MASTER KEY cannot run inside a transaction block"));
        }
        let w = self.ensure_writer().await?;
        let old = self.server.keys.snapshot();
        let new_bytes = Keyring::random_bytes();
        let newk = Keyring::from_bytes(new_bytes);
        let cat = self.cat();
        let mut values = 0usize;
        {
            let mut wg = w.lock().unwrap();
            for meta in cat.tables.values() {
                let info = TableInfo::new(meta);
                let prot: Vec<(usize, usize, String)> = meta
                    .columns
                    .iter()
                    .filter_map(|c| {
                        let d = c.domain.clone()?;
                        Some((info.schema.index_of(&c.name).ok()?, info.schema.index_of(&format!("{}{CT_SUFFIX}", c.name)).ok()?, d))
                    })
                    .collect();
                if prot.is_empty() {
                    continue;
                }
                let mut fail = None;
                blocking(|| {
                    wg.for_each(&info, &mut |wr, chunk| {
                        let b = store::decode_rows(&info, &chunk)?;
                        let mut cols = b.columns().to_vec();
                        for (ii, ci, d) in &prot {
                            let cts = cols[*ci].as_any().downcast_ref::<StringArray>().ok_or_else(|| internal("ciphertext column"))?;
                            let mut idx = Vec::with_capacity(cts.len());
                            let mut ct = Vec::with_capacity(cts.len());
                            for r in 0..cts.len() {
                                if cts.is_null(r) {
                                    idx.push(None);
                                    ct.push(None);
                                    continue;
                                }
                                let Some(v) = old.decrypt(d, cts.value(r)) else {
                                    fail = Some(DepError::sql("XX001", format!("a protected value in {} could not be decrypted with the current key", meta.name)));
                                    return Err(internal("decrypt"));
                                };
                                idx.push(Some(newk.index_entry(d, &v)));
                                ct.push(Some(newk.encrypt(d, &v)));
                                values += 1;
                            }
                            cols[*ii] = Arc::new(StringArray::from(idx));
                            cols[*ci] = Arc::new(StringArray::from(ct));
                        }
                        let nb = RecordBatch::try_new(b.schema(), cols)?;
                        for ((rid, _), row) in chunk.iter().zip(encode_rows(&nb)?) {
                            wr.put_raw(&info, *rid, &row)?;
                        }
                        Ok(())
                    })
                })
                .map_err(|e| fail.take().unwrap_or(e))?;
                for ix in &info.indexes {
                    blocking(|| wg.build_index(&info, ix, false))?;
                }
            }
            wg.put_meta("key_fingerprint", newk.fingerprint().as_bytes())?;
        }
        crate::persist::write_new_key(&self.server, &newk)?;
        self.commit_txn().await?;
        crate::persist::finish_new_key(&self.server)?;
        self.server.keys.replace(new_bytes);
        self.pending_audit.lock().unwrap().push(Event { kind: "key_rotated", object: "master key".into(), domain: None, count: values as u32 });
        self.add_notes(vec![format!(
            "The master key was replaced: {values} protected values were indexed and encrypted again with the new key, and the indexes were rebuilt. The old key no longer opens this data."
        )]);
        Ok(Output::Command("ALTER SYSTEM".into()))
    }

    async fn execute_inner(&self, sql: &str) -> Result<Streamed> {
        let w = sqltext::leading_words(sql, 4);
        let w: Vec<&str> = w.iter().map(|s| s.as_str()).collect();
        let ddl = matches!(w.first(), Some(&"CREATE") | Some(&"DROP") | Some(&"GRANT") | Some(&"REVOKE") | Some(&"COMMENT") | Some(&"REINDEX"))
            || (w.first() == Some(&"ALTER") && w.get(1) != Some(&"SYSTEM"));
        if ddl {
            self.require_operational("Schema changes are")?;
            if !self.restore {
                self.ensure_writer().await?;
            }
        }
        let out: Output = match w.as_slice() {
            ["SET", "DEP", ".", name] | ["RESET", "DEP", ".", name] => {
                return Err(if *name == "PATH" {
                    DepError::dep(
                        "DEP_PATH_IMMUTABLE",
                        format!("The access path of this connection is {} and cannot be changed.", self.path.as_str()),
                        Some("The path is fixed at authentication. Connect through another endpoint instead."),
                    )
                } else {
                    DepError::sql("42704", format!("unrecognized configuration parameter \"dep.{}\"", name.to_lowercase()))
                });
            }
            ["SHOW", "DEP", ".", "PATH"] => text_rows(&["dep.path"], vec![vec![Some(self.path.as_str().to_string())]])?,
            ["SHOW", "TABLES" | "COLUMNS" | "FUNCTIONS" | "ROLES" | "GRANTS" | "ENDPOINTS" | "STORAGE" | "INDEXES" | "INDEX", ..] => return self.show(sql).await.map(Streamed::from),
            ["SHOW", "DEP", "NOTES", ..] => text_rows(&["note"], self.notes().into_iter().map(|n| vec![Some(n)]).collect())?,
            ["SHOW", "PRIVACY", "MATRIX", ..] => self.privacy_matrix()?,
            ["SET", "ROLE", ..] | ["RESET", "ROLE"] | ["SET", "SESSION", "AUTHORIZATION", ..] => self.set_role(sql)?,
            ["SHOW", "ROLE"] | ["SHOW", "CURRENT_USER"] | ["SHOW", "SESSION_AUTHORIZATION"] | ["SHOW", "SESSION", "AUTHORIZATION"] => {
                let v = if w[1] == "ROLE" || w[1] == "CURRENT_USER" { self.role() } else { self.session_user.clone() };
                text_rows(&[&w[1..].join(" ").to_lowercase()], vec![vec![Some(v)]])?
            }
            ["CREATE", "ROLE", ..] | ["CREATE", "USER", ..] | ["ALTER", "ROLE", ..] | ["ALTER", "USER", ..] => self.role_ddl(sql)?,
            ["GRANT", ..] | ["REVOKE", ..] => self.grant(sql)?,
            ["CREATE", "ENDPOINT", ..] => {
                self.require_superuser("create endpoints")?;
                self.create_endpoint(sql)?
            }
            ["CREATE", "FUNCTION", ..] | ["CREATE", "OR", "REPLACE", "FUNCTION"] => self.create_function(sql).await?,
            ["CREATE", "TABLE", ..] | ["CREATE", "TEMPORARY" | "TEMP", "TABLE", ..] => {
                self.require_superuser("create tables")?;
                self.create_table(sql).await?
            }
            ["CREATE", "INDEX", ..] | ["CREATE", "UNIQUE", "INDEX", ..] => {
                self.require_superuser("create indexes")?;
                self.create_index(sql).await?
            }
            ["REINDEX", ..] => {
                self.require_superuser("rebuild indexes")?;
                self.reindex(sql).await?
            }
            ["CREATE", ..] | ["COPY", ..] => {
                self.require_operational("Schema changes are")?;
                return Err(DepError::unsupported("Only CREATE TABLE, CREATE INDEX, CREATE FUNCTION and CREATE ENDPOINT are supported."));
            }
            ["ALTER", "SYSTEM", "ROTATE", ..] => self.rotate_key().await?,
            ["ALTER", ..] => return Err(DepError::unsupported("Only ALTER ROLE and ALTER SYSTEM ROTATE MASTER KEY are supported.")),
            ["DROP", ..] => self.drop(sql).await?,
            ["COMMENT", ..] => {
                let (name, text) = sqltext::parse_comment_on_function(sql)?;
                let role = self.role();
                self.cat_mut(|cat| {
                    let f = cat.functions.get(&name).ok_or_else(|| DepError::sql("42883", format!("function {name} does not exist")))?;
                    if f.owner != role && !cat.is_superuser(&role) {
                        return Err(denied(format!("must be owner of function {name}")));
                    }
                    let mut copy = clone_meta(f);
                    copy.comment = text;
                    cat.functions.insert(name, Arc::new(copy));
                    Ok(())
                })?;
                Output::Command("COMMENT".into())
            }
            ["INSERT", ..] | ["UPDATE", ..] | ["DELETE", ..] | ["MERGE", ..] | ["TRUNCATE", ..] => {
                self.require_operational("Writes are")?;
                return self.dml(sql).await.map(Streamed::from);
            }
            ["BEGIN", ..] | ["START", "TRANSACTION", ..] => self.begin()?,
            ["COMMIT", ..] | ["END", ..] => self.commit().await?,
            ["ROLLBACK", "TO", ..] => self.rollback_to(sql)?,
            ["ROLLBACK", ..] | ["ABORT", ..] => self.rollback(),
            ["SAVEPOINT", ..] => self.savepoint(sql)?,
            ["RELEASE", ..] => self.release(sql)?,
            ["SHOW", "TRANSACTION", ..] => text_rows(&["transaction_isolation"], vec![vec![Some("repeatable read".into())]])?,
            ["SHOW", "AUDIT", ..] => self.show_audit(sql)?,
            ["SET", ..] => Output::Command("SET".into()),
            _ => return self.query_or_dml(sql).await,
        };
        Ok(out.into())
    }

    async fn query_or_dml(&self, sql: &str) -> Result<Streamed> {
        sqltext::check_reserved(sql)?;
        let mut stmts = DFParser::parse_sql_with_dialect(sql, &PostgreSqlDialect {})?;
        if stmts.len() != 1 {
            return Err(DepError::sql("42601", "expected one statement"));
        }
        match stmts.pop_front().unwrap() {
            DFStatement::Statement(s) => match *s {
                SqlStatement::Query(_) => {
                    let plan = self.query_plan(DFStatement::Statement(s)).await?;
                    self.run_query(plan).await
                }
                SqlStatement::Explain { statement, .. } => self.explain(*statement).await.map(Streamed::from),
                other => Err(DepError::unsupported(format!(
                    "Statement not supported: {}",
                    other.to_string().split_whitespace().take(3).collect::<Vec<_>>().join(" ")
                ))),
            },
            DFStatement::Explain(e) => match *e.statement {
                DFStatement::Statement(s) => self.explain(*s).await.map(Streamed::from),
                _ => Err(DepError::unsupported("EXPLAIN of this statement is not supported.")),
            },
            _ => Err(DepError::unsupported("This statement is not supported.")),
        }
    }

    async fn dml(&self, sql: &str) -> Result<Output> {
        sqltext::check_reserved(sql)?;
        let mut stmts = DFParser::parse_sql_with_dialect(sql, &PostgreSqlDialect {})?;
        if stmts.len() != 1 {
            return Err(DepError::sql("42601", "expected one statement"));
        }
        match stmts.pop_front().unwrap() {
            DFStatement::Statement(s) => match *s {
                SqlStatement::Insert(ins) => self.insert(ins).await,
                SqlStatement::Update(up) => self.update(up).await,
                SqlStatement::Delete(del) => self.delete(del).await,
                other => Err(DepError::unsupported(format!(
                    "Statement not supported: {}",
                    other.to_string().split_whitespace().take(3).collect::<Vec<_>>().join(" ")
                ))),
            },
            _ => Err(DepError::unsupported("This statement is not supported.")),
        }
    }

    /// Plans a query with DataFusion, then applies the DEP rules to the plan.
    async fn query_plan(&self, stmt: DFStatement) -> Result<LogicalPlan> {
        let state = self.state()?;
        let plan = state.statement_to_plan(stmt).await?;
        let (out, events) = {
            let cat = self.cat();
            let mut an = self.analyzer(&cat, None);
            let out = self.query_plan_inner(&cat, &mut an, &plan);
            (out, an.take_events())
        };
        let plan = out?;
        self.admit_events(events)?;
        Ok(plan)
    }

    fn query_plan_inner(&self, cat: &Catalog, an: &mut Analyzer<'_>, plan: &LogicalPlan) -> Result<LogicalPlan> {
        let node = an.analyze(plan)?;
        an.close_top(&node)?;
        self.check_uses(cat, &privileges::collect(plan, cat))?;
        let mut notes = an.take_notes();
        for (_, name, t) in Analyzer::visible(&node) {
            match t {
                Taint::Prot { domain, desc, .. } => notes.push(format!("Output column {name} is PROTECTED column {desc} (MATCH DOMAIN {domain}) and is returned as NULL.")),
                Taint::Disc { domain, desc } => notes.push(format!("Output column {name} is disclosed: {desc} (DISCLOSES {domain}).")),
                _ => {}
            }
        }
        self.add_notes(notes);
        Analyzer::finalize(node)
    }

    /// Who may reach each protected column, and how: the endpoints (paths) a role can use,
    /// whether it may match the column, and the declared functions it may call that
    /// disclose or link the column's match domain. A role that is not a superuser sees its own row only.
    fn privacy_matrix(&self) -> Result<Output> {
        let cat = self.cat();
        let me = self.role();
        let su = cat.is_superuser(&me);
        let names = ["match_domain", "column", "role", "paths", "may_match", "discloses_via", "links_via"];
        let mut rows = Vec::new();
        for t in cat.tables.values() {
            for c in t.columns.iter().filter(|c| c.domain.is_some()) {
                let domain = c.domain.clone().unwrap();
                for r in cat.roles.values().filter(|r| su || r.name == me) {
                    let paths: Vec<String> = cat.endpoints.iter().filter(|(e, _)| r.login && cat.can_connect(&r.name, e)).map(|(e, p)| format!("{e} ({})", p.as_str())).collect();
                    let via = |kind: DomainPriv| -> String {
                        cat.functions
                            .values()
                            .filter(|f| match kind {
                                DomainPriv::Disclose => f.discloses.contains(&domain),
                                DomainPriv::Link => f.links.contains(&domain),
                            })
                            .filter(|f| cat.can_execute(&r.name, f))
                            .map(|f| format!("{}()", f.name))
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    rows.push(vec![
                        Some(domain.clone()),
                        Some(format!("{}.{}", t.name, c.name)),
                        Some(r.name.clone()),
                        Some(paths.join(", ")),
                        Some(if cat.can_select(&r.name, &t.name, &c.name) { "yes".into() } else { "no".into() }),
                        Some(via(DomainPriv::Disclose)),
                        Some(via(DomainPriv::Link)),
                    ]);
                }
            }
        }
        text_rows(&names, rows)
    }

    async fn run_query(&self, plan: LogicalPlan) -> Result<Streamed> {
        let df = DataFrame::new(self.state()?, plan);
        let schema = Arc::new(df.schema().as_arrow().clone());
        let stream = df.execute_stream().await?;
        Ok(Streamed::Rows { schema, stream })
    }

    async fn explain(&self, stmt: SqlStatement) -> Result<Output> {
        if !matches!(stmt, SqlStatement::Query(_)) {
            return Err(DepError::unsupported("EXPLAIN supports queries."));
        }
        let plan = self.query_plan(DFStatement::Statement(Box::new(stmt))).await?;
        let mut text = format!("{}", plan.display_indent());
        // How the rows are read: index lookups or every row.
        let state = self.state()?;
        let log = state.config().get_extension::<ReadView>().map(|v| v.scans.clone());
        let df = DataFrame::new(state, plan);
        if df.create_physical_plan().await.is_ok() {
            if let Some(log) = log {
                let scans = log.lock().unwrap().clone();
                if !scans.is_empty() {
                    text.push_str("\nRead:");
                    for s in scans {
                        text.push_str(&format!("\n  {s}"));
                    }
                }
            }
        }
        let lines: Vec<Vec<Option<String>>> = text.lines().map(|l| vec![Some(l.to_string())]).collect();
        text_rows(&["dep_plan"], lines)
    }

    /// The stored form of a table (index entries and ciphertexts), without the DEP rules.
    async fn stored_rows(&self, table: &str, limit: Option<usize>) -> Result<Output> {
        if !self.cat().tables.contains_key(table) {
            return Err(DepError::sql("42P01", format!("relation \"{table}\" does not exist")));
        }
        let ctx = SessionContext::new_with_state(self.state()?);
        let mut df = ctx.table(table).await?;
        if let Some(n) = limit {
            df = df.limit(0, Some(n))?;
        }
        let schema = Arc::new(df.schema().as_arrow().clone());
        Ok(Output::Rows { schema, batches: df.collect().await? })
    }

    // ------------------------------------------------------------------ DDL

    fn create_endpoint(&self, sql: &str) -> Result<Output> {
        let (name, mode) = sqltext::parse_create_endpoint(sql)?;
        let path = match mode.as_str() {
            "OPERATIONAL" => PathMode::Operational,
            "MASKED_READ" => PathMode::MaskedRead,
            _ => return Err(DepError::sql("22023", format!("invalid endpoint mode \"{mode}\"")).with_hint("The modes are OPERATIONAL and MASKED_READ.")),
        };
        self.cat_mut(|cat| {
            if cat.endpoints.contains_key(&name) {
                return Err(DepError::sql("42710", format!("endpoint \"{name}\" already exists")));
            }
            cat.endpoints.insert(name, path);
            Ok(())
        })?;
        Ok(Output::Command("CREATE ENDPOINT".into()))
    }

    async fn create_table(&self, sql: &str) -> Result<Output> {
        let (stripped, protected) = sqltext::strip_protected(sql)?;
        let mut parsed = datafusion::sql::sqlparser::parser::Parser::parse_sql(&PostgreSqlDialect {}, &stripped)?;
        let Some(SqlStatement::CreateTable(ct)) = parsed.pop() else {
            return Err(DepError::sql("42601", "expected CREATE TABLE"));
        };
        if ct.query.is_some() {
            return Err(DepError::unsupported("CREATE TABLE ... AS is not supported. Create the table, then INSERT ... SELECT."));
        }
        let name = object_name(&ct.name);
        let cat = self.cat();
        if cat.tables.contains_key(&name) {
            if ct.if_not_exists {
                return Ok(Output::Command("CREATE TABLE".into()));
            }
            return Err(DepError::sql("42P07", format!("relation \"{name}\" already exists")));
        }
        let mut columns = Vec::new();
        let mut uniques: Vec<Vec<String>> = Vec::new();
        let mut table_pk: Vec<String> = Vec::new();
        for c in &ct.constraints {
            match c {
                TableConstraint::ForeignKey(_) => {
                    return Err(DepError::unsupported("Declare references on the column (col TYPE REFERENCES t (c)), not as a table constraint."));
                }
                TableConstraint::PrimaryKey(pk) => table_pk = index_columns(&pk.columns)?,
                TableConstraint::Unique(u) => uniques.push(index_columns(&u.columns)?),
                other => return Err(DepError::unsupported(format!("Table constraint not supported: {other}"))),
            }
        }
        for cd in &ct.columns {
            let cname = ident_name(&cd.name);
            let type_name = cd.data_type.to_string();
            let data_type = parse_type(&type_name)?;
            let mut primary_key = table_pk.contains(&cname);
            let mut not_null = false;
            let mut references = None;
            for o in &cd.options {
                match &o.option {
                    ColumnOption::PrimaryKey(_) => primary_key = true,
                    ColumnOption::NotNull => not_null = true,
                    ColumnOption::Null => {}
                    ColumnOption::Unique(_) => uniques.push(vec![cname.clone()]),
                    ColumnOption::ForeignKey(fk) => {
                        for a in [&fk.on_delete, &fk.on_update].into_iter().flatten() {
                            if !matches!(a, ast::ReferentialAction::NoAction | ast::ReferentialAction::Restrict) {
                                return Err(DepError::unsupported(format!("ON DELETE / ON UPDATE {a} is not supported: references are checked (NO ACTION).")));
                            }
                        }
                        let col = fk.referred_columns.first().map(ident_name).ok_or_else(|| DepError::sql("42601", "REFERENCES needs a column"))?;
                        references = Some((object_name(&fk.foreign_table), col));
                    }
                    ColumnOption::Default(_) => return Err(DepError::unsupported("DEFAULT values are not supported.")),
                    ColumnOption::Check(_) => return Err(DepError::unsupported("CHECK constraints are not supported.")),
                    _ => {}
                }
            }
            let prot = protected.iter().find(|p| p.column == cname);
            let mut domain = prot.and_then(|p| p.domain.clone());
            if let Some((rt, rc)) = &references {
                let rmeta = if *rt == name { None } else { Some(cat.tables.get(rt).ok_or_else(|| DepError::sql("42P01", format!("relation \"{rt}\" does not exist")))?) };
                if let Some(rmeta) = rmeta {
                    if rmeta.temporary && !ct.temporary {
                        return Err(DepError::sql("42P16", format!("constraints on permanent tables may reference only permanent tables ({rt} is TEMPORARY)")));
                    }
                    let rcol = rmeta
                        .column(rc)
                        .ok_or_else(|| DepError::sql("42703", format!("column \"{rc}\" referenced in foreign key constraint does not exist")))?;
                    match (prot.is_some(), &rcol.domain) {
                        (false, Some(d)) => {
                            return Err(DepError::dep(
                                "DEP_REFERENCE_MISMATCH",
                                format!("Column {name}.{cname} references PROTECTED column {rt}.{rc} (match domain {d}) but is not PROTECTED."),
                                Some(&format!("Declare it {type_name} PROTECTED REFERENCES {rt} ({rc}), so the reference is kept as an index entry.")),
                            ))
                        }
                        (true, None) => {
                            return Err(DepError::dep(
                                "DEP_REFERENCE_MISMATCH",
                                format!("Column {name}.{cname} is PROTECTED but references the ordinary column {rt}.{rc}."),
                                Some("A reference and the column it references must have the same protection and match domain."),
                            ))
                        }
                        (true, Some(d)) => {
                            if let Some(own) = &domain {
                                if own != d {
                                    return Err(DepError::dep(
                                        "DEP_MATCH_DOMAIN_MISMATCH",
                                        format!("Column {name}.{cname} (match domain {own}) references {rt}.{rc} (match domain {d})."),
                                        Some("A reference and the column it references must have the same match domain."),
                                    ));
                                }
                            }
                            domain = Some(d.clone());
                        }
                        (false, None) => {}
                    }
                }
            }
            if prot.is_some() {
                if data_type != DataType::Utf8 {
                    return Err(DepError::sql("42804", format!("PROTECTED column {cname} must be TEXT")));
                }
                if domain.is_none() {
                    return Err(DepError::sql("42601", format!("PROTECTED column {cname} needs a match domain"))
                        .with_hint("Write TEXT PROTECTED MATCH DOMAIN name, or PROTECTED REFERENCES a protected column."));
                }
            }
            columns.push(ColumnMeta { name: cname, data_type, type_name, domain, primary_key, references, not_null: not_null || primary_key });
        }
        for c in table_pk.iter().chain(uniques.iter().flatten()) {
            if !columns.iter().any(|x| &x.name == c) {
                return Err(DepError::sql("42703", format!("column \"{c}\" named in key does not exist")));
            }
        }
        let tid = self.server.next_tid.fetch_add(1, Ordering::SeqCst);
        let mut meta = TableMeta { name: name.clone(), tid, columns, indexes: vec![], temporary: ct.temporary };
        derive_indexes(&mut meta);
        for u in uniques {
            if meta.indexes.iter().any(|i| i.unique && i.columns == u) {
                continue;
            }
            let iname = format!("{}_{}_key", name, u.join("_"));
            if let Some(i) = meta.indexes.iter_mut().find(|i| i.columns == u && !i.unique) {
                i.unique = true;
                i.name = iname;
            } else {
                meta.indexes.push(IndexMeta { name: iname, columns: u, unique: true, primary: false, auto: true, enforced: true });
            }
        }
        // A referenced column needs an index, to check references against.
        let mut parents = Vec::new();
        let refs: Vec<(String, String)> = meta.columns.iter().filter_map(|c| c.references.clone()).collect();
        for (rt, rc) in refs {
            if rt == name {
                ensure_index(&mut meta, &rc);
            } else if let Some(p) = cat.tables.get(&rt) {
                if p.single_index(&rc).is_none() {
                    parents.push((rt, rc));
                }
            }
        }
        let w = self.ensure_writer().await?;
        for (pt, pc) in &parents {
            let mut p = cat.tables.get(pt).unwrap().clone();
            ensure_index(&mut p, pc);
            let info = TableInfo::new(&p);
            let ix = info.indexes.iter().find(|i| i.meta.columns.len() == 1 && &i.meta.columns[0] == pc).cloned().unwrap();
            {
                let mut wg = w.lock().unwrap();
                blocking(|| wg.build_index(&info, &ix, false))?;
                wg.written.insert(p.tid);
            }
            self.cat_mut(|cat| {
                cat.tables.insert(pt.clone(), p);
                Ok(())
            })?;
        }
        {
            let mut wg = w.lock().unwrap();
            wg.created.insert((meta.temporary, tid));
            wg.written.insert(tid);
        }
        let temporary = meta.temporary;
        self.cat_mut(|cat| {
            cat.tables.insert(name.clone(), meta);
            Ok(())
        })?;
        if temporary {
            self.add_notes(vec![format!("{name} is a TEMPORARY table: it is kept in memory only, and is gone when the server stops.")]);
        }
        Ok(Output::Command("CREATE TABLE".into()))
    }

    async fn create_index(&self, sql: &str) -> Result<Output> {
        let mut parsed = datafusion::sql::sqlparser::parser::Parser::parse_sql(&PostgreSqlDialect {}, sql)?;
        let Some(SqlStatement::CreateIndex(ci)) = parsed.pop() else {
            return Err(DepError::sql("42601", "expected CREATE INDEX"));
        };
        if ci.predicate.is_some() || !ci.include.is_empty() {
            return Err(DepError::unsupported("Partial indexes and INCLUDE are not supported."));
        }
        let table = object_name(&ci.table_name);
        let columns = index_columns(&ci.columns)?;
        let cat = self.cat();
        let meta = cat.tables.get(&table).ok_or_else(|| DepError::sql("42P01", format!("relation \"{table}\" does not exist")))?.clone();
        for c in &columns {
            if meta.column(c).is_none() {
                return Err(DepError::sql("42703", format!("column \"{c}\" does not exist")));
            }
        }
        let name = match &ci.name {
            Some(n) => object_name(n),
            None => format!("{table}_{}_idx", columns.join("_")),
        };
        if cat.index_owner(&name).is_some() || cat.tables.contains_key(&name) {
            if ci.if_not_exists {
                self.add_notes(vec![format!("Index {name} already exists; nothing was done.")]);
                return Ok(Output::Command("CREATE INDEX".into()));
            }
            return Err(DepError::sql("42P07", format!("relation \"{name}\" already exists")));
        }
        let (_, w) = self.write_table(&table).await?;
        let mut m = meta.clone();
        m.indexes.push(IndexMeta { name: name.clone(), columns: columns.clone(), unique: ci.unique, primary: false, auto: false, enforced: true });
        let info = TableInfo::new(&m);
        let ix = info.indexes.iter().find(|i| i.meta.name == name).cloned().unwrap();
        {
            let mut wg = w.lock().unwrap();
            blocking(|| wg.build_index(&info, &ix, true))?;
        }
        let prot: Vec<&String> = columns.iter().filter(|c| m.column(c).map(|x| x.domain.is_some()).unwrap_or(false)).collect();
        if !prot.is_empty() {
            self.add_notes(vec![format!(
                "Index {name} keys PROTECTED column(s) {} by their blind index entries: it answers equality only, and holds no plaintext.",
                prot.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            )]);
        }
        self.cat_mut(|cat| {
            cat.tables.insert(table.clone(), m);
            Ok(())
        })?;
        Ok(Output::Command("CREATE INDEX".into()))
    }

    /// `REINDEX TABLE t` (or `REINDEX INDEX i`): rebuilds indexes. A unique index whose
    /// uniqueness was not enforced (duplicates found in older data) is enforced again when its
    /// keys are now unique.
    async fn reindex(&self, sql: &str) -> Result<Output> {
        let words: Vec<String> = sql.trim().trim_end_matches(';').split_whitespace().map(|w| w.trim_matches('"').to_lowercase()).collect();
        let (kind, target) = match words.as_slice() {
            [_, k, t] => (k.to_uppercase(), t.clone()),
            _ => return Err(DepError::sql("42601", "REINDEX TABLE name, or REINDEX INDEX name")),
        };
        let cat = self.cat();
        let (table, only) = match kind.as_str() {
            "TABLE" => (target.clone(), None),
            "INDEX" => (cat.index_owner(&target).map(|t| t.name.clone()).ok_or_else(|| DepError::sql("42704", format!("index \"{target}\" does not exist")))?, Some(target.clone())),
            _ => return Err(DepError::sql("42601", "REINDEX TABLE name, or REINDEX INDEX name")),
        };
        let (info, w) = self.write_table(&table).await?;
        let mut m = info.meta.clone();
        let mut notes = Vec::new();
        for ix in &info.indexes {
            if only.as_ref().map(|o| *o != ix.meta.name).unwrap_or(false) {
                continue;
            }
            let dups = {
                let mut wg = w.lock().unwrap();
                blocking(|| wg.build_index(&info, ix, false))?
            };
            if ix.meta.unique {
                let mi = m.indexes.iter_mut().find(|i| i.name == ix.meta.name).unwrap();
                let was = mi.enforced;
                mi.enforced = dups == 0;
                if dups > 0 {
                    notes.push(format!("{}: {dups} duplicate key(s); uniqueness is still not enforced.", ix.meta.name));
                } else if !was {
                    notes.push(format!("{}: no duplicate keys; uniqueness is enforced again.", ix.meta.name));
                }
            }
        }
        self.add_notes(notes);
        self.cat_mut(|cat| {
            cat.tables.insert(table.clone(), m);
            Ok(())
        })?;
        Ok(Output::Command("REINDEX".into()))
    }

    async fn create_function(&self, sql: &str) -> Result<Output> {
        let def = sqltext::parse_create_function(sql)?;
        if let Some(l) = &def.language {
            if l != "sql" {
                return Err(DepError::dep(
                    "DEP_DECLARED_LANGUAGE",
                    format!("Function {} is written in LANGUAGE {l}.", def.name),
                    Some("Declared functions must be LANGUAGE sql, so the engine can analyze their bodies."),
                ));
            }
        }
        let owner = self.role();
        {
            let cat = self.cat();
            if let Some(old) = cat.functions.get(&def.name) {
                if !def.or_replace {
                    return Err(DepError::sql("42723", format!("function \"{}\" already exists", def.name)).with_hint("Use CREATE OR REPLACE FUNCTION."));
                }
                if old.owner != owner && !cat.is_superuser(&owner) {
                    return Err(denied(format!("must be owner of function {}", def.name)));
                }
            }
            for (list, kind, word) in [(&def.discloses, DomainPriv::Disclose, "DISCLOSE"), (&def.links, DomainPriv::Link, "LINK")] {
                for d in list.iter() {
                    if !self.restore && !cat.has_domain_priv(&owner, d, kind) {
                        return Err(DepError::dep(
                            "DEP_DECLARATION_PRIVILEGE",
                            format!("Role {owner} may not declare functions that {}S match domain {d}.", word),
                            Some(&format!("A superuser can allow it: GRANT {word} ON MATCH DOMAIN {d} TO {owner}.")),
                        ));
                    }
                }
            }
            let known: Vec<String> = cat.tables.values().flat_map(|t| t.columns.iter().filter_map(|c| c.domain.clone())).collect();
            for d in def.discloses.iter().chain(def.links.iter()) {
                if !known.contains(d) {
                    return Err(DepError::sql("42704", format!("match domain \"{d}\" does not exist")));
                }
            }
        }
        let params: Vec<(String, DataType, String)> = def.params.iter().map(|(n, t)| Ok((n.clone(), parse_type(t)?, t.clone()))).collect::<Result<_>>()?;
        let returns: Vec<(String, DataType, String)> = def.returns.iter().map(|(n, t)| Ok((n.clone(), parse_type(t)?, t.clone()))).collect::<Result<_>>()?;
        let bound = sqltext::bind_params(&def.body, &def.params)?;
        sqltext::check_reserved(&bound)?;
        let mut stmts = DFParser::parse_sql_with_dialect(&bound, &PostgreSqlDialect {})?;
        let stmt = match (stmts.len(), stmts.pop_front()) {
            (1, Some(DFStatement::Statement(s))) if matches!(*s, SqlStatement::Query(_)) => DFStatement::Statement(s),
            _ => return Err(DepError::unsupported("The body of a declared function must be a single SELECT.")),
        };
        let plan = self.state()?.statement_to_plan(stmt).await?;
        let region = Region { name: def.name.clone(), discloses: def.discloses.clone(), links: def.links.clone() };
        let (final_plan, disclosed, param_match, uses) = {
            let cat = self.cat();
            let mut an = Analyzer::new(PathMode::Operational, Some(region), &cat, self.server.index_udf.clone(), self.server.decrypt_udf.clone());
            let node = an.analyze(&plan)?;
            an.close_top(&node)?;
            // The body runs with the owner's privileges, so the owner must hold them now.
            let uses = privileges::collect(&plan, &cat);
            if !self.restore {
                self.check_uses(&cat, &uses)?;
            }
            let visible = Analyzer::visible(&node);
            if visible.len() != returns.len() {
                return Err(DepError::sql("42P13", format!("the body returns {} column(s), but RETURNS TABLE declares {}", visible.len(), returns.len())));
            }
            let mut exprs = Vec::new();
            let mut disclosed = Vec::new();
            for ((q, n, t), (rn, rt, _)) in visible.iter().zip(returns.iter()) {
                let colexpr = Expr::Column(Column::new(q.clone(), n));
                let e = match t {
                    Taint::Prot { .. } => cast(lit(ScalarValue::Null), rt.clone()),
                    Taint::Disc { domain, .. } => {
                        disclosed.push((rn.clone(), domain.clone()));
                        cast(colexpr, rt.clone())
                    }
                    _ => cast(colexpr, rt.clone()),
                };
                exprs.push(e.alias(rn));
            }
            let p = LogicalPlan::Projection(Projection::try_new(exprs, Arc::new(node.plan))?);
            (p, disclosed, an.param_match, uses)
        };
        let comment = self.cat().functions.get(&def.name).and_then(|f| f.comment.clone());
        let meta = Arc::new(FunctionMeta {
            name: def.name.clone(),
            params,
            returns,
            masked_ok: def.discloses.is_empty() && !param_match,
            discloses: def.discloses,
            links: def.links,
            body: def.body.trim().to_string(),
            comment,
            plan: final_plan,
            disclosed,
            reads: uses.columns.iter().cloned().collect(),
            scans: uses.tables.iter().filter(|t| !uses.columns.iter().any(|(tt, _)| tt == *t)).cloned().collect(),
            calls: uses.functions.iter().cloned().collect(),
            owner,
            source: sql.trim().to_string(),
        });
        self.cat_mut(|cat| {
            cat.function_order.retain(|n| *n != def.name);
            cat.function_order.push(def.name.clone());
            cat.functions.insert(def.name, meta);
            Ok(())
        })?;
        Ok(Output::Command("CREATE FUNCTION".into()))
    }

    async fn drop(&self, sql: &str) -> Result<Output> {
        let (kind, if_exists, name) = sqltext::parse_drop(sql)?;
        let role = self.role();
        let superuser = self.cat().is_superuser(&role);
        if kind != "FUNCTION" && !superuser {
            return Err(denied(format!("permission denied to drop {}", kind.to_lowercase())).with_hint("Only a superuser can do this."));
        }
        let tag = format!("DROP {kind}");
        self.cat_mut(|cat| {
            match kind.as_str() {
                "ROLE" | "USER" => {
                    if name == ADMIN || name == role {
                        return Err(DepError::sql("55006", format!("role \"{name}\" cannot be dropped")));
                    }
                    if cat.roles.remove(&name).is_none() {
                        return if if_exists { Ok(()) } else { Err(DepError::sql("42704", format!("role \"{name}\" does not exist"))) };
                    }
                    cat.table_privs.retain(|(r, _), _| *r != name);
                    cat.domain_privs.retain(|(r, _), _| *r != name);
                    for s in cat.endpoint_grants.values_mut() {
                        s.remove(&name);
                    }
                    for s in cat.fn_execute.values_mut() {
                        s.remove(&name);
                    }
                    Ok(())
                }
                "TABLE" => {
                    if !cat.tables.contains_key(&name) {
                        return if if_exists { Ok(()) } else { Err(DepError::sql("42P01", format!("table \"{name}\" does not exist"))) };
                    }
                    if let Some((child, col, _)) = cat.references_to(&name).into_iter().find(|(c, _, _)| *c != name) {
                        return Err(DepError::sql("2BP01", format!("cannot drop table {name} because other objects depend on it: {child}.{col} references it"))
                            .with_hint(format!("Drop {child} first.")));
                    }
                    cat.tables.remove(&name);
                    cat.table_privs.retain(|(_, t), _| *t != name);
                    Ok(())
                }
                "INDEX" => {
                    let Some(owner) = cat.index_owner(&name).map(|t| t.name.clone()) else {
                        return if if_exists { Ok(()) } else { Err(DepError::sql("42704", format!("index \"{name}\" does not exist"))) };
                    };
                    let t = cat.tables.get_mut(&owner).unwrap();
                    if t.index(&name).map(|i| i.auto).unwrap_or(false) {
                        return Err(DepError::sql("2BP01", format!("cannot drop index {name}: the engine keeps it for a key, a reference or a PROTECTED column of {owner}")));
                    }
                    t.indexes.retain(|i| i.name != name);
                    Ok(())
                }
                "FUNCTION" => {
                    if let Some(f) = cat.functions.get(&name) {
                        if f.owner != role && !superuser {
                            return Err(denied(format!("must be owner of function {name}")));
                        }
                    }
                    cat.fn_execute.remove(&name);
                    cat.function_order.retain(|n| *n != name);
                    if cat.functions.remove(&name).is_none() {
                        return if if_exists { Ok(()) } else { Err(DepError::sql("42883", format!("function {name} does not exist"))) };
                    }
                    Ok(())
                }
                "ENDPOINT" => {
                    if cat.endpoints.remove(&name).is_none() && !if_exists {
                        return Err(DepError::sql("42704", format!("endpoint \"{name}\" does not exist")));
                    }
                    Ok(())
                }
                _ => Err(DepError::unsupported(format!("DROP {kind} is not supported."))),
            }
        })?;
        Ok(Output::Command(tag))
    }

    // ------------------------------------------------------------------ catalog views

    /// `SHOW TABLES`, `SHOW COLUMNS FROM t`, `SHOW INDEXES [FROM t]`, `SHOW FUNCTIONS`,
    /// `SHOW ROLES`, `SHOW GRANTS`, `SHOW ENDPOINTS`, and, for superusers, `SHOW STORAGE t [n]`.
    async fn show(&self, sql: &str) -> Result<Output> {
        let words: Vec<String> = sql.trim().trim_end_matches(';').split_whitespace().map(|w| w.trim_matches('"').to_string()).collect();
        let what = words.get(1).map(|w| w.to_uppercase()).unwrap_or_default();
        let arg = words.iter().skip(2).find(|w| !w.eq_ignore_ascii_case("FROM") && !w.eq_ignore_ascii_case("LIMIT")).map(|w| w.to_lowercase());
        let text = |v: &str| Some(v.to_string());
        let cat = self.cat();
        match what.as_str() {
            "TABLES" => {
                let view = self.view()?;
                let mut rows = Vec::new();
                for t in cat.tables.values() {
                    let info = TableInfo::new(t);
                    let n = view.source(&info).count(&info)?;
                    rows.push(vec![
                        text(&t.name),
                        text(&t.columns.iter().filter(|c| c.domain.is_some()).map(|c| c.name.clone()).collect::<Vec<_>>().join(", ")),
                        Some(n.to_string()),
                        text(if t.temporary { "yes" } else { "" }),
                    ]);
                }
                text_rows(&["table", "protected_columns", "rows", "temporary"], rows)
            }
            "COLUMNS" => {
                let name = arg.ok_or_else(|| DepError::sql("42601", "SHOW COLUMNS FROM table"))?;
                let t = cat.tables.get(&name).ok_or_else(|| DepError::sql("42P01", format!("relation \"{name}\" does not exist")))?;
                text_rows(
                    &["column", "type", "match_domain", "primary_key", "references", "not_null", "unique"],
                    t.columns
                        .iter()
                        .map(|c| {
                            let unique = t.indexes.iter().any(|i| i.unique && i.columns.len() == 1 && i.columns[0] == c.name);
                            vec![
                                text(&c.name),
                                text(&c.type_name),
                                c.domain.clone(),
                                text(if c.primary_key { "yes" } else { "" }),
                                c.references.as_ref().map(|(t, c)| format!("{t} ({c})")),
                                text(if c.not_null { "yes" } else { "" }),
                                text(if unique { "yes" } else { "" }),
                            ]
                        })
                        .collect(),
                )
            }
            "INDEXES" | "INDEX" => {
                let mut rows = Vec::new();
                for t in cat.tables.values().filter(|t| arg.as_ref().map(|a| *a == t.name).unwrap_or(true)) {
                    for i in &t.indexes {
                        let prot = i.columns.iter().any(|c| t.column(c).map(|x| x.domain.is_some()).unwrap_or(false));
                        rows.push(vec![
                            text(&t.name),
                            text(&i.name),
                            text(&i.columns.join(", ")),
                            text(if i.primary { "primary key" } else if i.unique { "unique" } else { "" }),
                            text(if !i.unique { "" } else if i.enforced { "yes" } else { "no (duplicate keys)" }),
                            text(if i.auto { "engine" } else { "CREATE INDEX" }),
                            text(if prot { "blind index entries" } else { "values" }),
                        ]);
                    }
                }
                if let Some(a) = &arg {
                    if !cat.tables.contains_key(a) {
                        return Err(DepError::sql("42P01", format!("relation \"{a}\" does not exist")));
                    }
                }
                text_rows(&["table", "index", "columns", "kind", "enforced", "made_by", "keys"], rows)
            }
            "FUNCTIONS" => text_rows(
                &["function", "discloses", "links", "owner", "masked_path", "comment", "executable_by", "body"],
                cat.function_order
                    .iter()
                    .filter_map(|n| cat.functions.get(n))
                    .map(|f| {
                        vec![
                            text(&f.signature()),
                            text(&f.discloses.join(", ")),
                            text(&f.links.join(", ")),
                            text(&f.owner),
                            text(if f.masked_ok { "yes" } else { "no" }),
                            f.comment.clone(),
                            text(&cat.fn_execute.get(&f.name).map(|s| s.iter().cloned().collect::<Vec<_>>().join(", ")).unwrap_or_default()),
                            text(&f.body),
                        ]
                    })
                    .collect(),
            ),
            "ROLES" => {
                let default = *self.server.default_match_limit.read().unwrap();
                text_rows(
                    &["role", "superuser", "login", "endpoints", "match_limit"],
                    describe_roles(&cat)
                        .into_iter()
                        .map(|(r, eps)| {
                            let limit = match r.match_limit {
                                RoleLimit::Limit(l) => l.describe(),
                                RoleLimit::Unlimited => "none".into(),
                                RoleLimit::Default if r.superuser => String::new(),
                                RoleLimit::Default => default.map(|l| format!("{} (server default)", l.describe())).unwrap_or_default(),
                            };
                            vec![text(&r.name), text(if r.superuser { "yes" } else { "" }), text(if r.login { "yes" } else { "" }), text(&eps.join(", ")), text(&limit)]
                        })
                        .collect(),
                )
            }
            "GRANTS" => text_rows(
                &["grantee", "privilege"],
                describe_privileges(&cat)
                    .into_iter()
                    .map(|l| {
                        let (g, p) = l.split_once(": ").unwrap_or(("", &l));
                        vec![text(g), text(p)]
                    })
                    .collect(),
            ),
            "ENDPOINTS" => text_rows(&["endpoint", "path"], cat.endpoints.iter().map(|(e, p)| vec![text(e), text(p.as_str())]).collect()),
            "STORAGE" => {
                self.require_superuser("see the stored form of a table")?;
                let name = arg.ok_or_else(|| DepError::sql("42601", "SHOW STORAGE table [LIMIT n]"))?;
                let limit = words.iter().filter_map(|w| w.parse::<usize>().ok()).next().unwrap_or(1000);
                self.stored_rows(&name, Some(limit)).await
            }
            _ => Err(DepError::sql("42601", format!("unrecognized SHOW {what}"))),
        }
    }

    // ------------------------------------------------------------------ roles and privileges

    fn set_role(&self, sql: &str) -> Result<Output> {
        let w = sqltext::leading_words(sql, 4);
        let target = if w.first().map(|x| x == "RESET").unwrap_or(false) {
            self.session_user.clone()
        } else {
            let toks: Vec<String> = sqltext::split(sql)?
                .first()
                .cloned()
                .unwrap_or_default()
                .split_whitespace()
                .map(|t| t.trim_matches(|c| c == '\'' || c == '"').to_lowercase())
                .collect();
            let name = toks.last().cloned().unwrap_or_default();
            if name == "none" || name == "default" {
                self.session_user.clone()
            } else {
                name
            }
        };
        let cat = self.cat();
        if !cat.roles.contains_key(&target) {
            return Err(DepError::sql("22023", format!("role \"{target}\" does not exist")));
        }
        if target != self.session_user && !cat.is_superuser(&self.session_user) {
            return Err(denied(format!("permission denied to set role \"{target}\"")));
        }
        if !cat.can_connect(&target, &self.endpoint) {
            return Err(denied(format!("role \"{target}\" is not allowed to connect through endpoint \"{}\"", self.endpoint)));
        }
        *self.role.lock().unwrap() = target;
        Ok(Output::Command("SET".into()))
    }

    fn role_ddl(&self, sql: &str) -> Result<Output> {
        let (verb, name, o) = sqltext::parse_role(sql)?;
        let role = self.role();
        self.cat_mut(|cat| {
            let superuser = cat.is_superuser(&role);
            let own_password_only = verb == "ALTER" && name == role && o.login.is_none() && o.superuser.is_none() && o.match_limit.is_none() && o.password.is_some();
            if !superuser && !own_password_only {
                return Err(denied(format!("permission denied to {} role", verb.to_lowercase())).with_hint("Only a superuser can do this."));
            }
            let hash = |p: &Option<String>| p.as_ref().map(|p| md5_hex(&format!("{p}{name}")));
            if verb == "CREATE" {
                if cat.roles.contains_key(&name) || name == PUBLIC {
                    return Err(DepError::sql("42710", format!("role \"{name}\" already exists")));
                }
                let password_md5 = o.password.as_ref().and_then(hash);
                cat.roles.insert(name.clone(), Role { name: name.clone(), login: o.login.unwrap_or(false), superuser: o.superuser.unwrap_or(false), password_md5, match_limit: o.match_limit.unwrap_or_default() });
                return Ok(Output::Command("CREATE ROLE".into()));
            }
            let r = cat.roles.get_mut(&name).ok_or_else(|| DepError::sql("42704", format!("role \"{name}\" does not exist")))?;
            if let Some(l) = o.login {
                r.login = l;
            }
            if let Some(su) = o.superuser {
                if name == ADMIN && !su {
                    return Err(DepError::sql("55006", "the admin role must stay a superuser"));
                }
                r.superuser = su;
            }
            if let Some(p) = &o.password {
                r.password_md5 = hash(p);
            }
            if let Some(l) = o.match_limit {
                r.match_limit = l;
            }
            Ok(Output::Command("ALTER ROLE".into()))
        })
    }

    fn grant(&self, sql: &str) -> Result<Output> {
        let g: GrantStmt = sqltext::parse_grant(sql)?;
        let role = self.role();
        let tag = if g.grant { "GRANT" } else { "REVOKE" };
        self.cat_mut(|cat| {
            let superuser = cat.is_superuser(&role);
            for r in &g.roles {
                if r != PUBLIC && !cat.roles.contains_key(r) {
                    return Err(DepError::sql("42704", format!("role \"{r}\" does not exist")));
                }
            }
            let wrong = |p: &Privilege, what: &str| DepError::sql("0LP01", format!("invalid privilege type {} for {what}", format!("{p:?}").split('(').next().unwrap_or("").to_uppercase()));
            match &g.object {
                Object::Table(tables) => {
                    for t in tables {
                        if !superuser {
                            return Err(denied(format!("permission denied for table {t}")).with_hint("Only a superuser grants table privileges."));
                        }
                        let meta = cat.tables.get(t).ok_or_else(|| DepError::sql("42P01", format!("relation \"{t}\" does not exist")))?.clone();
                        for p in &g.privileges {
                            if let Privilege::Select(Some(cols)) | Privilege::Update(Some(cols)) = p {
                                for c in cols {
                                    if meta.column(c).is_none() {
                                        return Err(DepError::sql("42703", format!("column \"{c}\" of relation \"{t}\" does not exist")));
                                    }
                                }
                            }
                        }
                        for r in &g.roles {
                            let e: &mut TablePriv = cat.table_privs.entry((r.clone(), t.clone())).or_default();
                            for p in &g.privileges {
                                match (p, g.grant) {
                                    (Privilege::Select(None), v) => {
                                        e.select_all = v;
                                        if !v {
                                            e.select_cols.clear();
                                        }
                                    }
                                    (Privilege::Select(Some(cols)), true) => e.select_cols.extend(cols.iter().cloned()),
                                    (Privilege::Select(Some(cols)), false) => cols.iter().for_each(|c| {
                                        e.select_cols.remove(c);
                                    }),
                                    (Privilege::Update(None), v) => {
                                        e.update_all = v;
                                        if !v {
                                            e.update_cols.clear();
                                        }
                                    }
                                    (Privilege::Update(Some(cols)), true) => e.update_cols.extend(cols.iter().cloned()),
                                    (Privilege::Update(Some(cols)), false) => cols.iter().for_each(|c| {
                                        e.update_cols.remove(c);
                                    }),
                                    (Privilege::Insert, v) => e.insert = v,
                                    (Privilege::Delete, v) => e.delete = v,
                                    (Privilege::All, v) => {
                                        *e = TablePriv { select_all: v, insert: v, update_all: v, delete: v, ..Default::default() };
                                    }
                                    (p, _) => return Err(wrong(p, "table")),
                                }
                            }
                        }
                    }
                }
                Object::Function(f) => {
                    let meta = cat.functions.get(f).ok_or_else(|| DepError::sql("42883", format!("function {f} does not exist")))?.clone();
                    if meta.owner != role && !superuser {
                        return Err(denied(format!("permission denied for function {f}")).with_hint("Only the owner of a function, or a superuser, grants EXECUTE on it."));
                    }
                    for p in &g.privileges {
                        if !matches!(p, Privilege::Execute | Privilege::All) {
                            return Err(wrong(p, "function"));
                        }
                    }
                    if g.grant && meta.is_declared() && g.roles.iter().any(|r| r == PUBLIC) {
                        return Err(DepError::dep(
                            "DEP_PUBLIC_EXECUTE",
                            format!("Function {f} is declared DISCLOSES or LINKS, and cannot be executable by PUBLIC."),
                            Some("Grant EXECUTE to the specific roles that need it."),
                        ));
                    }
                    let set = cat.fn_execute.entry(f.clone()).or_default();
                    for r in &g.roles {
                        if g.grant {
                            set.insert(r.clone());
                        } else {
                            set.remove(r);
                        }
                    }
                }
                Object::MatchDomain(d) => {
                    if !superuser {
                        return Err(denied(format!("permission denied for match domain {d}")).with_hint("Only a superuser grants DISCLOSE and LINK."));
                    }
                    let known = cat.tables.values().any(|t| t.columns.iter().any(|c| c.domain.as_deref() == Some(d.as_str())));
                    if !known {
                        return Err(DepError::sql("42704", format!("match domain \"{d}\" does not exist")));
                    }
                    if g.grant && g.roles.iter().any(|r| r == PUBLIC) {
                        return Err(DepError::dep("DEP_DECLARATION_PRIVILEGE", "DISCLOSE and LINK cannot be granted to PUBLIC.", Some("Grant them to the roles that declare functions.")));
                    }
                    for r in &g.roles {
                        let e = cat.domain_privs.entry((r.clone(), d.clone())).or_insert((false, false));
                        for p in &g.privileges {
                            match p {
                                Privilege::Disclose => e.0 = g.grant,
                                Privilege::Link => e.1 = g.grant,
                                Privilege::All => *e = (g.grant, g.grant),
                                p => return Err(wrong(p, "match domain")),
                            }
                        }
                    }
                }
                Object::Endpoint(ep) => {
                    if !superuser {
                        return Err(denied(format!("permission denied for endpoint {ep}")).with_hint("Only a superuser grants CONNECT."));
                    }
                    if !cat.endpoints.contains_key(ep) {
                        return Err(DepError::sql("42704", format!("endpoint \"{ep}\" does not exist")));
                    }
                    for p in &g.privileges {
                        if !matches!(p, Privilege::Connect | Privilege::All) {
                            return Err(wrong(p, "endpoint"));
                        }
                    }
                    let set = cat.endpoint_grants.entry(ep.clone()).or_default();
                    for r in &g.roles {
                        if g.grant {
                            set.insert(r.clone());
                        } else {
                            set.remove(r);
                        }
                    }
                }
            }
            Ok(Output::Command(tag.into()))
        })
    }

    // ------------------------------------------------------------------ DML

    fn table_meta(&self, name: &str) -> Result<TableMeta> {
        self.cat().tables.get(name).cloned().ok_or_else(|| DepError::sql("42P01", format!("relation \"{name}\" does not exist")))
    }

    async fn insert(&self, ins: ast::Insert) -> Result<Output> {
        let TableObject::TableName(tn) = &ins.table else {
            return Err(DepError::unsupported("INSERT target must be a table."));
        };
        if ins.on.is_some() || ins.returning.is_some() {
            return Err(DepError::unsupported("INSERT ... ON CONFLICT and RETURNING are not supported."));
        }
        let name = object_name(tn);
        let meta = self.table_meta(&name)?;
        let targets: Vec<String> =
            if ins.columns.is_empty() { meta.columns.iter().map(|c| c.name.clone()).collect() } else { ins.columns.iter().map(object_name).collect() };
        for t in &targets {
            if meta.column(t).is_none() {
                return Err(DepError::sql("42703", format!("column \"{t}\" of relation \"{name}\" does not exist")));
            }
        }
        let source = ins.source.ok_or_else(|| DepError::unsupported("INSERT needs VALUES or a SELECT."))?;
        let (info, w) = self.write_table(&name).await?;
        let plan = self.state()?.statement_to_plan(DFStatement::Statement(Box::new(SqlStatement::Query(source)))).await?;
        let events: Vec<Event>;
        let write_plan = {
            let cat = self.cat();
            let mut an = self.analyzer(&cat, None);
            let node = an.analyze(&plan)?;
            events = an.take_events();
            an.close_top(&node)?;
            if !cat.can_insert(&self.role(), &name) {
                return Err(denied(format!("permission denied for table {name}")));
            }
            self.check_uses(&cat, &privileges::collect(&plan, &cat))?;
            let mut notes = an.take_notes();
            for c in meta.columns.iter().filter(|c| c.domain.is_some() && targets.contains(&c.name)) {
                notes.push(format!("PROTECTED column {name}.{} (MATCH DOMAIN {}) is stored as a blind index and ciphertext.", c.name, c.domain.as_deref().unwrap_or("")));
            }
            self.add_notes(notes);
            let vis = Analyzer::visible(&node);
            if vis.len() != targets.len() {
                return Err(DepError::sql("42601", format!("INSERT has {} expressions but {} target columns", vis.len(), targets.len())));
            }
            let mut exprs = Vec::new();
            for c in &meta.columns {
                let src = targets.iter().position(|t| *t == c.name).map(|j| &vis[j]);
                let null = |dt: DataType| cast(lit(ScalarValue::Null), dt);
                match (&c.domain, src) {
                    (None, None) => exprs.push(null(c.data_type.clone()).alias(&c.name)),
                    (Some(_), None) => {
                        exprs.push(null(DataType::Utf8).alias(&c.name));
                        exprs.push(null(DataType::Utf8).alias(format!("{}{CT_SUFFIX}", c.name)));
                    }
                    (_, Some((q, n, t))) => {
                        let col = Expr::Column(Column::new(q.clone(), n));
                        match (t, &c.domain) {
                            (Taint::Disc { desc, .. }, _) => return Err(DepError::disclosed_use(desc, "a stored value")),
                            (Taint::Prot { desc, .. }, None) => {
                                return Err(DepError::dep(
                                    "DEP_PROTECTED_OPERATION",
                                    format!("Storing PROTECTED value {desc} in the ordinary column {name}.{} is forbidden.", c.name),
                                    Some("To release a value, use a disclosing function."),
                                ))
                            }
                            (Taint::Prot { domain, desc, has_ct, .. }, Some(d)) => {
                                if domain != d {
                                    return Err(DepError::dep(
                                        "DEP_MATCH_DOMAIN_MISMATCH",
                                        format!("{desc} (match domain {domain}) cannot be stored in {name}.{} (match domain {d}).", c.name),
                                        None,
                                    ));
                                }
                                if !has_ct {
                                    return Err(DepError::unsupported("Copy protected values directly from a table column."));
                                }
                                exprs.push(col.alias(&c.name));
                                exprs.push(Expr::Column(Column::new(q.clone(), format!("{n}{CT_SUFFIX}"))).alias(format!("{}{CT_SUFFIX}", c.name)));
                            }
                            (_, None) => exprs.push(cast(col, c.data_type.clone()).alias(&c.name)),
                            (_, Some(d)) => {
                                let v = cast(col, DataType::Utf8);
                                exprs.push(self.server.index_udf.call(vec![lit(d.clone()), v.clone()]).alias(&c.name));
                                exprs.push(self.server.encrypt_udf.call(vec![lit(d.clone()), v]).alias(format!("{}{CT_SUFFIX}", c.name)));
                            }
                        }
                    }
                }
            }
            LogicalPlan::Projection(Projection::try_new(exprs, Arc::new(node.plan))?)
        };
        self.admit_events(events)?;
        // INSERT ... SELECT from the same table reads all of its rows before writing any.
        let reads_target = plan_reads(&write_plan, &name);
        let mut stream = DataFrame::new(self.state()?, write_plan).execute_stream().await?;
        let mut held = Vec::new();
        let mut written: Vec<(u64, Vec<u8>)> = Vec::new();
        let mut n = 0u64;
        while let Some(b) = stream.next().await {
            let b = b?;
            if b.num_rows() == 0 {
                continue;
            }
            let b = RecordBatch::try_new(info.schema.clone(), b.columns().to_vec())?;
            if reads_target {
                held.push(b);
                continue;
            }
            n += self.insert_batch(&info, &w, &b, &mut written)?;
        }
        drop(stream);
        for b in held {
            n += self.insert_batch(&info, &w, &b, &mut written)?;
        }
        self.check_references_out(&info, &w, &written)?;
        Ok(Output::Command(format!("INSERT 0 {n}")))
    }

    fn insert_batch(&self, info: &Arc<TableInfo>, w: &Arc<Mutex<TxWriter>>, b: &RecordBatch, written: &mut Vec<(u64, Vec<u8>)>) -> Result<u64> {
        check_not_null(info, b)?;
        let rows = encode_rows(b)?;
        let n = rows.len() as u64;
        let mut wg = w.lock().unwrap();
        blocking(|| -> Result<()> {
            let start = wg.reserve(info, n)?;
            for (i, row) in rows.into_iter().enumerate() {
                let rid = start + i as u64;
                wg.put(info, rid, &row, None)?;
                if info.meta.columns.iter().any(|c| c.references.is_some()) {
                    written.push((rid, row));
                }
            }
            Ok(())
        })?;
        Ok(n)
    }

    /// Checks that each written row's references point at rows that exist (foreign keys, child side).
    fn check_references_out(&self, info: &TableInfo, w: &Arc<Mutex<TxWriter>>, rows: &[(u64, Vec<u8>)]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let cat = self.cat();
        let mut wg = w.lock().unwrap();
        for c in info.meta.columns.iter() {
            let Some((pt, pc)) = &c.references else { continue };
            let parent = cat.tables.get(pt).ok_or_else(|| DepError::sql("42P01", format!("relation \"{pt}\" does not exist")))?;
            let pinfo = TableInfo::new(parent);
            let pix = pinfo.indexes.iter().find(|i| i.meta.columns.len() == 1 && &i.meta.columns[0] == pc).cloned().ok_or_else(|| internal("referenced column without an index"))?;
            let cix = info.indexes.iter().find(|i| i.meta.columns.len() == 1 && i.meta.columns[0] == c.name).cloned().ok_or_else(|| internal("reference without an index"))?;
            for (_, row) in rows {
                let Some(key) = info.key(&cix, row)? else { continue };
                if !blocking(|| wg.has_key(&pinfo, &pix.meta.name, &key, None))? {
                    let shown = if c.domain.is_some() { format!("Key ({})", c.name) } else { format!("Key ({})=({})", c.name, info.show(cix.cols[0], row)) };
                    return Err(DepError::sql(
                        "23503",
                        format!("insert or update on table \"{}\" violates foreign key constraint \"{}_{}_fkey\": {shown} is not present in table \"{pt}\".", info.meta.name, info.meta.name, c.name),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Checks that no row still references a key that rows of this table no longer have
    /// (foreign keys, referenced side). `old` are the rows as they were before the statement.
    fn check_references_in(&self, info: &TableInfo, w: &Arc<Mutex<TxWriter>>, old: &[(u64, Vec<u8>)]) -> Result<()> {
        if old.is_empty() {
            return Ok(());
        }
        let cat = self.cat();
        let refs = cat.references_to(&info.meta.name);
        if refs.is_empty() {
            return Ok(());
        }
        let mut wg = w.lock().unwrap();
        for (child, ccol, pcol) in refs {
            let Some(pix) = info.indexes.iter().find(|i| i.meta.columns.len() == 1 && i.meta.columns[0] == pcol).cloned() else { continue };
            let cmeta = cat.tables.get(&child).ok_or_else(|| internal("referencing table"))?;
            let cinfo = TableInfo::new(cmeta);
            let Some(cix) = cinfo.indexes.iter().find(|i| i.meta.columns.len() == 1 && i.meta.columns[0] == ccol).cloned() else { continue };
            for (_, row) in old {
                let Some(key) = info.key(&pix, row)? else { continue };
                // Another row of this table may still have the key.
                if blocking(|| wg.has_key(info, &pix.meta.name, &key, None))? {
                    continue;
                }
                if blocking(|| wg.has_key(&cinfo, &cix.meta.name, &key, None))? {
                    let protected = info.meta.column(&pcol).map(|c| c.domain.is_some()).unwrap_or(false);
                    let shown = if protected { format!("Key ({pcol})") } else { format!("Key ({pcol})=({})", info.show(pix.cols[0], row)) };
                    return Err(DepError::sql(
                        "23503",
                        format!(
                            "update or delete on table \"{}\" violates foreign key constraint \"{child}_{ccol}_fkey\" on table \"{child}\": {shown} is still referenced from table \"{child}\".",
                            info.meta.name
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Plans `SELECT <exprs>, (<selection>) FROM <table>` through the DEP analyzer, with the
    /// row id column, and returns the rewritten expressions with their kinds, the condition,
    /// and the plan's input.
    async fn plan_row_exprs(&self, name: &str, table: &ast::TableWithJoins, exprs: &[String], selection: Option<&ast::Expr>) -> Result<(Vec<(Expr, Taint)>, Expr, Arc<LogicalPlan>)> {
        if !table.joins.is_empty() {
            return Err(DepError::unsupported("UPDATE and DELETE on one table only."));
        }
        let mut items: Vec<String> = exprs.iter().enumerate().map(|(i, e)| format!("{e} AS __dep_v{i}")).collect();
        items.push(format!("({}) AS __dep_hit", selection.map(|s| s.to_string()).unwrap_or_else(|| "true".into())));
        let sql = format!("SELECT {} FROM {}", items.join(", "), table);
        let mut stmts = DFParser::parse_sql_with_dialect(&sql, &PostgreSqlDialect {})?;
        let plan = self.state_with(Some(name))?.statement_to_plan(stmts.pop_front().unwrap()).await?;
        let (node, events) = {
            let cat = self.cat();
            let mut an = self.analyzer(&cat, None);
            let node = an.analyze(&plan);
            let node = node.and_then(|node| {
                an.close_top(&node)?;
                self.check_uses(&cat, &privileges::collect(&plan, &cat))?;
                Ok(node)
            });
            self.add_notes(an.take_notes());
            (node, an.take_events())
        };
        let node = node?;
        self.admit_events(events)?;
        let LogicalPlan::Projection(p) = &node.plan else {
            return Err(DepError::unsupported("UPDATE form not supported."));
        };
        let mut out = Vec::new();
        for (i, e) in p.expr.iter().enumerate().take(exprs.len()) {
            let e = match e {
                Expr::Alias(a) => (*a.expr).clone(),
                other => other.clone(),
            };
            out.push((e, node.taints[i].clone()));
        }
        let hit = match &p.expr[exprs.len()] {
            Expr::Alias(a) => (*a.expr).clone(),
            other => other.clone(),
        };
        if let Taint::Disc { desc, .. } | Taint::Prot { desc, .. } = &node.taints[exprs.len()] {
            return Err(DepError::protected_op("A bare column as a condition", desc));
        }
        Ok((out, hit, p.input.clone()))
    }

    async fn update(&self, up: ast::Update) -> Result<Output> {
        let name = match &up.table.relation {
            ast::TableFactor::Table { name, .. } => object_name(name),
            _ => return Err(DepError::unsupported("UPDATE target must be a table.")),
        };
        if up.from.is_some() || up.returning.is_some() {
            return Err(DepError::unsupported("UPDATE ... FROM and RETURNING are not supported."));
        }
        let meta = self.table_meta(&name)?;
        let (info, w) = self.write_table(&name).await?;
        let mut targets = Vec::new();
        let mut values = Vec::new();
        for a in &up.assignments {
            let AssignmentTarget::ColumnName(c) = &a.target else {
                return Err(DepError::unsupported("Tuple assignments are not supported."));
            };
            let c = object_name(c);
            if meta.column(&c).is_none() {
                return Err(DepError::sql("42703", format!("column \"{c}\" of relation \"{name}\" does not exist")));
            }
            targets.push(c);
            values.push(a.value.to_string());
        }
        let (vals, hit, input) = self.plan_row_exprs(&name, &up.table, &values, up.selection.as_ref()).await?;
        {
            let cat = self.cat();
            let role = self.role();
            if let Some(c) = targets.iter().find(|c| !cat.can_update(&role, &name, c)) {
                return Err(denied(format!("permission denied for column {c} of table {name}")).with_hint(format!("GRANT UPDATE ({c}) ON {name} TO {role} would allow it.")));
            }
        }
        let ischema = input.schema().clone();
        let colref = |n: &str| -> Result<Expr> {
            let (q, f) = ischema.qualified_field_with_unqualified_name(n)?;
            Ok(Expr::Column(Column::new(q.cloned(), f.name())))
        };
        let mut exprs = vec![colref(ROWID)?.alias(ROWID)];
        for c in &meta.columns {
            let assigned = targets.iter().position(|t| *t == c.name).map(|j| &vals[j]);
            let ct_name = format!("{}{CT_SUFFIX}", c.name);
            match (&c.domain, assigned) {
                (None, None) => exprs.push(colref(&c.name)?.alias(&c.name)),
                (Some(_), None) => {
                    exprs.push(colref(&c.name)?.alias(&c.name));
                    exprs.push(colref(&ct_name)?.alias(&ct_name));
                }
                (_, Some((_, Taint::Disc { desc, .. }))) => return Err(DepError::disclosed_use(desc, "a stored value")),
                (None, Some((_, Taint::Prot { desc, .. }))) => {
                    return Err(DepError::dep("DEP_PROTECTED_OPERATION", format!("Storing PROTECTED value {desc} in the ordinary column {name}.{} is forbidden.", c.name), None))
                }
                (None, Some((v, _))) => exprs.push(cast(v.clone(), c.data_type.clone()).alias(&c.name)),
                (Some(d), Some((v, Taint::Prot { domain, desc, .. }))) => {
                    if domain != d {
                        return Err(DepError::dep(
                            "DEP_MATCH_DOMAIN_MISMATCH",
                            format!("{desc} (match domain {domain}) cannot be stored in {name}.{} (match domain {d}).", c.name),
                            None,
                        ));
                    }
                    let Expr::Column(src) = v else { return Err(DepError::unsupported("Copy protected values directly from a column.")) };
                    let src_ct = Expr::Column(Column::new(src.relation.clone(), format!("{}{CT_SUFFIX}", src.name)));
                    exprs.push(v.clone().alias(&c.name));
                    exprs.push(src_ct.alias(&ct_name));
                }
                (Some(d), Some((v, _))) => {
                    let s = cast(v.clone(), DataType::Utf8);
                    exprs.push(self.server.index_udf.call(vec![lit(d.clone()), s.clone()]).alias(&c.name));
                    exprs.push(self.server.encrypt_udf.call(vec![lit(d.clone()), s]).alias(&ct_name));
                }
            }
        }
        // A filter keeps the rows where the condition is true (not NULL), and an equality on an
        // indexed column in it becomes an index lookup.
        let filtered = LogicalPlan::Filter(Filter::try_new(hit, input)?);
        let plan = LogicalPlan::Projection(Projection::try_new(exprs, Arc::new(filtered))?);
        // Every matching row is computed before any is written.
        let batches = DataFrame::new(self.state()?, plan).collect().await?;
        let mut old_rows = Vec::new();
        let mut new_rows = Vec::new();
        let mut n = 0usize;
        for b in &batches {
            if b.num_rows() == 0 {
                continue;
            }
            let ids = b.column(0).as_any().downcast_ref::<UInt64Array>().ok_or_else(|| internal("row id column"))?.clone();
            let phys = RecordBatch::try_new(info.schema.clone(), b.columns()[1..].to_vec())?;
            check_not_null(&info, &phys)?;
            let rows = encode_rows(&phys)?;
            let mut wg = w.lock().unwrap();
            blocking(|| -> Result<()> {
                for (i, row) in rows.into_iter().enumerate() {
                    let rid = ids.value(i);
                    let old = wg.get(&info, rid)?;
                    wg.put(&info, rid, &row, old.as_deref())?;
                    if let Some(o) = old {
                        old_rows.push((rid, o));
                    }
                    new_rows.push((rid, row));
                    n += 1;
                }
                Ok(())
            })?;
        }
        if info.meta.columns.iter().any(|c| c.references.is_some()) {
            self.check_references_out(&info, &w, &new_rows)?;
        }
        self.check_references_in(&info, &w, &old_rows)?;
        Ok(Output::Command(format!("UPDATE {n}")))
    }

    async fn delete(&self, del: ast::Delete) -> Result<Output> {
        let tables = match &del.from {
            FromTable::WithFromKeyword(t) | FromTable::WithoutKeyword(t) => t,
        };
        if tables.len() != 1 || del.using.is_some() || del.returning.is_some() {
            return Err(DepError::unsupported("DELETE from one table only."));
        }
        let name = match &tables[0].relation {
            ast::TableFactor::Table { name, .. } => object_name(name),
            _ => return Err(DepError::unsupported("DELETE target must be a table.")),
        };
        let (info, w) = self.write_table(&name).await?;
        let (_, hit, input) = self.plan_row_exprs(&name, &tables[0], &[], del.selection.as_ref()).await?;
        if !self.cat().can_delete(&self.role(), &name) {
            return Err(denied(format!("permission denied for table {name}")));
        }
        let ischema = input.schema().clone();
        let (q, f) = ischema.qualified_field_with_unqualified_name(ROWID)?;
        let rid = Expr::Column(Column::new(q.cloned(), f.name()));
        let filtered = LogicalPlan::Filter(Filter::try_new(hit, input)?);
        let plan = LogicalPlan::Projection(Projection::try_new(vec![rid.alias(ROWID)], Arc::new(filtered))?);
        let batches = DataFrame::new(self.state()?, plan).collect().await?;
        let mut old_rows = Vec::new();
        {
            let mut wg = w.lock().unwrap();
            blocking(|| -> Result<()> {
                for b in &batches {
                    let ids = b.column(0).as_any().downcast_ref::<UInt64Array>().ok_or_else(|| internal("row id column"))?;
                    for i in 0..ids.len() {
                        let rid = ids.value(i);
                        if let Some(old) = wg.get(&info, rid)? {
                            wg.delete(&info, rid, &old)?;
                            old_rows.push((rid, old));
                        }
                    }
                }
                Ok(())
            })?;
        }
        let n = old_rows.len();
        self.check_references_in(&info, &w, &old_rows)?;
        Ok(Output::Command(format!("DELETE {n}")))
    }
}

pub(crate) fn clone_meta(f: &FunctionMeta) -> FunctionMeta {
    FunctionMeta {
        name: f.name.clone(),
        params: f.params.clone(),
        returns: f.returns.clone(),
        discloses: f.discloses.clone(),
        links: f.links.clone(),
        body: f.body.clone(),
        comment: f.comment.clone(),
        plan: f.plan.clone(),
        disclosed: f.disclosed.clone(),
        masked_ok: f.masked_ok,
        owner: f.owner.clone(),
        reads: f.reads.clone(),
        scans: f.scans.clone(),
        calls: f.calls.clone(),
        source: f.source.clone(),
    }
}
