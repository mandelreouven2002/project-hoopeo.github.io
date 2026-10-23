//! dep-engine: the DEP Engine server (Project Hoopeo).
//!
//! It speaks the PostgreSQL wire protocol. The user name is the role, and the database
//! name is the endpoint, which fixes the access path for the whole connection:
//!
//!   dep-cli -h 127.0.0.1 -p 5433 -U reception_app -d operational
//!   psql    -h 127.0.0.1 -p 5433 -U bi_dashboard  -d analytics
//!
//! With --data-dir the server keeps its state on disk; without it, in memory.
//! Every option can also come from the environment (DEP_LISTEN, DEP_DATA_DIR, DEP_KEY_FILE,
//! DEP_INIT, DEP_ADMIN_PASSWORD, DEP_INIT_ADMIN_PASSWORD_FILE, DEP_TLS_CERT, DEP_TLS_KEY, DEP_TLS_SELF_SIGNED,
//! DEP_TLS_REQUIRED, DEP_HTTP_LISTEN, DEP_MATCH_LIMIT, DEP_LOCK_TIMEOUT, DEP_MEMORY_LIMIT, DEP_CACHE_SIZE,
//! DEP_IDLE_IN_TRANSACTION_TIMEOUT, DEP_MAX_CONNECTIONS), which is how containers and services configure it.
//!
//! TLS: with --tls-cert and --tls-key (PEM), clients that ask for it get an encrypted
//! connection (sslmode=require or verify-full). --tls-self-signed creates a certificate in the
//! data directory (or the working directory) the first time.

use rand::RngCore;
use tokio::net::TcpListener;

use dep_engine::DepServer;

fn usage() -> ! {
    eprintln!("usage: dep-engine [--listen 127.0.0.1:5433] [--data-dir DIR] [--key-file FILE] [--init FILE]... [--admin-password PASSWORD]");
    eprintln!("                  [--tls-cert FILE --tls-key FILE | --tls-self-signed [--tls-host NAME]...] [--tls-required]");
    eprintln!("                  [--http-listen 127.0.0.1:8433] [--match-limit 1000/minute] [--lock-timeout 30]");
    eprintln!("                  [--memory-limit 4GB] [--cache-size 1GB] [--idle-in-transaction-timeout 300] [--max-connections 200]");
    eprintln!("  --data-dir   keep the state in DIR (created if new); without it the server keeps everything in memory");
    eprintln!("  --key-file   where the master key is kept (default DIR/master.key); keep it apart from the data");
    eprintln!("  --init       a DEP SQL script run as admin, only when the database is new");
    eprintln!("  --admin-password  sets the admin password; for a new database without one, a random password is printed");
    eprintln!("  --init-admin-password-file  for a new database only: the admin password is read from FILE");
    eprintln!("  --tls-cert, --tls-key  a PEM certificate chain and private key: clients may then connect with TLS");
    eprintln!("  --tls-self-signed      create a self-signed certificate (DIR/tls/server.crt) for localhost and each --tls-host");
    eprintln!("  --tls-required         refuse connections without TLS");
    eprintln!("  --http-listen          also serve the HTTP API (HTTPS with TLS): /v1/query, /v1/health, /metrics");
    eprintln!("  --match-limit          default limit on protected matches for roles that are not superusers, e.g. 1000/minute");
    eprintln!("  --lock-timeout         seconds a writer waits for the write lock (default 30)");
    eprintln!("  --memory-limit         memory for running queries; beyond it they spill to encrypted files (default: 1/4 of RAM)");
    eprintln!("  --cache-size           page cache of the data file (default: 1/8 of RAM, at most 4GB)");
    eprintln!("  --idle-in-transaction-timeout  seconds before a transaction left idle is rolled back (default 300; 0: never)");
    eprintln!("  --max-connections      client connections served at once (default 200; 0: no limit)");
    std::process::exit(2)
}

