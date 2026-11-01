//! One way to talk to DEP, whether the server runs in this process or elsewhere.
//!
//! `Session::Local` holds a connection to a server in the same process (an in-memory
//! database, or a data directory opened directly). `Session::Remote` talks to a DEP Engine
//! server over the PostgreSQL wire protocol, in plain TCP or TLS. Both return the same text
//! results and DEP notes, so DEP CLI and Hoopeo Studio work the same way against either.

use std::sync::{Arc, Mutex};

use datafusion::arrow::array::Array;
use datafusion::arrow::util::display::array_value_to_string;

use crate::error::{sqlstate_of, DepError, Result};
use crate::server::{Connection, Output};
use crate::sqltext;

/// A result as text, the way a client sees it.
#[derive(Debug, Clone, serde::Serialize)]
pub enum Reply {
    Rows { columns: Vec<String>, rows: Vec<Vec<Option<String>>> },
    Command(String),
}

impl Reply {
    pub fn from_output(o: &Output) -> Reply {
        match o {
            Output::Command(t) => Reply::Command(t.clone()),
            Output::Rows { schema, batches } => {
                let columns = schema.fields().iter().map(|f| f.name().clone()).collect();
                let mut rows = Vec::new();
                for b in batches {
                    for r in 0..b.num_rows() {
                        rows.push((0..b.num_columns()).map(|c| if b.column(c).is_null(r) { None } else { array_value_to_string(b.column(c), r).ok() }).collect());
                    }
                }
                Reply::Rows { columns, rows }
            }
        }
    }
}

/// How a remote connection uses TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TlsMode {
    /// Plain TCP (a server on this machine, or inside a private network).
    #[default]
    Disable,
    /// Encrypted, without checking the server certificate (a self-signed server).
    Require,
    /// Encrypted, and the certificate must be valid for the host name (public roots, or the CA file).
    VerifyFull,
}

impl TlsMode {
    pub fn parse(s: &str) -> Option<TlsMode> {
        match s.to_ascii_lowercase().as_str() {
            "disable" | "off" | "" => Some(TlsMode::Disable),
            "require" | "prefer" => Some(TlsMode::Require),
            "verify-full" | "verify-ca" | "verify" => Some(TlsMode::VerifyFull),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            TlsMode::Disable => "disable",
            TlsMode::Require => "require",
            TlsMode::VerifyFull => "verify-full",
        }
    }
}

/// Where to connect.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RemoteTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub endpoint: String,
    #[serde(default)]
    pub tls: TlsMode,
    /// A PEM file of CA certificates to trust with `verify-full`, in addition to the public roots.
    #[serde(default)]
    pub ca_file: Option<String>,
}

pub struct Remote {
    client: tokio_postgres::Client,
    pub target: RemoteTarget,
    /// Notices the server sent during the last statement (the DEP notes arrive this way).
    notices: Arc<Mutex<Vec<String>>>,
    /// None outside a transaction; Some(failed) inside one.
    txn: Mutex<Option<bool>>,
}

pub enum Session {
    Local(Connection),
    Remote(Remote),
}

const DEP_CODES: [&str; 16] = [
    "DEP_REVEAL_OUTSIDE_DISCLOSURE",
    "DEP_DECLARATION_PRIVILEGE",
    "DEP_DECLARED_LANGUAGE",
    "DEP_DECLARED_CALL",
    "DEP_PUBLIC_EXECUTE",
    "DEP_REFERENCE_MISMATCH",
    "DEP_PROTECTED_OPERATION",
    "DEP_ENDPOINT_OPERATION",
    "DEP_MATCH_DOMAIN_MISMATCH",
    "DEP_DISCLOSED_VALUE_USE",
    "DEP_UNQUALIFIED_ACCESS",
    "DEP_PATH_IMMUTABLE",
    "DEP_UNANCHORED_MATCH",
    "DEP_UNSUPPORTED_FORM",
    "DEP_MATCH_RATE_LIMITED",
    "DEP_ERROR_WITHHELD",
];

