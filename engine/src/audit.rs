//! The audit log, match rate limits, and server metrics.
//!
//! Every use of a protected value is recorded: each match through a blind index (with the
//! number of values matched), each call of a function declared DISCLOSES or LINKS, each
//! statement the DEP rules refused, and each statement refused by a rate limit. A record
//! names the column or function and the match domain; it never contains a protected
//! value, and it does not keep the statement text, which could.
//!
//! A role may be limited to a number of protected matches per period
//! (`ALTER ROLE r MATCH LIMIT 100 PER MINUTE`), or the server may set a default limit for
//! every role that is not a superuser (`--match-limit 1000/minute`).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// One audited use of protected data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    #[serde(default)]
    pub seq: u64,
    /// UTC, RFC 3339.
    pub at: String,
    pub session_user: String,
    pub role: String,
    pub endpoint: String,
    pub path: String,
    /// match, disclose, link, denied, rate_limited, key_rotated.
    pub kind: String,
    /// The column (table.column) or function (name()).
    pub object: String,
    pub domain: Option<String>,
    /// Values matched, for a match.
    pub count: u32,
    /// The DEP error code, for a refused statement.
    pub detail: Option<String>,
}

/// An event noted by the analyzer, before it becomes a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub kind: &'static str,
    pub object: String,
    pub domain: Option<String>,
    pub count: u32,
}

pub fn now_utc() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// A match limit: at most `count` protected matches per `per_secs` seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchLimit {
    pub count: u32,
    pub per_secs: u64,
}

impl MatchLimit {
    /// Parses `1000/minute`, `50/second`, `10000/hour`, `500/day`.
    pub fn parse(s: &str) -> Option<MatchLimit> {
        let (n, unit) = s.trim().split_once('/')?;
        let count = n.trim().parse().ok()?;
        let per_secs = unit_secs(unit.trim())?;
        Some(MatchLimit { count, per_secs })
    }
    pub fn describe(&self) -> String {
        let unit = match self.per_secs {
            1 => "SECOND".to_string(),
            60 => "MINUTE".to_string(),
            3600 => "HOUR".to_string(),
            86400 => "DAY".to_string(),
            n => format!("{n} SECONDS"),
        };
        format!("{} PER {unit}", self.count)
    }
}

pub fn unit_secs(unit: &str) -> Option<u64> {
    match unit.to_ascii_lowercase().trim_end_matches('s') {
        "second" | "sec" => Some(1),
        "minute" | "min" => Some(60),
        "hour" => Some(3600),
        "day" => Some(86400),
        _ => None,
    }
}

/// The match limit set on a role: the server default, none, or its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RoleLimit {
    #[default]
    Default,
    Unlimited,
    Limit(MatchLimit),
}

/// Counts protected matches per role over a sliding window.
#[derive(Default)]
pub struct RateLimiter {
    windows: Mutex<HashMap<String, VecDeque<(Instant, u32)>>>,
}

impl RateLimiter {
    /// Admits `n` more matches for `role` under `limit`, or says how many were already used.
    pub fn admit(&self, role: &str, limit: MatchLimit, n: u32) -> Result<(), u32> {
        let mut w = self.windows.lock().unwrap();
        let q = w.entry(role.to_string()).or_default();
        let now = Instant::now();
        let window = Duration::from_secs(limit.per_secs);
        while q.front().map(|(t, _)| now.duration_since(*t) >= window).unwrap_or(false) {
            q.pop_front();
        }
        let used: u32 = q.iter().map(|(_, c)| *c).sum();
        if used.saturating_add(n) > limit.count {
            return Err(used);
        }
        q.push_back((now, n));
        Ok(())
    }
}

