//! dep-server and the remote client, over a real TCP connection.

use dep_engine::client::{RemoteTarget, Reply, Session, TlsMode};
use std::process::{Child, Command, Stdio};

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn start(port: u16) -> Server {
    start_with(port, &[])
}

fn start_with(port: u16, extra: &[&str]) -> Server {
    let child = Command::new(env!("CARGO_BIN_EXE_dep-engine"))
        .args(["--listen", &format!("127.0.0.1:{port}"), "--init", concat!(env!("CARGO_MANIFEST_DIR"), "/../public/examples/clinic.sql"), "--admin-password", "secret"])
        .args(extra)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // Wait until the server accepts connections (a fixed sleep is too short on a slow machine).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Server(child)
}

fn target(port: u16, user: &str, endpoint: &str) -> RemoteTarget {
    RemoteTarget { host: "127.0.0.1".into(), port, user: user.into(), endpoint: endpoint.into(), ..Default::default() }
}

#[tokio::test]
async fn remote_sessions() {
    let port = 15433;
    let _s = start(port);
    let rec = Session::connect_remote(target(port, "reception_app", "operational"), "demo").await.unwrap();
    match rec.execute_one("SELECT full_name, national_id FROM patients WHERE national_id = '123456782'").await.unwrap() {
        Reply::Rows { columns, rows } => {
            assert_eq!(columns, ["full_name", "national_id"]);
            assert_eq!(rows, vec![vec![Some("Dana Cohen".to_string()), None]]);
        }
        other => panic!("{other:?}"),
    }
    // The DEP notes arrive as notices, and SHOW DEP NOTES repeats them.
    let notes = rec.notes().await;
    assert!(notes.iter().any(|n| n.contains("patients.national_id is matched by equality")), "{notes:?}");
    assert!(notes.iter().any(|n| n.contains("Output column national_id is PROTECTED")), "{notes:?}");
    match rec.execute_one("SHOW DEP NOTES").await.unwrap() {
        Reply::Rows { rows, .. } => assert_eq!(rows.len(), notes.len()),
        other => panic!("{other:?}"),
    }
    rec.execute_one("SELECT 1").await.unwrap();
    assert!(rec.notes().await.is_empty());
    // DEP error codes survive the wire.
    let e = rec.execute_one("SELECT full_name FROM patients WHERE national_id LIKE '1%'").await.unwrap_err();
    assert_eq!(e.code, Some("DEP_PROTECTED_OPERATION"));
    assert_eq!(e.sqlstate, "ZD101");
    assert!(matches!(rec.execute_one("UPDATE patients SET full_name = 'x' WHERE national_id = '123456782'").await.unwrap(), Reply::Command(t) if t == "UPDATE 1"));
    assert_eq!(rec.path().await, "OPERATIONAL");

    let bi = Session::connect_remote(target(port, "bi_dashboard", "analytics"), "demo").await.unwrap();
    assert_eq!(bi.path().await, "MASKED_READ");
    assert_eq!(bi.execute_one("SELECT health_fund FROM patients WHERE national_id = '1'").await.unwrap_err().code, Some("DEP_ENDPOINT_OPERATION"));
    assert!(matches!(bi.execute_one("SELECT * FROM diagnoses_per_fund()").await.unwrap(), Reply::Rows { rows, .. } if rows.len() == 4));

    // Wrong password, wrong endpoint.
    assert!(Session::connect_remote(target(port, "reception_app", "operational"), "nope").await.is_err());
    assert!(Session::connect_remote(target(port, "reception_app", "analytics"), "demo").await.is_err());

    // SET ROLE lasts for the connection.
    let admin = Session::connect_remote(target(port, "admin", "operational"), "secret").await.unwrap();
    admin.execute_one("SET ROLE reception_app").await.unwrap();
    assert_eq!(admin.role().await, "reception_app");
    assert_eq!(admin.execute_one("SELECT phone FROM patients").await.unwrap_err().sqlstate, "42501");
    admin.execute_one("RESET ROLE").await.unwrap();
    assert!(matches!(admin.execute_one("SHOW TABLES").await.unwrap(), Reply::Rows { rows, .. } if rows.len() == 3));
    assert!(matches!(admin.execute_one("SHOW STORAGE patients").await.unwrap(), Reply::Rows { rows, .. } if rows.len() == 6));
    assert_eq!(rec.execute_one("SHOW STORAGE patients").await.unwrap_err().sqlstate, "42501");
}

