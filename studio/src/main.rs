//! Hoopeo Studio: the developer application for DEP SQL (Project Hoopeo), in the spirit of
//! PL/SQL Developer.
//!
//! The window is a web view (Tauri). Everything that touches a database runs here, in Rust,
//! through the DEP Engine client library, the same one DEP CLI uses: a local data directory
//! opened in process (optionally served on a TCP port so that other tools can connect), an
//! empty scratch database in memory, or a DEP Engine server elsewhere over the PostgreSQL wire
//! protocol, with or without TLS.
//!
//! The commands of the application live in the native menu bar. A menu choice is sent to the
//! web view as a "menu" event carrying the item id; the web view decides what to do with it.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dep_engine::client::{open_local, RemoteTarget, Reply, Session, TlsMode};
use dep_engine::DepServer;
use serde::{Deserialize, Serialize};
use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_dialog::DialogExt;

const NOTICE: &str = include_str!("../../NOTICE");
const EVALUATION_LICENSE: &str = include_str!("../../LICENSES/LicenseRef-Hoopeo-Evaluation.txt");
const THIRD_PARTY_NOTICES: &str = include_str!("../../THIRD-PARTY-NOTICES");
const SITE: &str = "https://lp.reuven.click";

/// A saved connection. Passwords are never written to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub name: String,
    /// "memory" (empty, in memory), "local" (a data directory), or "server".
    pub kind: String,
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_user")]
    pub user: String,
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    #[serde(default)]
    pub data_dir: String,
    /// For a server: "disable", "require" or "verify-full".
    #[serde(default)]
    pub tls: String,
    /// For a server with verify-full: a PEM file of CA certificates to trust.
    #[serde(default)]
    pub ca_file: String,
    /// For a local database: also serve it on this TCP port (127.0.0.1) while it is open.
    #[serde(default)]
    pub serve_port: Option<u16>,
}

fn default_port() -> u16 {
    5433
}
fn default_user() -> String {
    "admin".into()
}
fn default_endpoint() -> String {
    "operational".into()
}

struct Open {
    session: Arc<Session>,
    /// The in-process server behind a local connection.
    server: Option<Arc<DepServer>>,
    /// The key of that server in `AppState::shared`.
    key: Option<String>,
    profile: Profile,
}

struct Listener {
    addr: String,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Default)]
