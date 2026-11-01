//! The HTTP API of DEP Engine (Axum), over HTTPS when the server has a TLS certificate.
//!
//! ```text
//! GET  /v1/health   status and version (no authentication)
//! GET  /metrics     counters in the Prometheus text format (no authentication; no data)
//! POST /v1/query    runs statements: Basic authentication as a role, endpoint in the
//!                   DEP-Endpoint header or ?endpoint= (default operational)
//! ```
//!
//! A query request is JSON, `{"sql": "...", "params": [...]}`, or plain text SQL. The
//! statements run in order on one connection, so `BEGIN; ...; COMMIT;` works in one
//! request; they stop at the first error, and a transaction left open is rolled back.
//! `$1`, `$2`... in the SQL take the values of `params`. The response holds, for each
//! statement, its columns and rows (or its command tag), its DEP notes, and its error.
//! The same DEP rules, privileges, match limits and audit log apply as on the wire protocol.

use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use datafusion::arrow::array::ArrayRef;
use datafusion::arrow::datatypes::DataType;
use datafusion::arrow::util::display::array_value_to_string;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use crate::audit::{Gauges, Metrics};
use crate::{sqltext, DepError, DepServer, Output};

#[derive(Deserialize, Default)]
struct QueryBody {
    sql: String,
    #[serde(default)]
    params: Vec<Value>,
}

#[derive(Deserialize)]
struct EndpointParam {
    endpoint: Option<String>,
}

fn problem(status: StatusCode, e: &DepError) -> Response {
    (status, Json(json!({ "error": { "code": e.code, "sqlstate": e.sqlstate, "message": e.message, "hint": e.hint } }))).into_response()
}

fn unauthorized(msg: &str) -> Response {
    (StatusCode::UNAUTHORIZED, [(header::WWW_AUTHENTICATE, "Basic realm=\"DEP Engine\", charset=\"UTF-8\"")], Json(json!({ "error": { "code": null, "sqlstate": "28P01", "message": msg, "hint": null } })))
        .into_response()
}

fn cell(col: &ArrayRef, r: usize) -> Value {
    use datafusion::arrow::array::*;
    if col.is_null(r) {
        return Value::Null;
    }
    macro_rules! num {
        ($t:ty) => {
            json!(col.as_any().downcast_ref::<$t>().unwrap().value(r))
        };
    }
    match col.data_type() {
        DataType::Boolean => num!(BooleanArray),
        DataType::Int8 => num!(Int8Array),
        DataType::Int16 => num!(Int16Array),
        DataType::Int32 => num!(Int32Array),
        DataType::Int64 => num!(Int64Array),
        DataType::UInt8 => num!(UInt8Array),
        DataType::UInt16 => num!(UInt16Array),
        DataType::UInt32 => num!(UInt32Array),
        DataType::UInt64 => num!(UInt64Array),
        DataType::Float32 => num!(Float32Array),
        DataType::Float64 => num!(Float64Array),
        _ => Value::String(array_value_to_string(col, r).unwrap_or_default()),
    }
}

/// A JSON parameter as a SQL literal.
fn literal(v: &Value) -> String {
    match v {
        Value::Null => "NULL".into(),
        Value::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => sqltext::quote_literal(s),
        other => sqltext::quote_literal(&other.to_string()),
    }
}

async fn health(State(s): State<Arc<DepServer>>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "product": "DEP Engine",
        "version": env!("CARGO_PKG_VERSION"),
        "persistent": s.is_persistent(),
        "tables": s.catalog_now().tables.len(),
    }))
}

async fn metrics(State(s): State<Arc<DepServer>>) -> impl IntoResponse {
    let counts = s.row_counts().unwrap_or_default();
    let g = Gauges {
        tables: counts.len(),
        rows: counts.iter().map(|(_, n)| *n).sum(),
        persistent: s.is_persistent(),
        memory_limit: s.memory_limit,
        spill_bytes: s.spill.as_ref().map(|x| x.written.load(std::sync::atomic::Ordering::Relaxed)).unwrap_or(0),
    };
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], s.metrics.render(&g))
}