fn from_pg(e: tokio_postgres::Error) -> DepError {
    match e.as_db_error() {
        Some(db) => {
            let code = db.detail().and_then(|d| DEP_CODES.iter().find(|c| **c == d).copied());
            DepError { code, sqlstate: code.map(|c| sqlstate_of(c).to_string()).unwrap_or_else(|| db.code().code().to_string()), message: db.message().to_string(), hint: db.hint().map(|h| h.to_string()) }
        }
        None => DepError::sql("08006", e.to_string()),
    }
}

/// The command tag psql would print, rebuilt from the statement and the row count
/// (the wire protocol client reports only the count).
fn tag_for(stmt: &str, n: u64) -> String {
    let w = sqltext::leading_words(stmt, 3);
    let w: Vec<&str> = w.iter().map(|s| s.as_str()).collect();
    match w.as_slice() {
        ["INSERT", ..] => format!("INSERT 0 {n}"),
        ["UPDATE", ..] | ["DELETE", ..] => format!("{} {n}", w[0]),
        ["CREATE", "OR", "REPLACE"] => "CREATE FUNCTION".into(),
        ["CREATE" | "DROP" | "ALTER", k, ..] => format!("{} {}", w[0], if *k == "USER" { "ROLE" } else { k }),
        [first, ..] => first.to_string(),
        [] => String::new(),
    }
}

impl Session {
    /// Connects to a dep-server over the PostgreSQL wire protocol.
    pub async fn connect_remote(target: RemoteTarget, password: &str) -> Result<Session> {
        let mut cfg = tokio_postgres::Config::new();
        cfg.host(&target.host).port(target.port).user(&target.user).password(password).dbname(&target.endpoint).application_name("dep");
        cfg.connect_timeout(std::time::Duration::from_secs(10));
        let notices = Arc::new(Mutex::new(Vec::new()));
        let client = match target.tls {
            TlsMode::Disable => {
                cfg.ssl_mode(tokio_postgres::config::SslMode::Disable);
                let (client, conn) = cfg.connect(tokio_postgres::NoTls).await.map_err(from_pg)?;
                spawn_connection(conn, notices.clone());
                client
            }
            mode => {
                cfg.ssl_mode(tokio_postgres::config::SslMode::Require);
                let tls = crate::tls::client_config(mode, target.ca_file.as_deref())?;
                let (client, conn) = cfg.connect(tokio_postgres_rustls::MakeRustlsConnect::new(tls)).await.map_err(from_pg)?;
                spawn_connection(conn, notices.clone());
                client
            }
        };
        Ok(Session::Remote(Remote { client, target, notices, txn: Mutex::new(None) }))
    }

    /// The DEP notes of the last statement: how the rules applied to it. Empty for plain SQL.
    pub async fn notes(&self) -> Vec<String> {
        match self {
            Session::Local(c) => c.notes(),
            Session::Remote(r) => r.notices.lock().unwrap().iter().filter_map(|n| n.strip_prefix("DEP: ").map(str::to_string)).collect(),
        }
    }

    /// The transaction state: None outside a transaction, Some(false) in one, Some(true) in a failed one.
    pub fn txn_status(&self) -> Option<bool> {
        match self {
            Session::Local(c) => c.in_txn().then(|| c.txn_failed()),
            Session::Remote(r) => *r.txn.lock().unwrap(),
        }
    }

    /// The TLS mode of the connection (always `disable` in process).
    pub fn tls(&self) -> TlsMode {
        match self {
            Session::Local(_) => TlsMode::Disable,
            Session::Remote(r) => r.target.tls,
        }
    }

    /// Runs a script, one statement at a time. Like psql, a failed statement does not stop the rest.
    pub async fn execute(&self, sql: &str) -> Vec<Result<Reply>> {
        match self {
            Session::Local(c) => c.execute(sql).await.into_iter().map(|r| r.map(|o| Reply::from_output(&o))).collect(),
            Session::Remote(r) => {
                let stmts = match sqltext::split(sql) {
                    Ok(s) => s,
                    Err(e) => return vec![Err(e)],
                };
                let mut out = Vec::new();
                for s in stmts {
                    out.push(r.run(&s).await);
                }
                out
            }
        }
    }

    pub async fn execute_one(&self, sql: &str) -> Result<Reply> {
        match self {
            Session::Local(c) => c.execute_one(sql).await.map(|o| Reply::from_output(&o)),
            Session::Remote(r) => r.run(sql).await,
        }
    }