struct AppState {
    next: AtomicU32,
    open: Mutex<HashMap<u32, Open>>,
    /// In-process servers shared by connections to the same local database, so that two
    /// windows on one data directory see the same data.
    shared: Mutex<HashMap<String, Arc<DepServer>>>,
    /// Local databases served on a TCP port, by server key.
    listeners: Mutex<HashMap<String, Listener>>,
    started: Mutex<Option<Instant>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnInfo {
    id: u32,
    name: String,
    role: String,
    endpoint: String,
    path: String,
    location: String,
    kind: String,
    tls: String,
    /// "127.0.0.1:5433" when the local database is also served on a port.
    serving: Option<String>,
    persistent: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorInfo {
    code: Option<String>,
    sqlstate: String,
    message: String,
    hint: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatementResult {
    statement: String,
    elapsed_ms: f64,
    columns: Option<Vec<String>>,
    rows: Option<Vec<Vec<Option<String>>>>,
    tag: Option<String>,
    error: Option<ErrorInfo>,
    /// How the DEP rules applied to the statement. Empty for plain SQL.
    notes: Vec<String>,
}

fn err(e: dep_engine::DepError) -> String {
    serde_json::json!({ "code": e.code, "sqlstate": e.sqlstate, "message": e.message, "hint": e.hint }).to_string()
}

fn plain(msg: impl Into<String>) -> String {
    serde_json::json!({ "code": null, "sqlstate": "", "message": msg.into(), "hint": null }).to_string()
}

fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join(if cfg!(target_os = "macos") { "Hoopeo Studio" } else { "hoopeo-studio" })
}

fn profiles_path() -> PathBuf {
    config_dir().join("connections.json")
}

fn builtin_profiles() -> Vec<Profile> {
    let p = |name: &str, kind: &str, user: &str, endpoint: &str| Profile {
        name: name.into(),
        kind: kind.into(),
        host: String::new(),
        port: 5433,
        user: user.into(),
        endpoint: endpoint.into(),
        data_dir: String::new(),
        tls: String::new(),
        ca_file: String::new(),
        serve_port: None,
    };
    vec![Profile { host: "127.0.0.1".into(), ..p("DEP Engine on this computer", "server", "admin", "operational") }]
}

/// Saved connections. Connections of earlier versions to the demo clinic, which Studio no longer
/// has, are left out.
fn read_saved() -> Vec<Profile> {
    let saved: Vec<Profile> = std::fs::read_to_string(profiles_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    saved.into_iter().filter(|p| p.kind != "demo").collect()
}

fn write_saved(saved: &[Profile]) -> Result<(), String> {
    let path = profiles_path();
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| plain(e.to_string()))?;
    }
    std::fs::write(&path, serde_json::to_vec_pretty(saved).unwrap()).map_err(|e| plain(e.to_string()))
}

#[tauri::command]
fn list_profiles() -> Vec<Profile> {
    let mut all = builtin_profiles();
    for s in read_saved() {
        all.retain(|p| p.name != s.name);
        all.push(s);
    }
    all
}

#[tauri::command]
fn save_profile(profile: Profile) -> Result<(), String> {
    let mut saved = read_saved();
    saved.retain(|p| p.name != profile.name);
    saved.push(profile);
    write_saved(&saved)
}

#[tauri::command]
fn delete_profile(name: String) -> Result<(), String> {
    let mut saved = read_saved();
    saved.retain(|p| p.name != name);
    write_saved(&saved)
}

/// For automated checks: with HOOPEO_STUDIO_SELFTEST set, the window runs the script in the
/// variable as soon as it opens (see ui/app.js).
#[tauri::command]
fn self_test() -> Option<String> {
    std::env::var("HOOPEO_STUDIO_SELFTEST").ok().filter(|v| !v.is_empty())
}

#[tauri::command]
fn app_info() -> serde_json::Value {
    serde_json::json!({
        "product": "Hoopeo Studio",
        "version": env!("CARGO_PKG_VERSION"),
        "engine": format!("DEP Engine {} on Apache DataFusion", env!("CARGO_PKG_VERSION")),
        "copyright": "(c) Reuven Mandelovich",
        "site": SITE,
        "license": EVALUATION_LICENSE,
        "notice": NOTICE,
        "platform": std::env::consts::OS,
    })
}

/// The full texts of the third-party licenses; large, so the window asks for them only when opened.
#[tauri::command]
fn third_party_notices() -> &'static str {
    THIRD_PARTY_NOTICES
}

#[tauri::command]
fn default_data_dir() -> String {
    dep_engine::persist::default_data_dir().display().to_string()
}

/// Where a new local database goes by default: next to the default data directory.
#[tauri::command]
fn suggest_data_dir(name: String) -> String {
    let base = dep_engine::persist::default_data_dir();
    let parent = base.parent().map(|p| p.to_path_buf()).unwrap_or(base);
    let slug: String = name.trim().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' }).collect();
    parent.join("databases").join(if slug.is_empty() { "local".into() } else { slug }).display().to_string()
}

fn server_key(p: &Profile) -> String {
    match p.kind.as_str() {
        "local" => format!("local:{}", if p.data_dir.is_empty() { default_data_dir() } else { p.data_dir.clone() }),
        _ => format!("memory:{}", p.name),
    }
}