/// Counters for /metrics.
#[derive(Default)]
pub struct Metrics {
    pub statements: AtomicU64,
    pub errors: AtomicU64,
    pub protected_matches: AtomicU64,
    pub disclosures: AtomicU64,
    pub links: AtomicU64,
    pub rate_limited: AtomicU64,
    pub commits: AtomicU64,
    pub rollbacks: AtomicU64,
    pub wire_connections: AtomicU64,
    pub wire_active: AtomicU64,
    pub http_requests: AtomicU64,
    pub index_lookups: AtomicU64,
    pub full_scans: AtomicU64,
    pub rows_read: AtomicU64,
    pub idle_terminated: AtomicU64,
    pub error_codes: Mutex<BTreeMap<String, u64>>,
}

/// Values for the gauges of /metrics that the counters do not hold.
#[derive(Default)]
pub struct Gauges {
    pub tables: usize,
    pub rows: u64,
    pub persistent: bool,
    pub memory_limit: usize,
    pub spill_bytes: u64,
}

impl Metrics {
    pub fn inc(c: &AtomicU64, n: u64) {
        c.fetch_add(n, Ordering::Relaxed);
    }

    pub fn error(&self, code: &str) {
        Self::inc(&self.errors, 1);
        *self.error_codes.lock().unwrap().entry(code.to_string()).or_default() += 1;
    }

    /// The counters in the Prometheus text format.
    pub fn render(&self, g2: &Gauges) -> String {
        let g = |c: &AtomicU64| c.load(Ordering::Relaxed);
        let mut s = String::new();
        let mut metric = |name: &str, kind: &str, help: &str, v: u64| {
            s.push_str(&format!("# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {v}\n"));
        };
        metric("dep_statements_total", "counter", "Statements executed.", g(&self.statements));
        metric("dep_protected_matches_total", "counter", "Protected values matched through a blind index.", g(&self.protected_matches));
        metric("dep_disclosures_total", "counter", "Calls of functions declared DISCLOSES.", g(&self.disclosures));
        metric("dep_links_total", "counter", "Calls of functions declared LINKS.", g(&self.links));
        metric("dep_rate_limited_total", "counter", "Statements refused by a match rate limit.", g(&self.rate_limited));
        metric("dep_commits_total", "counter", "Transactions committed (each autocommit statement that writes counts).", g(&self.commits));
        metric("dep_rollbacks_total", "counter", "Transactions rolled back.", g(&self.rollbacks));
        metric("dep_wire_connections_total", "counter", "PostgreSQL wire connections accepted.", g(&self.wire_connections));
        metric("dep_wire_connections_active", "gauge", "PostgreSQL wire connections open now.", g(&self.wire_active));
        metric("dep_http_requests_total", "counter", "HTTP API requests.", g(&self.http_requests));
        metric("dep_index_lookups_total", "counter", "Table scans answered from an index.", g(&self.index_lookups));
        metric("dep_full_scans_total", "counter", "Table scans that read every row.", g(&self.full_scans));
        metric("dep_rows_read_total", "counter", "Rows read from tables.", g(&self.rows_read));
        metric("dep_idle_transactions_ended_total", "counter", "Transactions ended by the idle-in-transaction timeout.", g(&self.idle_terminated));
        metric("dep_tables", "gauge", "Tables.", g2.tables as u64);
        metric("dep_rows", "gauge", "Rows in all tables.", g2.rows);
        metric("dep_persistent", "gauge", "1 when the server keeps its data in a data directory.", g2.persistent as u64);
        metric("dep_query_memory_limit_bytes", "gauge", "Memory for query execution; beyond it, queries spill to encrypted files.", g2.memory_limit as u64);
        metric("dep_spilled_bytes_total", "counter", "Bytes written to encrypted spill files.", g2.spill_bytes);
        s.push_str("# HELP dep_errors_total Statements that failed, by SQLSTATE or DEP code.\n# TYPE dep_errors_total counter\n");
        for (code, n) in self.error_codes.lock().unwrap().iter() {
            s.push_str(&format!("dep_errors_total{{code=\"{code}\"}} {n}\n"));
        }
        s
    }
}