    /// The role in effect now (it changes with SET ROLE).
    pub async fn role(&self) -> String {
        match self {
            Session::Local(c) => c.role(),
            Session::Remote(r) => match r.run("SHOW ROLE").await {
                Ok(Reply::Rows { rows, .. }) => rows.first().and_then(|x| x.first().cloned().flatten()).unwrap_or_else(|| r.target.user.clone()),
                _ => r.target.user.clone(),
            },
        }
    }

    pub fn endpoint(&self) -> String {
        match self {
            Session::Local(c) => c.endpoint.clone(),
            Session::Remote(r) => r.target.endpoint.clone(),
        }
    }

    /// OPERATIONAL or MASKED_READ.
    pub async fn path(&self) -> String {
        match self {
            Session::Local(c) => c.path.as_str().to_string(),
            Session::Remote(r) => match r.run("SHOW dep.path").await {
                Ok(Reply::Rows { rows, .. }) => rows.first().and_then(|x| x.first().cloned().flatten()).unwrap_or_default(),
                _ => String::new(),
            },
        }
    }

    /// A short description: where, as whom.
    pub fn location(&self) -> String {
        match self {
            Session::Local(c) => format!("local ({})", if c.server().is_persistent() { "data directory" } else { "in memory" }),
            Session::Remote(r) => format!("{}:{}", r.target.host, r.target.port),
        }
    }
}

/// Drives a wire connection, collecting the notices it receives.
fn spawn_connection<S, T>(mut conn: tokio_postgres::Connection<S, T>, notices: Arc<Mutex<Vec<String>>>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        use futures::StreamExt;
        let mut messages = futures::stream::poll_fn(move |cx| conn.poll_message(cx));
        while let Some(m) = messages.next().await {
            match m {
                Ok(tokio_postgres::AsyncMessage::Notice(n)) => notices.lock().unwrap().push(n.message().to_string()),
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
}

impl Remote {
    async fn run(&self, stmt: &str) -> Result<Reply> {
        use tokio_postgres::SimpleQueryMessage as M;
        let w = sqltext::leading_words(stmt, 3);
        if w.len() < 3 || w[0] != "SHOW" || w[1] != "DEP" || w[2] != "NOTES" {
            self.notices.lock().unwrap().clear();
        }
        let msgs = match self.client.simple_query(stmt).await {
            Ok(m) => m,
            Err(e) => {
                let mut t = self.txn.lock().unwrap();
                if t.is_some() {
                    *t = Some(true);
                }
                return Err(from_pg(e));
            }
        };
        match sqltext::leading_words(stmt, 1).first().map(|s| s.as_str()) {
            Some("BEGIN") | Some("START") => *self.txn.lock().unwrap() = Some(false),
            Some("COMMIT") | Some("END") | Some("ROLLBACK") | Some("ABORT") => *self.txn.lock().unwrap() = None,
            _ => {}
        }
        let mut columns: Vec<String> = Vec::new();
        let mut rows = Vec::new();
        let mut is_rows = false;
        let mut count = 0;
        for m in msgs {
            match m {
                M::RowDescription(cols) => {
                    is_rows = true;
                    columns = cols.iter().map(|c| c.name().to_string()).collect();
                }
                M::Row(r) => {
                    is_rows = true;
                    if columns.is_empty() {
                        columns = r.columns().iter().map(|c| c.name().to_string()).collect();
                    }
                    rows.push((0..r.len()).map(|i| r.get(i).map(|v| v.to_string())).collect());
                }
                M::CommandComplete(n) => count = n,
                _ => {}
            }
        }
        Ok(if is_rows { Reply::Rows { columns, rows } } else { Reply::Command(tag_for(stmt, count)) })
    }
}

/// Opens a local session: a data directory (created if new) or, with `None`, an in-memory database.
pub async fn open_local(data_dir: Option<&std::path::Path>, key_file: Option<&std::path::Path>, user: &str, endpoint: &str) -> Result<(Session, Arc<crate::DepServer>, bool)> {
    let (server, fresh) = match data_dir {
        Some(d) => crate::persist::open(d, key_file).await?,
        None => (crate::DepServer::new(), true),
    };
    let conn = server.connect_as(user, endpoint)?;
    Ok((Session::Local(conn), server, fresh))
}
