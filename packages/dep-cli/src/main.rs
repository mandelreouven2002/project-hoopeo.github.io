//! DEP CLI (dep-cli): the DEP SQL shell, in the style of psql. Part of Project Hoopeo.
//!
//! Local (the database runs inside the shell):
//!   dep-cli                                   the default data directory, as admin
//!   dep-cli --data-dir ~/clinic --user reception_app
//!   dep-cli --memory --init examples/clinic.sql
//!
//! Remote (a dep-server elsewhere, over the PostgreSQL wire protocol):
//!   dep-cli -h db.example.com -p 5433 -U reception_app -d operational
//!
//! A local shell logs in without a password, like a local trusted connection in
//! PostgreSQL: whoever can read the data directory and the key already holds the data.
//! A remote connection always asks for one (or reads DEP_PASSWORD / PGPASSWORD).

use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::sync::Arc;

use dep_engine::client::{open_local, RemoteTarget, Reply, Session, TlsMode};
use dep_engine::format::{render_reply, table};
use dep_engine::DepServer;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

const HELP: &str = r"Meta-commands:
  \c ENDPOINT [ROLE]  open a new connection through ENDPOINT, as ROLE (default: the current one)
  \conninfo           show where you are connected, as whom, and on which access path
  \dt                 list tables
  \d TABLE            describe a table
  \di [TABLE]         list indexes (of TABLE)
  \df                 list declared functions
  \du                 list roles and the endpoints they may connect through
  \dp                 list privileges
  \storage TABLE      show TABLE as the server stores it (superusers only)
  \matrix             show the privacy matrix: who may reach each protected column, and how
  \audit [N]          show the latest N audit records (superusers)
  \notes [on|off]     print the DEP notes after each statement (on by default)
  \i FILE             run a script, printing each statement before its result
  \echo TEXT          print TEXT
  \q                  quit
Statements end with a semicolon. BEGIN ... COMMIT runs statements as one transaction
(the prompt shows =*> inside one, and =!> when it failed and needs ROLLBACK, or ROLLBACK TO
a savepoint).";

struct Shell {
    session: Session,
    /// The in-process server, for a local shell.
    local: Option<Arc<DepServer>>,
    /// Print each statement before its result, as psql -a does. On for scripts.
    echo: bool,
    /// Print the DEP notes after each statement.
    notes: bool,
    role: String,
}

fn print_result(r: dep_engine::Result<Reply>) {
    match r {
        Ok(o) => println!("{}\n", render_reply(&o)),
        Err(e) => println!("{e}\n"),
    }
}

fn password_for(user: &str, host: &str) -> String {
    if let Ok(p) = std::env::var("DEP_PASSWORD").or_else(|_| std::env::var("PGPASSWORD")) {
        return p;
    }
    rpassword::prompt_password(format!("Password for {user}@{host}: ")).unwrap_or_default()
}

fn complete(buf: &str) -> bool {
    use dep_engine::datafusion::sql::sqlparser::tokenizer::Token;
    match dep_engine::sqltext::tokens(buf) {
        Ok(t) => t.iter().rev().find(|t| !matches!(t, Token::Whitespace(_) | Token::EOF)) == Some(&Token::SemiColon),
        Err(_) => false,
    }
}

impl Shell {
    /// Prints a result, then the DEP notes of the statement, the way psql prints NOTICE lines.
    async fn report(&self, r: dep_engine::Result<Reply>) {
        let notes = if self.notes { self.session.notes().await } else { vec![] };
        match r {
            Ok(o) => {
                for n in &notes {
                    println!("DEP:  {n}");
                }
                println!("{}\n", render_reply(&o));
            }
            Err(e) => println!("{e}\n"),
        }
    }

    fn prompt(&self) -> String {
        let mark = match self.session.txn_status() {
            None => "",
            Some(false) => "*",
            Some(true) => "!",
        };
        format!("{}@{}={mark}> ", self.role, self.session.endpoint())
    }

    async fn refresh_role(&mut self) {
        self.role = self.session.role().await;
    }