#[tokio::test]
async fn remote_tls() {
    let port = 15434;
    let dir = std::env::temp_dir().join(format!("dep-tls-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _s = start_with(port, &["--data-dir", dir.to_str().unwrap(), "--tls-self-signed"]);
    let mut t = target(port, "admin", "operational");
    // require: encrypted, certificate not checked.
    t.tls = TlsMode::Require;
    let s = Session::connect_remote(t.clone(), "secret").await.unwrap();
    assert!(matches!(s.execute_one("SHOW TABLES").await.unwrap(), Reply::Rows { rows, .. } if rows.len() == 3));
    // verify-full: fails against the public roots, passes with the server certificate as CA.
    t.tls = TlsMode::VerifyFull;
    assert!(Session::connect_remote(t.clone(), "secret").await.is_err());
    t.ca_file = Some(dir.join("tls/server.crt").display().to_string());
    let s = Session::connect_remote(t.clone(), "secret").await.unwrap();
    assert_eq!(s.path().await, "OPERATIONAL");
    // A plain connection still works (TLS is offered, not forced).
    let plain = Session::connect_remote(target(port, "admin", "operational"), "secret").await.unwrap();
    assert_eq!(plain.role().await, "admin");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Prepared statements with parameters (the extended protocol), typed columns, and
/// transactions over the wire, as a driver uses them.
#[tokio::test]
async fn extended_protocol_and_transactions() {
    let port = 15435;
    let _s = start(port);
    let connect = |user: &'static str, pw: &'static str| async move {
        let (c, conn) = tokio_postgres::Config::new().host("127.0.0.1").port(port).user(user).password(pw).dbname("operational").connect(tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(conn);
        c
    };
    let mut admin = connect("admin", "secret").await;
    let rec = connect("reception_app", "demo").await;

    // Parameters bind as values; protected outputs come back NULL.
    let rows = rec.query("SELECT full_name, national_id FROM patients WHERE national_id = $1", &[&"123456782"]).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, String>(0), "Dana Cohen");
    assert_eq!(rows[0].get::<_, Option<String>>(1), None);
    // A parameter is a value, never SQL.
    let rows = rec.query("SELECT full_name FROM patients WHERE national_id = $1", &[&"1' OR '1'='1"]).await.unwrap();
    assert!(rows.is_empty());
    // Typed parameters and columns.
    let rows = admin.query("SELECT patient_id, full_name FROM patients WHERE patient_id = $1", &[&2i64]).await.unwrap();
    assert_eq!(rows[0].get::<_, i64>(0), 2);
    let n: i64 = admin.query_one("SELECT count(*) FROM patients", &[]).await.unwrap().get(0);
    assert_eq!(n, 6);
    // DEP errors keep their SQLSTATE through prepared statements.
    let e = rec.query("SELECT full_name FROM patients WHERE national_id LIKE $1", &[&"1%"]).await.unwrap_err();
    assert_eq!(e.as_db_error().unwrap().code().code(), "ZD101");

    // A transaction with the driver's API: rolled back, then committed.
    {
        let tx = admin.transaction().await.unwrap();
        tx.execute("INSERT INTO patients (patient_id, national_id, full_name, health_fund) VALUES ($1, $2, $3, $4)", &[&80i64, &"222222226", &"Ori Ben", &"Clalit"]).await.unwrap();
        let seen = rec.query("SELECT full_name FROM patients WHERE national_id = $1", &[&"222222226"]).await.unwrap();
        assert!(seen.is_empty(), "uncommitted rows are not visible to others");
        tx.rollback().await.unwrap();
    }
    {
        let tx = admin.transaction().await.unwrap();
        tx.execute("INSERT INTO patients (patient_id, national_id, full_name, health_fund) VALUES ($1, $2, $3, $4)", &[&80i64, &"222222226", &"Ori Ben", &"Clalit"]).await.unwrap();
        tx.commit().await.unwrap();
    }
    let seen = rec.query("SELECT full_name FROM patients WHERE national_id = $1", &[&"222222226"]).await.unwrap();
    assert_eq!(seen.len(), 1);
    // A bound value never comes back in an error message.
    let e = rec.query("SELECT CAST(CAST($1 AS TEXT) AS BIGINT)", &[&"12345678x"]).await.unwrap_err();
    let msg = e.as_db_error().unwrap().message().to_string();
    assert!(!msg.contains("12345678x") && msg.contains("$1"), "{msg}");
    // SET ROLE lasts for the connection, across statements.
    admin.batch_execute("SET ROLE reception_app").await.unwrap();
    assert!(admin.query("SELECT phone FROM patients", &[]).await.is_err());
    admin.batch_execute("RESET ROLE").await.unwrap();
}

#[tokio::test]
async fn tls_required_and_http_api() {
    let port = 15436;
    let http = 15481;
    let dir = std::env::temp_dir().join(format!("dep-tlsreq-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _s = start_with(port, &["--data-dir", dir.to_str().unwrap(), "--tls-self-signed", "--tls-required", "--http-listen", &format!("127.0.0.1:{http}")]);
    // Without TLS: refused.
    assert!(Session::connect_remote(target(port, "admin", "operational"), "secret").await.is_err());
    let mut t = target(port, "admin", "operational");
    t.tls = TlsMode::Require;
    let s = Session::connect_remote(t, "secret").await.unwrap();
    assert_eq!(s.path().await, "OPERATIONAL");

    // The HTTP API, over HTTPS with the same certificate.
    let ca = dir.join("tls/server.crt");
    let tls = std::sync::Arc::new(dep_engine::tls::client_config(TlsMode::VerifyFull, Some(ca.to_str().unwrap())).unwrap());
    let request = |path: &str, auth: Option<&str>, body: Option<&str>| {
        let mut r = format!("{} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n", if body.is_some() { "POST" } else { "GET" });
        if let Some(a) = auth {
            use base64::Engine;
            r.push_str(&format!("Authorization: Basic {}\r\n", base64::engine::general_purpose::STANDARD.encode(a)));
        }
        if let Some(b) = body {
            r.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", b.len()));
        }
        r.push_str("\r\n");
        r.push_str(body.unwrap_or(""));
        r
    };
    let send = |req: String| {
        let tls = tls.clone();
        async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let tcp = tokio::net::TcpStream::connect(("127.0.0.1", http)).await.unwrap();
            let conn = tokio_rustls::TlsConnector::from(tls);
            let mut stream = conn.connect(rustls::pki_types::ServerName::try_from("localhost").unwrap(), tcp).await.unwrap();
            stream.write_all(req.as_bytes()).await.unwrap();
            let mut out = Vec::new();
            let _ = stream.read_to_end(&mut out).await;
            String::from_utf8_lossy(&out).to_string()
        }
    };
    let health = send(request("/v1/health", None, None)).await;
    assert!(health.starts_with("HTTP/1.1 200"), "{health}");
    assert!(health.contains("\"status\":\"ok\""));
    let denied = send(request("/v1/query", Some("reception_app:wrong"), Some(r#"{"sql":"SELECT 1"}"#))).await;
    assert!(denied.starts_with("HTTP/1.1 401"), "{denied}");
    let body = r#"{"sql":"SELECT full_name, national_id FROM patients WHERE national_id = $1","params":["123456782"]}"#;
    let ok = send(request("/v1/query?endpoint=operational", Some("reception_app:demo"), Some(body))).await;
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    assert!(ok.contains("Dana Cohen") && ok.contains("[\"Dana Cohen\",null]"), "{ok}");
    assert!(ok.contains("is matched by equality"), "the DEP notes come back: {ok}");
    let bad = send(request("/v1/query", Some("reception_app:demo"), Some(r#"{"sql":"SELECT full_name FROM patients WHERE national_id LIKE '1%'"}"#))).await;
    assert!(bad.starts_with("HTTP/1.1 422") && bad.contains("DEP_PROTECTED_OPERATION"), "{bad}");
    // The result names each statement as the client wrote it, and errors carry no bound value.
    let body = r#"{"sql":"SELECT full_name FROM patients WHERE national_id = $1; SELECT CAST($2 AS BIGINT)","params":["123456782", "98765432x"]}"#;
    let r = send(request("/v1/query", Some("reception_app:demo"), Some(body))).await;
    assert!(r.starts_with("HTTP/1.1 422"), "{r}");
    assert!(!r.contains("123456782") && !r.contains("98765432x"), "{r}");
    assert!(r.contains("national_id = $1") && r.contains("CAST($2 AS BIGINT)"), "{r}");
    let metrics = send(request("/metrics", None, None)).await;
    assert!(metrics.contains("dep_protected_matches_total"), "{metrics}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Many clients at once, as a server sees them: readers matching protected values through
/// indexes while writers commit transactions. Readers never wait for writers; writers take
/// turns; every committed row is there at the end, and no client sees a half-done transaction.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_clients() {
    let port = 15437;
    let dir = std::env::temp_dir().join(format!("dep-concurrent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _s = start_with(port, &["--data-dir", dir.to_str().unwrap()]);
    let connect = move |user: &'static str| async move {
        let (c, conn) = tokio_postgres::Config::new().host("127.0.0.1").port(port).user(user).password(if user == "admin" { "secret" } else { "demo" }).dbname("operational").connect(tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(conn);
        c
    };
    let writers = 4;
    let per_writer = 25;
    let mut tasks = Vec::new();
    for wi in 0..writers {
        tasks.push(tokio::spawn(async move {
            let mut c = connect("admin").await;
            for i in 0..per_writer {
                let id = 1000 + wi * per_writer + i;
                // Each transaction writes two rows that must appear together.
                loop {
                    let tx = c.transaction().await.unwrap();
                    let a = tx.execute("INSERT INTO patients (patient_id, national_id, full_name, health_fund) VALUES ($1, $2, 'Load', 'Clalit')", &[&(id as i64), &format!("9{id:08}")]).await;
                    let b = tx.execute("INSERT INTO appointments (appointment_id, patient_id, visit_type) VALUES ($1, $1, 'Load')", &[&(id as i64)]).await;
                    match (a, b) {
                        (Ok(_), Ok(_)) => {
                            tx.commit().await.unwrap();
                            break;
                        }
                        (a, b) => {
                            let e = a.err().or(b.err()).unwrap();
                            let code = e.as_db_error().map(|d| d.code().code().to_string()).unwrap_or_default();
                            // A write conflict or a lock wait is retried; anything else is a failure.
                            assert!(code == "40001" || code == "55P03", "{e}");
                            let _ = tx.rollback().await;
                        }
                    }
                }
            }
        }));
    }
    for _ in 0..8 {
        tasks.push(tokio::spawn(async move {
            let c = connect("reception_app").await;
            for i in 0..60 {
                let nid = ["123456782", "234567893", "345678904"][i % 3];
                let rows = c.query("SELECT full_name FROM patients WHERE national_id = $1", &[&nid]).await.unwrap();
                assert_eq!(rows.len(), 1);
                // Each Load patient has its appointment: a reader never sees one without the other.
                let n: i64 = c.query_one("SELECT count(*) FROM patients WHERE full_name = 'Load'", &[]).await.unwrap().get(0);
                let m: i64 = c.query_one("SELECT count(*) FROM appointments WHERE visit_type = 'Load'", &[]).await.unwrap().get(0);
                assert!(m >= n, "a reader saw {n} patients but {m} appointments");
            }
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    let c = connect("admin").await;
    let n: i64 = c.query_one("SELECT count(*) FROM patients WHERE full_name = 'Load'", &[]).await.unwrap().get(0);
    assert_eq!(n, (writers * per_writer) as i64);
    let joined: i64 = c.query_one("SELECT count(*) FROM patients p JOIN appointments a ON a.patient_id = p.patient_id WHERE a.visit_type = 'Load'", &[]).await.unwrap().get(0);
    assert_eq!(joined, n);
    let _ = std::fs::remove_dir_all(&dir);
}