async fn info(state: &AppState, id: u32, s: &Session, p: &Profile, key: Option<&str>) -> ConnInfo {
    let serving = key.and_then(|k| state.listeners.lock().unwrap().get(k).map(|l| l.addr.clone()));
    let persistent = match s {
        Session::Local(c) => c.server().is_persistent(),
        Session::Remote(_) => true,
    };
    ConnInfo {
        id,
        name: p.name.clone(),
        role: s.role().await,
        endpoint: s.endpoint(),
        path: s.path().await,
        location: s.location(),
        kind: p.kind.clone(),
        tls: s.tls().as_str().to_string(),
        serving,
        persistent,
    }
}

/// Opens (or reuses) the in-process server of a local profile.
async fn local_server(state: &AppState, profile: &Profile, key: &str) -> Result<Arc<DepServer>, String> {
    if let Some(s) = state.shared.lock().unwrap().get(key).cloned() {
        return Ok(s);
    }
    let dir = (profile.kind == "local").then(|| PathBuf::from(key.trim_start_matches("local:")));
    let (_admin, server, _fresh) = open_local(dir.as_deref(), None, "admin", "operational").await.map_err(err)?;
    state.shared.lock().unwrap().insert(key.to_string(), server.clone());
    Ok(server)
}

/// Serves a local database on 127.0.0.1:port (or another bind address), so that DEP CLI, psql and
/// other tools can connect to it while Studio has it open. The admin password is set first, since
/// every wire connection authenticates.
async fn start_listener(state: &AppState, key: &str, server: Arc<DepServer>, bind: &str, port: u16, admin_password: Option<&str>) -> Result<String, String> {
    if let Some(l) = state.listeners.lock().unwrap().get(key) {
        return Ok(l.addr.clone());
    }
    if let Some(p) = admin_password.filter(|p| !p.is_empty()) {
        server.set_password("admin", p).map_err(err)?;
        dep_engine::persist::flush(&server).await.map_err(err)?;
    } else if server.password_md5("admin").is_none() {
        return Err(plain("Set a password for admin first: every connection over the network authenticates."));
    }
    let addr = format!("{bind}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|e| plain(format!("Cannot listen on {addr}: {e}")))?;
    let task = tokio::spawn(dep_engine::wire::serve(server, listener));
    state.listeners.lock().unwrap().insert(key.to_string(), Listener { addr: addr.clone(), task });
    Ok(addr)
}

#[tauri::command]
async fn connect(state: tauri::State<'_, AppState>, profile: Profile, password: Option<String>, admin_password: Option<String>) -> Result<ConnInfo, String> {
    let (session, server, key) = match profile.kind.as_str() {
        "server" => {
            let tls = TlsMode::parse(&profile.tls).ok_or_else(|| plain(format!("Unknown TLS mode \"{}\".", profile.tls)))?;
            let t = RemoteTarget {
                host: if profile.host.is_empty() { "127.0.0.1".into() } else { profile.host.clone() },
                port: profile.port,
                user: profile.user.clone(),
                endpoint: profile.endpoint.clone(),
                tls,
                ca_file: Some(profile.ca_file.clone()).filter(|c| !c.is_empty()),
            };
            (Session::connect_remote(t, password.as_deref().unwrap_or("")).await.map_err(err)?, None, None)
        }
        _ => {
            let key = server_key(&profile);
            let server = local_server(&state, &profile, &key).await?;
            if let Some(port) = profile.serve_port {
                start_listener(&state, &key, server.clone(), "127.0.0.1", port, admin_password.as_deref()).await?;
            }
            (Session::Local(server.connect_as(&profile.user, &profile.endpoint).map_err(err)?), Some(server), Some(key))
        }
    };
    let id = state.next.fetch_add(1, Ordering::SeqCst) + 1;
    let session = Arc::new(session);
    let ci = info(&state, id, &session, &profile, key.as_deref()).await;
    state.open.lock().unwrap().insert(id, Open { session, server, key, profile });
    Ok(ci)
}

/// Serves the local database of an open connection on a port.
#[tauri::command]
async fn serve_local(state: tauri::State<'_, AppState>, id: u32, bind: String, port: u16, admin_password: Option<String>) -> Result<ConnInfo, String> {
    let (server, key, session, profile) = {
        let open = state.open.lock().unwrap();
        let o = open.get(&id).ok_or_else(|| plain("Not connected."))?;
        let (Some(server), Some(key)) = (o.server.clone(), o.key.clone()) else {
            return Err(plain("Only a local database can be served from Studio. A DEP Engine server is already reachable on its own port."));
        };
        (server, key, o.session.clone(), o.profile.clone())
    };
    let bind = if bind.is_empty() { "127.0.0.1".to_string() } else { bind };
    start_listener(&state, &key, server, &bind, port, admin_password.as_deref()).await?;
    Ok(info(&state, id, &session, &profile, Some(&key)).await)
}

#[tauri::command]
async fn stop_serving(state: tauri::State<'_, AppState>, id: u32) -> Result<ConnInfo, String> {
    let (key, session, profile) = {
        let open = state.open.lock().unwrap();
        let o = open.get(&id).ok_or_else(|| plain("Not connected."))?;
        (o.key.clone(), o.session.clone(), o.profile.clone())
    };
    if let Some(k) = &key {
        if let Some(l) = state.listeners.lock().unwrap().remove(k) {
            l.task.abort();
        }
    }
    Ok(info(&state, id, &session, &profile, key.as_deref()).await)
}

#[tauri::command]
fn disconnect(state: tauri::State<'_, AppState>, id: u32) {
    let removed = state.open.lock().unwrap().remove(&id);
    // A local database stays open (and served) while any connection uses it.
    if let Some(Open { key: Some(key), .. }) = removed {
        let still_used = state.open.lock().unwrap().values().any(|o| o.key.as_deref() == Some(key.as_str()));
        if !still_used {
            if let Some(l) = state.listeners.lock().unwrap().remove(&key) {
                l.task.abort();
            }
            state.shared.lock().unwrap().remove(&key);
        }
    }
}

fn session(state: &tauri::State<'_, AppState>, id: u32) -> Result<(Arc<Session>, Profile, Option<String>), String> {
    state.open.lock().unwrap().get(&id).map(|o| (o.session.clone(), o.profile.clone(), o.key.clone())).ok_or_else(|| plain("Not connected."))
}

#[tauri::command]
async fn connection_info(state: tauri::State<'_, AppState>, id: u32) -> Result<ConnInfo, String> {
    let (s, p, k) = session(&state, id)?;
    Ok(info(&state, id, &s, &p, k.as_deref()).await)
}

/// Runs every statement in `sql`, one at a time, and reports each one's result, timing and DEP notes.
/// Like a SQL window in PL/SQL Developer, a failed statement stops the run.
#[tauri::command]
async fn run(state: tauri::State<'_, AppState>, id: u32, sql: String, continue_on_error: Option<bool>) -> Result<Vec<StatementResult>, String> {
    let (s, _, _) = session(&state, id)?;
    let stmts = dep_engine::sqltext::split(&sql).map_err(err)?;
    let mut out = Vec::new();
    for st in stmts {
        let t = Instant::now();
        let r = s.execute_one(&st).await;
        let elapsed_ms = t.elapsed().as_secs_f64() * 1000.0;
        let failed = r.is_err();
        let notes = if failed { vec![] } else { s.notes().await };
        out.push(match r {
            Ok(Reply::Rows { columns, rows }) => StatementResult { statement: st, elapsed_ms, columns: Some(columns), rows: Some(rows), tag: None, error: None, notes },
            Ok(Reply::Command(tag)) => StatementResult { statement: st, elapsed_ms, columns: None, rows: None, tag: Some(tag), error: None, notes },
            Err(e) => StatementResult {
                statement: st,
                elapsed_ms,
                columns: None,
                rows: None,
                tag: None,
                error: Some(ErrorInfo { code: e.code.map(str::to_string), sqlstate: e.sqlstate, message: e.message, hint: e.hint }),
                notes,
            },
        });
        if failed && !continue_on_error.unwrap_or(false) {
            break;
        }
    }
    Ok(out)
}

/// The object browser: everything the SHOW commands report, in one call.
#[tauri::command]
async fn browse(state: tauri::State<'_, AppState>, id: u32) -> Result<serde_json::Value, String> {
    let (s, _, _) = session(&state, id)?;
    async fn rows(s: &Session, sql: &str) -> serde_json::Value {
        match s.execute_one(sql).await {
            Ok(Reply::Rows { columns, rows }) => serde_json::Value::Array(
                rows.into_iter().map(|r| serde_json::Value::Object(columns.iter().cloned().zip(r.into_iter().map(|v| v.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null))).collect())).collect(),
            ),
            _ => serde_json::Value::Array(vec![]),
        }
    }
    let tables = rows(&s, "SHOW TABLES").await;
    let mut with_columns = Vec::new();
    if let serde_json::Value::Array(ts) = &tables {
        for t in ts {
            let name = t.get("table").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let cols = rows(&s, &format!("SHOW COLUMNS FROM {name}")).await;
            with_columns.push(serde_json::json!({ "name": name, "columns": cols, "rows": t.get("rows").cloned(), "temporary": t.get("temporary").and_then(|v| v.as_str()) == Some("yes") }));
        }
    }
    Ok(serde_json::json!({
        "tables": with_columns,
        "functions": rows(&s, "SHOW FUNCTIONS").await,
        "roles": rows(&s, "SHOW ROLES").await,
        "grants": rows(&s, "SHOW GRANTS").await,
        "endpoints": rows(&s, "SHOW ENDPOINTS").await,
    }))
}