    async fn run_sql(&mut self, sql: &str) {
        if !self.echo {
            match dep_engine::sqltext::split(sql) {
                Ok(stmts) => {
                    for st in stmts {
                        let r = self.session.execute_one(&st).await;
                        self.report(r).await;
                    }
                }
                Err(e) => println!("{e}\n"),
            }
            self.refresh_role().await;
            return;
        }
        let stmts = match dep_engine::sqltext::split(sql) {
            Ok(s) => s,
            Err(e) => return println!("{e}\n"),
        };
        for st in stmts {
            let mut code = Vec::new();
            for line in st.lines() {
                if code.is_empty() && line.trim_start().starts_with("--") {
                    println!("{}", line.trim());
                } else if code.is_empty() && line.trim().is_empty() {
                    continue;
                } else {
                    code.push(line);
                }
            }
            if code.is_empty() {
                continue;
            }
            println!("{}{};", self.prompt(), code.join("\n"));
            let r = self.session.execute_one(&code.join("\n")).await;
            self.report(r).await;
            self.refresh_role().await;
        }
    }

    /// Runs a script that may mix SQL and meta-commands (one per line).
    async fn run_script(&mut self, text: &str) -> bool {
        let mut buf = String::new();
        for line in text.lines() {
            let only_comments = dep_engine::sqltext::is_whitespace_only(&buf);
            if line.trim_start().starts_with('\\') && (only_comments || complete(&buf)) {
                if only_comments {
                    if self.echo {
                        buf.lines().map(str::trim).filter(|l| !l.is_empty()).for_each(|l| println!("{l}"));
                    }
                } else {
                    let sql = std::mem::take(&mut buf);
                    self.run_sql(&sql).await;
                }
                buf.clear();
                if self.echo && !line.trim_start().starts_with("\\echo") {
                    println!("{}{}", self.prompt(), line.trim());
                }
                if !Box::pin(self.meta(line.trim())).await {
                    return false;
                }
                continue;
            }
            buf.push_str(line);
            buf.push('\n');
        }
        if !buf.trim().is_empty() {
            self.run_sql(&buf).await;
        }
        true
    }

    async fn show(&self, sql: &str, keep: &[&str]) {
        match self.session.execute_one(sql).await {
            Ok(Reply::Rows { columns, rows }) => {
                let idx: Vec<usize> = if keep.is_empty() { (0..columns.len()).collect() } else { keep.iter().filter_map(|k| columns.iter().position(|c| c == k)).collect() };
                let cols: Vec<String> = idx.iter().map(|&i| columns[i].clone()).collect();
                let rows: Vec<Vec<Option<String>>> = rows.iter().map(|r| idx.iter().map(|&i| r[i].clone()).collect()).collect();
                println!("{}\n", table(&cols, &rows));
            }
            other => print_result(other),
        }
    }

    async fn reconnect(&mut self, endpoint: &str, user: Option<&str>) {
        let new = match &self.session {
            Session::Local(c) => {
                let user = user.map(str::to_string).unwrap_or_else(|| c.session_user.clone());
                let server = self.local.clone().expect("local server");
                server.connect_as(&user, endpoint).map(Session::Local)
            }
            Session::Remote(r) => {
                let mut t = r.target.clone();
                t.endpoint = endpoint.to_string();
                if let Some(u) = user {
                    t.user = u.to_string();
                }
                let pw = password_for(&t.user, &t.host);
                Session::connect_remote(t, &pw).await
            }
        };
        match new {
            Ok(s) => {
                self.session = s;
                self.refresh_role().await;
                println!("You are now connected as \"{}\" through endpoint \"{}\" ({}).\n", self.role, self.session.endpoint(), self.session.path().await);
            }
            Err(e) => println!("{e}\n"),
        }
    }