#[tokio::main]
async fn main() {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let mut listen = env("DEP_LISTEN").unwrap_or_else(|| "127.0.0.1:5433".to_string());
    let mut data_dir = env("DEP_DATA_DIR").map(std::path::PathBuf::from);
    let mut key_file = env("DEP_KEY_FILE").map(std::path::PathBuf::from);
    let mut inits: Vec<String> = env("DEP_INIT").map(|v| v.split(':').map(str::to_string).collect()).unwrap_or_default();
    let mut admin_password = env("DEP_ADMIN_PASSWORD");
    let mut init_password_file = env("DEP_INIT_ADMIN_PASSWORD_FILE").map(std::path::PathBuf::from);
    let mut tls_cert = env("DEP_TLS_CERT").map(std::path::PathBuf::from);
    let mut tls_key = env("DEP_TLS_KEY").map(std::path::PathBuf::from);
    let mut self_signed = env("DEP_TLS_SELF_SIGNED").map(|v| v != "0" && v != "false").unwrap_or(false);
    let mut tls_hosts: Vec<String> = env("DEP_TLS_HOSTS").map(|v| v.split(',').map(str::to_string).collect()).unwrap_or_default();
    let mut tls_required = env("DEP_TLS_REQUIRED").map(|v| v != "0" && v != "false").unwrap_or(false);
    let mut http_listen = env("DEP_HTTP_LISTEN");
    let mut match_limit = env("DEP_MATCH_LIMIT");
    let mut lock_timeout = env("DEP_LOCK_TIMEOUT");
    let mut memory_limit = env("DEP_MEMORY_LIMIT");
    let mut cache_size = env("DEP_CACHE_SIZE");
    let mut idle_timeout = env("DEP_IDLE_IN_TRANSACTION_TIMEOUT");
    let mut max_connections = env("DEP_MAX_CONNECTIONS");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--listen" => listen = args.next().unwrap_or_else(|| usage()),
            "--data-dir" => data_dir = Some(args.next().unwrap_or_else(|| usage()).into()),
            "--key-file" => key_file = Some(args.next().unwrap_or_else(|| usage()).into()),
            "--init" => inits.push(args.next().unwrap_or_else(|| usage())),
            "--admin-password" => admin_password = Some(args.next().unwrap_or_else(|| usage())),
            "--init-admin-password-file" => init_password_file = Some(args.next().unwrap_or_else(|| usage()).into()),
            "--tls-cert" => tls_cert = Some(args.next().unwrap_or_else(|| usage()).into()),
            "--tls-key" => tls_key = Some(args.next().unwrap_or_else(|| usage()).into()),
            "--tls-self-signed" => self_signed = true,
            "--tls-host" => tls_hosts.push(args.next().unwrap_or_else(|| usage())),
            "--tls-required" => tls_required = true,
            "--http-listen" => http_listen = Some(args.next().unwrap_or_else(|| usage())),
            "--match-limit" => match_limit = Some(args.next().unwrap_or_else(|| usage())),
            "--lock-timeout" => lock_timeout = Some(args.next().unwrap_or_else(|| usage())),
            "--memory-limit" => memory_limit = Some(args.next().unwrap_or_else(|| usage())),
            "--cache-size" => cache_size = Some(args.next().unwrap_or_else(|| usage())),
            "--idle-in-transaction-timeout" => idle_timeout = Some(args.next().unwrap_or_else(|| usage())),
            "--max-connections" => max_connections = Some(args.next().unwrap_or_else(|| usage())),
            _ => usage(),
        }
    }
    let size = |v: &Option<String>, what: &str| -> Option<usize> {
        v.as_ref().map(|s| {
            dep_engine::server::parse_size(s).unwrap_or_else(|| {
                eprintln!("{what}: expected a size, for example 512MB or 4GB");
                std::process::exit(2)
            })
        })
    };
    let options = dep_engine::ServerOptions { memory_limit: size(&memory_limit, "--memory-limit"), cache_size: size(&cache_size, "--cache-size"), ..Default::default() };
    let (server, fresh) = match &data_dir {
        Some(d) => match dep_engine::persist::open_with(d, key_file.as_deref(), &options).await {
            Ok(x) => x,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1)
            }
        },
        None => match DepServer::in_memory(&options) {
            Ok(s) => (s, true),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1)
            }
        },
    };
    let idle: u64 = match &idle_timeout {
        Some(t) => t.parse().unwrap_or_else(|_| {
            eprintln!("--idle-in-transaction-timeout: expected seconds");
            std::process::exit(2)
        }),
        None => 300,
    };
    *server.idle_timeout.write().unwrap() = (idle > 0).then(|| std::time::Duration::from_secs(idle));
    let max_connections: usize = match &max_connections {
        Some(t) => t.parse().unwrap_or_else(|_| {
            eprintln!("--max-connections: expected a number");
            std::process::exit(2)
        }),
        None => 200,
    };
    if let Some(l) = &match_limit {
        let Some(limit) = dep_engine::audit::MatchLimit::parse(l) else {
            eprintln!("--match-limit: expected a count and a period, for example 1000/minute");
            std::process::exit(2)
        };
        *server.default_match_limit.write().unwrap() = Some(limit);
    }
    if let Some(t) = &lock_timeout {
        let Ok(secs) = t.parse::<u64>() else {
            eprintln!("--lock-timeout: expected seconds");
            std::process::exit(2)
        };
        *server.lock_timeout.write().unwrap() = std::time::Duration::from_secs(secs);
    }
    let mut generated = None;
    if admin_password.is_none() && fresh {
        if let Some(f) = &init_password_file {
            match std::fs::read_to_string(f) {
                Ok(p) if !p.trim().is_empty() => admin_password = Some(p.trim().to_string()),
                Ok(_) => {}
                Err(e) => {
                    eprintln!("{}: {e}", f.display());
                    std::process::exit(1)
                }
            }
        }
    }
    match (&admin_password, fresh) {
        (Some(p), _) => server.set_password("admin", p).unwrap(),
        (None, true) => {
            let mut b = [0u8; 12];
            rand::thread_rng().fill_bytes(&mut b);
            let p = hex::encode(b);
            server.set_password("admin", &p).unwrap();
            generated = Some(p);
        }
        (None, false) => {}
    }
    let admin = server.connect("operational").unwrap();
    if fresh {
        for f in &inits {
            let sql = std::fs::read_to_string(f).unwrap_or_else(|e| panic!("{f}: {e}"));
            for r in admin.execute(&sql).await {
                if let Err(e) = r {
                    eprintln!("{f}: {e}");
                }
            }
        }
    }
    if let Err(e) = dep_engine::persist::flush(&server).await {
        eprintln!("{e}");
        std::process::exit(1)
    }
    if self_signed && tls_cert.is_none() {
        let dir = data_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(".")).join("tls");
        match dep_engine::tls::ensure_self_signed(&dir, &tls_hosts) {
            Ok((c, k)) => {
                tls_cert = Some(c);
                tls_key = Some(k);
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1)
            }
        }
    }
    let tls = match (&tls_cert, &tls_key) {
        (Some(c), Some(k)) => match dep_engine::tls::server_acceptor(c, k) {
            Ok(a) => Some(a),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1)
            }
        },
        (None, None) => None,
        _ => usage(),
    };
    if tls_required && tls.is_none() {
        eprintln!("--tls-required needs a certificate: --tls-cert and --tls-key, or --tls-self-signed");
        std::process::exit(2)
    }
    let listener = TcpListener::bind(&listen).await.unwrap_or_else(|e| panic!("{listen}: {e}"));
    let (host, port) = listen.rsplit_once(':').unwrap_or(("127.0.0.1", "5433"));
    eprintln!("DEP Engine {} listening on {listen}", env!("CARGO_PKG_VERSION"));
    match &data_dir {
        Some(d) => eprintln!("  data directory: {}{}", d.display(), if fresh { " (new)" } else { "" }),
        None => eprintln!("  data kept in memory: it is lost when the server stops (use --data-dir to keep it)"),
    }
    match &tls_cert {
        Some(c) if tls_required => eprintln!("  TLS: required (certificate {}); clients connect with --sslmode require or verify-full", c.display()),
        Some(c) => eprintln!("  TLS: on (certificate {}); clients may use --sslmode require or verify-full", c.display()),
        None => eprintln!("  TLS: off (use --tls-cert and --tls-key, or --tls-self-signed)"),
    }
    if let Some(l) = *server.default_match_limit.read().unwrap() {
        eprintln!("  match limit: {} for each role that is not a superuser", l.describe());
    }
    let mb = |b: usize| format!("{} MB", b / (1024 * 1024));
    eprintln!(
        "  query memory: {} (beyond it, encrypted spill files); data file cache: {}; idle transactions end after {}; at most {} connections",
        mb(server.memory_limit),
        mb(options.cache_bytes()),
        if idle > 0 { format!("{idle} s") } else { "never".into() },
        if max_connections > 0 { max_connections.to_string() } else { "unlimited".into() }
    );
    for n in server.startup_notes.lock().unwrap().iter() {
        eprintln!("  note: {n}");
    }
    if let Some(h) = &http_listen {
        let http = TcpListener::bind(h).await.unwrap_or_else(|e| panic!("{h}: {e}"));
        eprintln!("  HTTP API: {}://{h}/v1/query  (health: /v1/health, metrics: /metrics)", if tls.is_some() { "https" } else { "http" });
        tokio::spawn(dep_engine::http::serve(server.clone(), http, tls.clone()));
    }
    if let Some(p) = generated {
        eprintln!("  password for admin: {p}");
    }
    eprintln!("  dep-cli -h {host} -p {port} -U admin -d operational   (OPERATIONAL)");
    eprintln!("  dep-cli -h {host} -p {port} -U admin -d analytics     (MASKED_READ)");
    dep_engine::wire::serve_with(server, listener, dep_engine::wire::WireOptions { tls, tls_required, max_connections }).await;
}