// ---------------------------------------------------------------------- files

#[tauri::command]
fn read_text_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| plain(format!("{path}: {e}")))
}

#[tauri::command]
fn write_text_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| plain(format!("{path}: {e}")))
}

/// Shows an open dialog for SQL files; returns the path, or nothing if cancelled.
#[tauri::command]
async fn pick_open_file(app: AppHandle) -> Option<String> {
    let d = app.dialog().file().add_filter("SQL", &["sql", "txt"]);
    tokio::task::spawn_blocking(move || d.blocking_pick_file()).await.ok().flatten().map(|p| p.to_string())
}

#[tauri::command]
async fn pick_save_file(app: AppHandle, suggested: Option<String>) -> Option<String> {
    let mut d = app.dialog().file().add_filter("SQL", &["sql"]);
    if let Some(name) = suggested {
        d = d.set_file_name(name);
    }
    tokio::task::spawn_blocking(move || d.blocking_save_file()).await.ok().flatten().map(|p| p.to_string())
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    let d = app.dialog().file();
    tokio::task::spawn_blocking(move || d.blocking_pick_folder()).await.ok().flatten().map(|p| p.to_string())
}

#[tauri::command]
async fn pick_any_file(app: AppHandle) -> Option<String> {
    let d = app.dialog().file();
    tokio::task::spawn_blocking(move || d.blocking_pick_file()).await.ok().flatten().map(|p| p.to_string())
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

/// Called by the main window once it is ready: closes the splash screen and shows the window,
/// after the splash has been up for a moment.
#[tauri::command]
async fn app_ready(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let started = state.started.lock().unwrap().unwrap_or_else(Instant::now);
    let min = Duration::from_millis(if std::env::var("HOOPEO_STUDIO_SELFTEST").is_ok() { 300 } else { 1400 });
    if let Some(rest) = min.checked_sub(started.elapsed()) {
        tokio::time::sleep(rest).await;
    }
    reveal(&app);
    Ok(())
}

fn reveal(app: &AppHandle) {
    // For screenshots of the splash screen only.
    if std::env::var("HOOPEO_STUDIO_KEEP_SPLASH").is_ok() {
        return;
    }
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
    }
    if let Some(splash) = app.get_webview_window("splash") {
        let _ = splash.close();
    }
}