    async fn meta(&mut self, line: &str) -> bool {
        let mut parts = line.split_whitespace();
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next();
        match cmd {
            "\\q" => return false,
            "\\?" | "\\h" => println!("{HELP}\n"),
            "\\notes" => {
                self.notes = !matches!(arg, Some("off") | Some("0") | Some("false"));
                println!("DEP notes are {}.\n", if self.notes { "on" } else { "off" });
            }
            "\\matrix" => self.show("SHOW PRIVACY MATRIX", &[]).await,
            "\\audit" => self.show(&format!("SHOW AUDIT {}", arg.unwrap_or("50")), &[]).await,
            "\\echo" => println!("{}", line.trim_start().trim_start_matches("\\echo").trim()),
            "\\c" | "\\connect" => match arg {
                Some(ep) => {
                    let user = parts.next().map(str::to_string);
                    self.reconnect(ep, user.as_deref()).await
                }
                None => println!("usage: \\c ENDPOINT [ROLE]\n"),
            },
            "\\conninfo" => println!("Role \"{}\", endpoint \"{}\", access path {}, server {}.\n", self.role, self.session.endpoint(), self.session.path().await, self.session.location()),
            "\\dt" => self.show("SHOW TABLES", &[]).await,
            "\\d" => match arg {
                Some(t) => self.show(&format!("SHOW COLUMNS FROM {t}"), &[]).await,
                None => self.show("SHOW TABLES", &[]).await,
            },
            "\\di" => match arg {
                Some(t) => self.show(&format!("SHOW INDEXES FROM {t}"), &[]).await,
                None => self.show("SHOW INDEXES", &[]).await,
            },
            "\\df" => self.show("SHOW FUNCTIONS", &["function", "discloses", "links", "owner", "masked_path", "executable_by"]).await,
            "\\du" => self.show("SHOW ROLES", &[]).await,
            "\\dp" => self.show("SHOW GRANTS", &[]).await,
            "\\storage" => match arg {
                Some(t) => self.show(&format!("SHOW STORAGE {t}"), &[]).await,
                None => println!("usage: \\storage TABLE\n"),
            },
            "\\i" => match arg.map(std::fs::read_to_string) {
                Some(Ok(sql)) => {
                    let was = self.echo;
                    self.echo = true;
                    let go_on = Box::pin(self.run_script(&sql)).await;
                    self.echo = was;
                    if !go_on {
                        return false;
                    }
                }
                Some(Err(e)) => println!("{e}\n"),
                None => println!("usage: \\i FILE\n"),
            },
            _ => println!("invalid command {cmd}. Try \\? for help.\n"),
        }
        true
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: dep-cli [-h HOST [-p PORT]] [-U ROLE] [-d ENDPOINT] [--data-dir DIR | --memory] [--key-file FILE]\n               [--init FILE]... [-c SQL] [-f FILE]\n               [--sslmode disable|require|verify-full] [--sslrootcert CA.pem]\n\n  without -h:  a local database in --data-dir (default: {}), or in memory with --memory\n  with -h:     a DEP Engine server, over the PostgreSQL wire protocol (password from DEP_PASSWORD, PGPASSWORD, or a prompt)",
        dep_engine::persist::default_data_dir().display()
    );
    std::process::exit(2)
}