/// A parameter's value as text, for removing it from messages.
fn raw(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

async fn query(State(s): State<Arc<DepServer>>, Query(q): Query<EndpointParam>, headers: HeaderMap, body: Bytes) -> Response {
    Metrics::inc(&s.metrics.http_requests, 1);
    // Who: Basic authentication, checked against the role's stored password.
    let Some(auth) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Basic ")) else {
        return unauthorized("authentication required: use Basic authentication with a role and its password");
    };
    let decoded = base64::engine::general_purpose::STANDARD.decode(auth.trim()).ok().and_then(|b| String::from_utf8(b).ok());
    let Some((user, password)) = decoded.as_deref().and_then(|d| d.split_once(':')) else {
        return unauthorized("malformed Basic authentication");
    };
    let user = user.to_lowercase();
    if !s.check_password(&user, password) {
        return unauthorized(&format!("password authentication failed for role \"{user}\""));
    }
    let endpoint = headers.get("dep-endpoint").and_then(|v| v.to_str().ok()).map(str::to_string).or(q.endpoint).unwrap_or_else(|| "operational".into());
    let conn = match s.connect_as(&user, &endpoint) {
        Ok(c) => c,
        Err(e) => return problem(StatusCode::FORBIDDEN, &e),
    };
    // What: JSON {"sql", "params"} or plain SQL.
    let is_json = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(|v| v.starts_with("application/json")).unwrap_or(false);
    let req: QueryBody = if is_json {
        match serde_json::from_slice(&body) {
            Ok(b) => b,
            Err(e) => return problem(StatusCode::BAD_REQUEST, &DepError::sql("22P02", format!("invalid JSON body: {e}"))),
        }
    } else {
        QueryBody { sql: String::from_utf8_lossy(&body).to_string(), params: vec![] }
    };
    // The statements are split before the parameters are bound, so each result names its
    // statement as the client wrote it ($1, $2...), never with the values in it.
    let lits: Vec<String> = req.params.iter().map(literal).collect();
    let raws: Vec<String> = req.params.iter().map(raw).collect();
    let stmts = match sqltext::split(&req.sql) {
        Ok(s) => s,
        Err(e) => return problem(StatusCode::BAD_REQUEST, &e),
    };
    let mut results = Vec::new();
    let mut failed = false;
    for st in stmts {
        let t = Instant::now();
        let r = match sqltext::substitute_params(&st, &lits) {
            Ok(bound) => conn.execute_one(&bound).await,
            Err(e) => Err(e),
        };
        let elapsed_ms = (t.elapsed().as_secs_f64() * 1000.0 * 100.0).round() / 100.0;
        let notes = conn.notes();
        let mut item = json!({ "statement": st, "elapsed_ms": elapsed_ms, "notes": notes });
        match r {
            Ok(Output::Rows { schema, batches }) => {
                item["columns"] = json!(schema.fields().iter().map(|f| f.name().clone()).collect::<Vec<_>>());
                let mut rows = Vec::new();
                for b in &batches {
                    for r in 0..b.num_rows() {
                        rows.push(Value::Array((0..b.num_columns()).map(|c| cell(b.column(c), r)).collect()));
                    }
                }
                item["row_count"] = json!(rows.len());
                item["rows"] = Value::Array(rows);
            }
            Ok(Output::Command(tag)) => item["command"] = json!(tag),
            Err(e) => {
                let message = sqltext::scrub_values(&e.message, &raws);
                let hint = e.hint.as_ref().map(|h| sqltext::scrub_values(h, &raws));
                item["error"] = json!({ "code": e.code, "sqlstate": e.sqlstate, "message": message, "hint": hint });
                failed = true;
            }
        }
        results.push(item);
        if failed {
            break;
        }
    }
    let rolled_back = conn.in_txn();
    let status = if failed { StatusCode::UNPROCESSABLE_ENTITY } else { StatusCode::OK };
    (status, Json(json!({ "role": conn.role(), "endpoint": conn.endpoint, "path": conn.path.as_str(), "results": results, "open_transaction_rolled_back": rolled_back }))).into_response()
}

pub fn router(server: Arc<DepServer>) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/v1/query", post(query))
        .layer(axum::extract::DefaultBodyLimit::max(4 * 1024 * 1024))
        .with_state(server)
}

/// A TLS listener for axum: handshakes run on their own tasks, so a slow client does not hold up others.
struct TlsListener {
    rx: tokio::sync::mpsc::Receiver<(tokio_rustls::server::TlsStream<tokio::net::TcpStream>, std::net::SocketAddr)>,
    local: std::net::SocketAddr,
}

impl axum::serve::Listener for TlsListener {
    type Io = tokio_rustls::server::TlsStream<tokio::net::TcpStream>;
    type Addr = std::net::SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            if let Some(x) = self.rx.recv().await {
                return x;
            }
            // The accept task ended: no more connections will come.
            std::future::pending::<()>().await;
        }
    }
    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(self.local)
    }
}

/// Serves the HTTP API on a bound listener, over HTTPS with an acceptor.
pub async fn serve(server: Arc<DepServer>, listener: TcpListener, tls: Option<pgwire::tokio::TlsAcceptor>) {
    server.start_reaper();
    let app = router(server);
    match tls {
        None => {
            let _ = axum::serve(listener, app).await;
        }
        Some(acceptor) => {
            let local = listener.local_addr().expect("local address");
            let (tx, rx) = tokio::sync::mpsc::channel(64);
            tokio::spawn(async move {
                loop {
                    let Ok((tcp, addr)) = listener.accept().await else { continue };
                    let acceptor = acceptor.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let hs = tokio::time::timeout(std::time::Duration::from_secs(10), acceptor.accept(tcp)).await;
                        if let Ok(Ok(stream)) = hs {
                            let _ = tx.send((stream, addr)).await;
                        }
                    });
                }
            });
            let _ = axum::serve(TlsListener { rx, local }, app).await;
        }
    }
}