// ---------------------------------------------------------------------- menu

fn item(app: &AppHandle, id: &str, text: &str, accel: Option<&str>) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(app, id, text, true, accel)
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let sep = || PredefinedMenuItem::separator(app);
    let mac = cfg!(target_os = "macos");

    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &item(app, "file.new", "New SQL Window", Some("CmdOrCtrl+N"))?,
            &item(app, "file.open", "Open...", Some("CmdOrCtrl+O"))?,
            &sep()?,
            &item(app, "file.save", "Save", Some("CmdOrCtrl+S"))?,
            &item(app, "file.saveAs", "Save As...", Some("CmdOrCtrl+Shift+S"))?,
            &sep()?,
            &item(app, "file.close", "Close SQL Window", Some("CmdOrCtrl+W"))?,
            &sep()?,
            &item(app, "file.exit", if mac { "Quit Hoopeo Studio" } else { "Exit" }, Some(if mac { "CmdOrCtrl+Q" } else { "Alt+F4" }))?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &item(app, "edit.undo", "Undo", Some("CmdOrCtrl+Z"))?,
            &item(app, "edit.redo", "Redo", Some("CmdOrCtrl+Shift+Z"))?,
            &sep()?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
            &sep()?,
            &item(app, "edit.comment", "Comment / Uncomment", Some("CmdOrCtrl+/"))?,
            &item(app, "edit.upper", "Uppercase Selection", Some("CmdOrCtrl+Shift+U"))?,
            &item(app, "edit.lower", "Lowercase Selection", Some("CmdOrCtrl+Shift+L"))?,
        ],
    )?;
    let view = Submenu::with_items(
        app,
        "View",
        true,
        &[
            &item(app, "view.browser", "Object Browser", Some("CmdOrCtrl+B"))?,
            &item(app, "view.output", "Output", Some("CmdOrCtrl+Shift+O"))?,
            &sep()?,
            &item(app, "view.refresh", "Refresh Objects", Some("CmdOrCtrl+R"))?,
            &sep()?,
            &item(app, "view.zoomIn", "Larger Text", Some("CmdOrCtrl+="))?,
            &item(app, "view.zoomOut", "Smaller Text", Some("CmdOrCtrl+-"))?,
            &item(app, "view.zoomReset", "Actual Size", Some("CmdOrCtrl+0"))?,
        ],
    )?;
    let session = Submenu::with_items(
        app,
        "Session",
        true,
        &[
            &item(app, "session.connect", "Connect to DEP Server...", Some("CmdOrCtrl+Shift+C"))?,
            &item(app, "session.openLocal", "Open Local Database...", None)?,
            &item(app, "session.newLocal", "New Local Database...", None)?,
            &sep()?,
            &item(app, "session.serve", "Serve on Port...", None)?,
            &item(app, "session.stopServe", "Stop Serving", None)?,
            &sep()?,
            &item(app, "session.begin", "Begin Transaction", None)?,
            &item(app, "session.commit", "Commit", Some("F10"))?,
            &item(app, "session.rollback", "Rollback", Some("Shift+F10"))?,
            &sep()?,
            &item(app, "session.info", "Connection Information", None)?,
            &item(app, "session.disconnect", "Disconnect", None)?,
        ],
    )?;
    let query = Submenu::with_items(
        app,
        "Query",
        true,
        &[
            &item(app, "query.execute", "Execute", Some("F8"))?,
            &item(app, "query.executeCurrent", "Execute Current Statement", Some("CmdOrCtrl+Enter"))?,
            &item(app, "query.explain", "Explain Plan", Some("F5"))?,
            &sep()?,
            &item(app, "query.notes", "DEP Notes of Last Statement", None)?,
            &item(app, "query.clearOutput", "Clear Output", None)?,
        ],
    )?;
    let tools = Submenu::with_items(
        app,
        "Tools",
        true,
        &[
            &item(app, "tools.matrix", "Privacy Matrix", Some("CmdOrCtrl+Shift+M"))?,
            &item(app, "tools.grants", "Grants", None)?,
            &item(app, "tools.roles", "Roles", None)?,
            &item(app, "tools.endpoints", "Endpoints", None)?,
            &item(app, "tools.audit", "Audit Log", None)?,
            &item(app, "tools.indexes", "Indexes", None)?,
            &sep()?,
            &item(app, "tools.storage", "Stored Form of Table...", None)?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &item(app, "window.next", "Next SQL Window", Some("CmdOrCtrl+Alt+Right"))?,
            &item(app, "window.prev", "Previous SQL Window", Some("CmdOrCtrl+Alt+Left"))?,
            &sep()?,
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &PredefinedMenuItem::fullscreen(app, None)?,
        ],
    )?;
    let help = Submenu::with_items(
        app,
        "Help",
        true,
        &[
            &item(app, "help.reference", "DEP SQL Reference", Some("F1"))?,
            &item(app, "help.errors", "DEP Error Codes", None)?,
            &sep()?,
            &item(app, "help.licenses", "Licenses and Notices", None)?,
            &item(app, "help.site", "lp.reuven.click", None)?,
            &sep()?,
            &item(app, "help.about", "About Hoopeo Studio", None)?,
        ],
    )?;

    let menu = Menu::new(app)?;
    if mac {
        let about = AboutMetadata {
            name: Some("Hoopeo Studio".into()),
            version: Some(env!("CARGO_PKG_VERSION").into()),
            copyright: Some("(c) Reuven Mandelovich, lp.reuven.click".into()),
            ..Default::default()
        };
        let app_menu = Submenu::with_items(
            app,
            "Hoopeo Studio",
            true,
            &[
                &PredefinedMenuItem::about(app, Some("About Hoopeo Studio"), Some(about))?,
                &sep()?,
                &PredefinedMenuItem::services(app, None)?,
                &sep()?,
                &PredefinedMenuItem::hide(app, None)?,
                &PredefinedMenuItem::hide_others(app, None)?,
                &PredefinedMenuItem::show_all(app, None)?,
                &sep()?,
                &PredefinedMenuItem::quit(app, None)?,
            ],
        )?;
        menu.append(&app_menu)?;
    }
    for m in [&file, &edit, &view, &session, &query, &tools, &window, &help] {
        menu.append(m)?;
    }
    Ok(menu)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            *app.state::<AppState>().started.lock().unwrap() = Some(Instant::now());
            let handle = app.handle().clone();
            let menu = build_menu(&handle)?;
            // On macOS the menu bar belongs to the app; elsewhere it belongs to the main window
            // (the splash screen has none).
            if cfg!(target_os = "macos") {
                app.set_menu(menu)?;
            } else if let Some(main) = app.get_webview_window("main") {
                main.set_menu(menu)?;
            }
            app.on_menu_event(|app, event| {
                let id = event.id().0.as_str();
                if id == "help.site" {
                    let _ = open_url(SITE);
                    return;
                }
                let _ = app.emit_to("main", "menu", id.to_string());
            });
            // If the window never reports ready (a broken web view), show it anyway.
            let h = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(10)).await;
                reveal(&h);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            self_test,
            app_info,
            third_party_notices,
            app_ready,
            list_profiles,
            save_profile,
            delete_profile,
            default_data_dir,
            suggest_data_dir,
            connect,
            serve_local,
            stop_serving,
            disconnect,
            connection_info,
            run,
            browse,
            read_text_file,
            write_text_file,
            pick_open_file,
            pick_save_file,
            pick_folder,
            pick_any_file,
            quit
        ])
        .run(tauri::generate_context!())
        .expect("error while running Hoopeo Studio");
}

/// Opens a web page in the default browser.
fn open_url(url: &str) -> std::io::Result<()> {
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(cmd).arg(url).spawn().map(|_| ())
}