#[tokio::main]
async fn main() {
    let mut endpoint = "operational".to_string();
    let mut user = None;
    let mut host = None;
    let mut port: u16 = 5433;
    let mut data_dir: Option<PathBuf> = None;
    let mut memory = false;
    let mut key_file: Option<PathBuf> = None;
    let mut inits = Vec::new();
    let mut command = None;
    let mut file = None;
    let mut sslmode = std::env::var("PGSSLMODE").unwrap_or_else(|_| "disable".into());
    let mut ca_file: Option<String> = std::env::var("PGSSLROOTCERT").ok();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "-h" | "--host" => host = Some(val()),
            "-p" | "--port" => port = val().parse().unwrap_or_else(|_| usage()),
            "-U" | "--user" => user = Some(val()),
            "-d" | "--endpoint" | "-e" => endpoint = val(),
            "--data-dir" => data_dir = Some(PathBuf::from(val())),
            "--memory" => memory = true,
            "--sslmode" => sslmode = val(),
            "--sslrootcert" => ca_file = Some(val()),
            "--key-file" => key_file = Some(PathBuf::from(val())),
            "--init" => inits.push(val()),
            "-c" => command = Some(val()),
            "-f" => file = Some(val()),
            "--help" => usage(),
            _ => usage(),
        }
    }

    let (session, local, banner) = if let Some(h) = host {
        let user = user.unwrap_or_else(|| "admin".into());
        let pw = password_for(&user, &h);
        let tls = TlsMode::parse(&sslmode).unwrap_or_else(|| usage());
        let target = RemoteTarget { host: h.clone(), port, user, endpoint: endpoint.clone(), tls, ca_file: ca_file.clone() };
        match Session::connect_remote(target, &pw).await {
            Ok(s) => (s, None, format!("server {h}:{port}{}", if tls == TlsMode::Disable { String::new() } else { format!(", TLS {}", tls.as_str()) })),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2)
            }
        }
    } else {
        let dir = if memory { None } else { Some(data_dir.unwrap_or_else(dep_engine::persist::default_data_dir)) };
        let user = user.unwrap_or_else(|| "admin".into());
        // Open as admin first, so a new database can be initialized before connecting as the chosen role.
        let (admin, server, fresh) = match open_local(dir.as_deref(), key_file.as_deref(), "admin", "operational").await {
            Ok(x) => x,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2)
            }
        };
        if fresh {
            for f in &inits {
                let sql = std::fs::read_to_string(f).unwrap_or_else(|e| panic!("{f}: {e}"));
                for r in admin.execute(&sql).await {
                    if let Err(e) = r {
                        eprintln!("{f}: {e}");
                    }
                }
            }
        } else if !inits.is_empty() {
            eprintln!("The data directory already exists, so --init was not run.");
        }
        let session = match server.connect_as(&user, &endpoint) {
            Ok(c) => Session::Local(c),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2)
            }
        };
        let where_ = match &dir {
            Some(d) => format!("local data directory {}", d.display()),
            None => "a local in-memory database".to_string(),
        };
        (session, Some(server), where_)
    };

    let role = session.role().await;
    let mut shell = Shell { session, local, echo: false, notes: true, role };

    if let Some(c) = command {
        shell.run_script(&c).await;
        return;
    }
    if let Some(f) = file {
        let sql = std::fs::read_to_string(&f).unwrap_or_else(|e| panic!("{f}: {e}"));
        shell.echo = true;
        shell.run_script(&sql).await;
        return;
    }
    if !std::io::stdin().is_terminal() {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).ok();
        shell.run_script(&s).await;
        return;
    }

    println!("DEP CLI {} (Project Hoopeo). DEP Engine on Apache DataFusion.", env!("CARGO_PKG_VERSION"));
    println!("Connected as \"{}\" through endpoint \"{}\" ({}), {banner}.", shell.role, shell.session.endpoint(), shell.session.path().await);
    println!("Type \\? for help, \\q to quit.\n");
    let mut rl = DefaultEditor::new().expect("terminal");
    let history = dirs::data_dir().map(|d| d.join(if cfg!(target_os = "macos") { "Project Hoopeo" } else { "project-hoopeo" }).join("dep-cli-history.txt"));
    if let Some(h) = &history {
        let _ = rl.load_history(h);
    }
    let mut buf = String::new();
    loop {
        let prompt = if buf.is_empty() { shell.prompt() } else { shell.prompt().replace("=> ", "-> ") };
        match rl.readline(&prompt) {
            Ok(line) => {
                if buf.is_empty() && line.trim_start().starts_with('\\') {
                    let _ = rl.add_history_entry(line.as_str());
                    if !shell.meta(line.trim()).await {
                        break;
                    }
                    continue;
                }
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(&line);
                if complete(&buf) {
                    let _ = rl.add_history_entry(buf.as_str());
                    let sql = std::mem::take(&mut buf);
                    shell.run_sql(&sql).await;
                }
            }
            Err(ReadlineError::Interrupted) => buf.clear(),
            Err(_) => break,
        }
    }
    if let Some(h) = &history {
        if let Some(p) = h.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let _ = rl.save_history(h);
    }
}
