//! Storage on disk (0.5): constraints, index lookups, savepoints, transactional schema
//! changes, large tables with bounded memory, the 0.4 migration, and idle transactions.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use datafusion::arrow::array::Array;
use datafusion::arrow::util::display::array_value_to_string;
use dep_engine::{persist, DepServer, Output, ServerOptions};

const CLINIC: &str = include_str!("../../examples/clinic.sql");

fn tempdir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("hoopeo-st-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn rows(o: &Output) -> Vec<Vec<Option<String>>> {
    let Output::Rows { batches, .. } = o else { panic!("expected rows, got {o:?}") };
    let mut v = Vec::new();
    for b in batches {
        for r in 0..b.num_rows() {
            v.push((0..b.num_columns()).map(|c| if b.column(c).is_null(r) { None } else { Some(array_value_to_string(b.column(c), r).unwrap()) }).collect());
        }
    }
    v
}

async fn clinic(server: &Arc<DepServer>) {
    let admin = server.connect("operational").unwrap();
    for (i, r) in admin.execute(CLINIC).await.into_iter().enumerate() {
        if let Err(e) = r {
            panic!("setup statement {i}: {e}");
        }
    }
}

async fn one(c: &dep_engine::Connection, sql: &str) -> Vec<Vec<Option<String>>> {
    rows(&c.execute_one(sql).await.unwrap_or_else(|e| panic!("{sql}: {e}")))
}

#[tokio::test]
async fn constraints_are_enforced_without_revealing_protected_values() {
    let s = DepServer::new();
    clinic(&s).await;
    let a = s.connect("operational").unwrap();
    // Primary key.
    let e = a.execute_one("INSERT INTO patients (patient_id, full_name) VALUES (1, 'Again')").await.unwrap_err();
    assert_eq!(e.sqlstate, "23505");
    assert!(e.message.contains("patients_pkey") && e.message.contains("(patient_id)=(1)"), "{}", e.message);
    // A key column cannot be NULL.
    assert_eq!(a.execute_one("INSERT INTO patients (full_name) VALUES ('No id')").await.unwrap_err().sqlstate, "23502");
    // A unique PROTECTED column: enforced through blind index entries; the value is never in the message.
    a.execute_one("CREATE TABLE cards (card_id BIGINT PRIMARY KEY, holder TEXT PROTECTED MATCH DOMAIN person_id UNIQUE NOT NULL, label TEXT)").await.unwrap();
    a.execute_one("INSERT INTO cards VALUES (1, '123456782', 'first')").await.unwrap();
    let e = a.execute_one("INSERT INTO cards VALUES (2, '123456782', 'second')").await.unwrap_err();
    assert_eq!(e.sqlstate, "23505");
    assert!(!e.message.contains("123456782"), "{}", e.message);
    assert!(e.message.contains("Key (holder)"), "{}", e.message);
    assert_eq!(a.execute_one("INSERT INTO cards (card_id, label) VALUES (3, 'x')").await.unwrap_err().sqlstate, "23502");
    // References: an ordinary key and a protected one.
    let e = a.execute_one("INSERT INTO appointments (appointment_id, patient_id) VALUES (900, 99)").await.unwrap_err();
    assert_eq!(e.sqlstate, "23503");
    assert!(e.message.contains("(patient_id)=(99)"), "{}", e.message);
    let e = a.execute_one("INSERT INTO medical_records (record_id, patient_national_id, diagnosis) VALUES (900, '999999999', 'x')").await.unwrap_err();
    assert_eq!(e.sqlstate, "23503");
    assert!(!e.message.contains("999999999"), "{}", e.message);
    let e = a.execute_one("DELETE FROM patients WHERE patient_id = 1").await.unwrap_err();
    assert_eq!(e.sqlstate, "23503");
    let e = a.execute_one("UPDATE patients SET national_id = '111111118' WHERE patient_id = 2").await.unwrap_err();
    assert_eq!(e.sqlstate, "23503", "medical_records still reference the old national_id");
    assert!(!e.message.contains("234567893"), "{}", e.message);
    // A patient no one references can go.
    a.execute_one("INSERT INTO patients (patient_id, national_id, full_name) VALUES (50, '505050505', 'Temp')").await.unwrap();
    a.execute_one("DELETE FROM patients WHERE patient_id = 50").await.unwrap();
    // A failed statement changes nothing (here: the third row of three violates the key).
    let e = a.execute_one("INSERT INTO cards VALUES (10, '1', 'a'), (11, '2', 'b'), (12, '1', 'c')").await.unwrap_err();
    assert_eq!(e.sqlstate, "23505");
    assert_eq!(one(&a, "SELECT card_id FROM cards").await.len(), 1);
    // Composite keys, and CREATE UNIQUE INDEX on data with duplicates.
    a.execute_one("CREATE TABLE shifts (day DATE, slot INT, nurse TEXT, PRIMARY KEY (day, slot))").await.unwrap();
    a.execute_one("INSERT INTO shifts VALUES ('2026-10-22', 1, 'A'), ('2026-10-22', 2, 'A')").await.unwrap();
    assert_eq!(a.execute_one("INSERT INTO shifts VALUES ('2026-10-22', 1, 'B')").await.unwrap_err().sqlstate, "23505");
    assert_eq!(a.execute_one("CREATE UNIQUE INDEX shifts_nurse ON shifts (nurse)").await.unwrap_err().sqlstate, "23505");
    a.execute_one("CREATE INDEX shifts_nurse ON shifts (nurse)").await.unwrap();
    assert_eq!(a.execute_one("DROP INDEX shifts_pkey").await.unwrap_err().sqlstate, "2BP01");
    a.execute_one("DROP INDEX shifts_nurse").await.unwrap();
    // A referenced table cannot be dropped first.
    assert_eq!(a.execute_one("DROP TABLE patients").await.unwrap_err().sqlstate, "2BP01");
    let idx = one(&a, "SHOW INDEXES FROM patients").await;
    assert!(idx.iter().any(|r| r[1].as_deref() == Some("patients_pkey") && r[3].as_deref() == Some("primary key")));
    assert!(idx.iter().any(|r| r[2].as_deref() == Some("national_id") && r[6].as_deref() == Some("blind index entries")));
}

#[tokio::test]
async fn equality_on_indexed_columns_reads_only_matching_rows() {
    let s = DepServer::new();
    clinic(&s).await;
    let rec = s.connect_as("reception_app", "operational").unwrap();
    let before = s.metrics.index_lookups.load(Ordering::Relaxed);
    let full = s.metrics.full_scans.load(Ordering::Relaxed);
    let o = one(&rec, "SELECT full_name FROM patients WHERE national_id = '345678904'").await;
    assert_eq!(o, vec![vec![Some("Noa Mizrahi".into())]]);
    one(&rec, "SELECT full_name FROM patients WHERE patient_id IN (2, 4)").await;
    assert_eq!(s.metrics.index_lookups.load(Ordering::Relaxed), before + 2);
    assert_eq!(s.metrics.full_scans.load(Ordering::Relaxed), full);
    one(&rec, "SELECT full_name FROM patients WHERE health_fund = 'Clalit'").await;
    assert_eq!(s.metrics.full_scans.load(Ordering::Relaxed), full + 1);
    let admin = s.connect("operational").unwrap();
    let plan: Vec<String> = one(&admin, "EXPLAIN SELECT full_name FROM patients WHERE national_id = '123456782'").await.into_iter().filter_map(|r| r[0].clone()).collect();
    assert!(plan.iter().any(|l| l.contains("patients: index patients_national_id_key, 1 key(s)")), "{plan:?}");
    // A lookup in the transaction's own writes.
    admin.execute_one("BEGIN").await.unwrap();
    admin.execute_one("INSERT INTO patients (patient_id, national_id, full_name) VALUES (60, '606060606', 'Mid Txn')").await.unwrap();
    assert_eq!(one(&admin, "SELECT full_name FROM patients WHERE national_id = '606060606'").await.len(), 1);
    admin.execute_one("ROLLBACK").await.unwrap();
    assert_eq!(one(&admin, "SELECT full_name FROM patients WHERE national_id = '606060606'").await.len(), 0);
}

#[tokio::test]
async fn savepoints() {
    let s = DepServer::new();
    clinic(&s).await;
    let a = s.connect("operational").unwrap();
    assert_eq!(a.execute_one("SAVEPOINT s").await.unwrap_err().sqlstate, "25P01");
    a.execute_one("BEGIN").await.unwrap();
    a.execute_one("INSERT INTO patients (patient_id, national_id, full_name) VALUES (70, '707070707', 'A')").await.unwrap();
    a.execute_one("SAVEPOINT s1").await.unwrap();
    a.execute_one("INSERT INTO patients (patient_id, national_id, full_name) VALUES (71, '717171717', 'B')").await.unwrap();
    a.execute_one("UPDATE patients SET full_name = 'A2' WHERE patient_id = 70").await.unwrap();
    a.execute_one("CREATE TABLE notes (id BIGINT PRIMARY KEY, body TEXT)").await.unwrap();
    // A failing statement fails the transaction, until ROLLBACK TO.
    assert_eq!(a.execute_one("INSERT INTO patients (patient_id, full_name) VALUES (70, 'dup')").await.unwrap_err().sqlstate, "23505");
    assert_eq!(a.execute_one("SELECT 1").await.unwrap_err().sqlstate, "25P02");
    a.execute_one("ROLLBACK TO SAVEPOINT s1").await.unwrap();
    // Back to the savepoint: B, the update and the table are gone; A is there, and the indexes agree.
    assert_eq!(one(&a, "SELECT full_name FROM patients WHERE patient_id = 70").await, vec![vec![Some("A".into())]]);
    assert_eq!(one(&a, "SELECT full_name FROM patients WHERE national_id = '717171717'").await.len(), 0);
    assert!(!one(&a, "SHOW TABLES").await.iter().any(|r| r[0].as_deref() == Some("notes")));
    // A row that breaks its second unique key leaves nothing in the first (here: patient_id 73 is
    // new, its national_id is taken), so after ROLLBACK TO the id is free.
    a.execute_one("SAVEPOINT s1b").await.unwrap();
    assert_eq!(a.execute_one("INSERT INTO patients (patient_id, national_id, full_name) VALUES (73, '123456782', 'X')").await.unwrap_err().sqlstate, "23505");
    a.execute_one("ROLLBACK TO s1b").await.unwrap();
    a.execute_one("INSERT INTO patients (patient_id, national_id, full_name) VALUES (73, '737373737', 'D')").await.unwrap();
    a.execute_one("DELETE FROM patients WHERE patient_id = 73").await.unwrap();
    // The key B had is free again.
    a.execute_one("INSERT INTO patients (patient_id, national_id, full_name) VALUES (71, '727272727', 'C')").await.unwrap();
    a.execute_one("SAVEPOINT s2").await.unwrap();
    a.execute_one("RELEASE SAVEPOINT s2").await.unwrap();
    assert_eq!(a.execute_one("ROLLBACK TO s2").await.unwrap_err().sqlstate, "3B001");
    a.execute_one("ROLLBACK").await.unwrap();
    a.execute_one("BEGIN").await.unwrap();
    a.execute_one("INSERT INTO patients (patient_id, national_id, full_name) VALUES (70, '707070707', 'A')").await.unwrap();
    a.execute_one("SAVEPOINT s1").await.unwrap();
    a.execute_one("INSERT INTO patients (patient_id, national_id, full_name) VALUES (71, '717171717', 'B')").await.unwrap();
    a.execute_one("ROLLBACK TO s1").await.unwrap();
    a.execute_one("INSERT INTO patients (patient_id, national_id, full_name) VALUES (72, '727272727', 'C')").await.unwrap();
    a.execute_one("COMMIT").await.unwrap();
    let b = s.connect("operational").unwrap();
    let names = one(&b, "SELECT full_name FROM patients WHERE patient_id >= 70 ORDER BY patient_id").await;
    assert_eq!(names, vec![vec![Some("A".into())], vec![Some("C".into())]]);
}

#[tokio::test]
async fn schema_changes_are_transactional() {
    let dir = tempdir("ddl");
    {
        let (s, _) = persist::open(&dir, None).await.unwrap();
        clinic(&s).await;
        let a = s.connect("operational").unwrap();
        let b = s.connect("operational").unwrap();
        a.execute_one("BEGIN").await.unwrap();
        a.execute_one("CREATE TABLE visits (visit_id BIGINT PRIMARY KEY, patient_id BIGINT REFERENCES patients (patient_id), note TEXT)").await.unwrap();
        a.execute_one("INSERT INTO visits VALUES (1, 1, 'ok')").await.unwrap();
        a.execute_one("CREATE ROLE nurse_app LOGIN PASSWORD 'pw'").await.unwrap();
        a.execute_one("GRANT SELECT ON visits TO nurse_app").await.unwrap();
        a.execute_one("GRANT CONNECT ON ENDPOINT operational TO nurse_app").await.unwrap();
        assert_eq!(b.execute_one("SELECT * FROM visits").await.unwrap_err().sqlstate, "42P01");
        assert!(s.connect_as("nurse_app", "operational").is_err(), "the role does not exist for others yet");
        a.execute_one("COMMIT").await.unwrap();
        assert_eq!(one(&b, "SELECT note FROM visits").await, vec![vec![Some("ok".into())]]);
        // DROP TABLE, then ROLLBACK: the table and its rows are still there.
        a.execute_one("BEGIN").await.unwrap();
        a.execute_one("DROP TABLE visits").await.unwrap();
        assert_eq!(a.execute_one("SELECT * FROM visits").await.unwrap_err().sqlstate, "42P01");
        a.execute_one("ROLLBACK").await.unwrap();
        assert_eq!(one(&a, "SELECT note FROM visits").await.len(), 1);
        // DROP TABLE, then COMMIT: gone, and a new table of the same name starts empty.
        a.execute_one("BEGIN").await.unwrap();
        a.execute_one("DROP TABLE visits").await.unwrap();
        a.execute_one("CREATE TABLE visits (visit_id BIGINT PRIMARY KEY)").await.unwrap();
        a.execute_one("COMMIT").await.unwrap();
        assert_eq!(one(&a, "SELECT * FROM visits").await.len(), 0);
        a.execute_one("INSERT INTO visits VALUES (5)").await.unwrap();
    }
    let (s, _) = persist::open(&dir, None).await.unwrap();
    // The role and its CONNECT grant were committed; its privileges on the dropped table went with the table.
    let n = s.connect_as("nurse_app", "operational").unwrap();
    assert_eq!(n.execute_one("SELECT * FROM visits").await.unwrap_err().sqlstate, "42501");
    let a = s.connect("operational").unwrap();
    assert_eq!(one(&a, "SELECT visit_id FROM visits").await, vec![vec![Some("5".into())]]);
    assert!(s.connect_as("reception_app", "operational").is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn large_tables_stream_and_spill_encrypted() {
    let dir = tempdir("large");
    let opts = ServerOptions { memory_limit: Some(16 << 20), cache_size: Some(16 << 20), target_partitions: Some(1), ..Default::default() };
    let n = 60_000u64;
    {
        let (s, _) = persist::open_with(&dir, None, &opts).await.unwrap();
        let a = s.connect("operational").unwrap();
        a.execute_one("CREATE TABLE events (id BIGINT PRIMARY KEY, who TEXT PROTECTED MATCH DOMAIN person_id, body TEXT)").await.unwrap();
        a.execute_one(&format!("INSERT INTO events SELECT value, CAST(value % 1000 AS TEXT), repeat('x', 250) || CAST(value AS TEXT) FROM generate_series(1, {n})"))
            .await
            .unwrap();
        assert_eq!(one(&a, "SELECT count(*) FROM events").await, vec![vec![Some(n.to_string())]]);
        // Larger than the query memory: the sort spills, to encrypted files.
        let top = one(&a, "SELECT id FROM events ORDER BY body DESC").await;
        assert_eq!(top.len(), 60_000);
        assert_eq!(top[0][0].as_deref(), Some("9999"));
        let spilled = s.spill.as_ref().unwrap().written.load(Ordering::Relaxed);
        assert!(spilled > 0, "the sort did not spill");
        // Protected lookups go through the index.
        let hits = one(&a, "SELECT count(*) FROM events WHERE who = '7'").await;
        assert_eq!(hits, vec![vec![Some((n / 1000).to_string())]]);
        let t = one(&a, "SHOW TABLES").await;
        assert!(t.iter().any(|r| r[0].as_deref() == Some("events") && r[2].as_deref() == Some(&n.to_string()[..])));
    }
    // Spill files are gone, and nothing in the directory holds a protected value.
    let spill = dir.join("spill");
    assert_eq!(std::fs::read_dir(&spill).map(|d| d.count()).unwrap_or(0), 0);
    // Opening reads the catalog only; the rows are read when asked for.
    let started = std::time::Instant::now();
    let (s, _) = persist::open_with(&dir, None, &opts).await.unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    let a = s.connect("operational").unwrap();
    assert_eq!(one(&a, "SELECT body FROM events WHERE id = 31337").await, vec![vec![Some(format!("{}31337", "x".repeat(250)))]]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_0_4_directory_is_migrated() {
    use redb::{Database, ReadableDatabase, TableDefinition};
    let dir = tempdir("v04");
    std::fs::create_dir_all(&dir).unwrap();
    let key = [9u8; 32];
    std::fs::write(dir.join("master.key"), hex::encode(key)).unwrap();
    let k = dep_engine::crypto::Keyring::from_bytes(key);
    let catalog = serde_json::json!({
        "format": 2,
        "endpoints": [["operational", "OPERATIONAL"], ["analytics", "MASKED_READ"]],
        "roles": [{"name": "admin", "login": true, "superuser": true, "password_md5": null}],
        "tables": [{"name": "people", "columns": [
            {"name": "id", "type_name": "BIGINT", "domain": null, "primary_key": true, "references": null},
            {"name": "nid", "type_name": "TEXT", "domain": "person_id", "primary_key": false, "references": null},
            {"name": "name", "type_name": "TEXT", "domain": null, "primary_key": false, "references": null}]}],
        "table_privs": [], "domain_privs": [], "endpoint_grants": [], "functions": [], "fn_execute": []
    });
    {
        let db = Database::create(dir.join("data.redb")).unwrap();
        let w = db.begin_write().unwrap();
        {
            let meta: TableDefinition<&str, &[u8]> = TableDefinition::new("dep_meta");
            let mut m = w.open_table(meta).unwrap();
            m.insert("format", b"2".as_slice()).unwrap();
            m.insert("key_fingerprint", k.fingerprint().as_bytes()).unwrap();
            m.insert("catalog", serde_json::to_vec(&catalog).unwrap().as_slice()).unwrap();
            m.insert("next_rowid/people", b"4".as_slice()).unwrap();
            let rows: TableDefinition<u64, &[u8]> = TableDefinition::new("rows/people");
            let mut t = w.open_table(rows).unwrap();
            // Two rows share id 1: 0.4 did not enforce primary keys.
            for (rid, id, nid, name) in [(1u64, 1, "111", "Ann"), (2, 2, "222", "Ben"), (3, 1, "333", "Dup")] {
                let row = serde_json::json!({"id": id, "nid": k.index_entry("person_id", nid), "nid__ct": k.encrypt("person_id", nid), "name": name});
                t.insert(rid, serde_json::to_vec(&row).unwrap().as_slice()).unwrap();
            }
            let audit: TableDefinition<u64, &[u8]> = TableDefinition::new("dep_audit");
            let rec = serde_json::json!({"seq": 1, "at": "2026-10-01T00:00:00Z", "session_user": "admin", "role": "admin", "endpoint": "operational", "path": "OPERATIONAL", "kind": "match", "object": "people.nid", "domain": "person_id", "count": 1, "detail": null});
            w.open_table(audit).unwrap().insert(1u64, serde_json::to_vec(&rec).unwrap().as_slice()).unwrap();
        }
        w.commit().unwrap();
        drop(db);
    }
    {
        let (s, fresh) = persist::open(&dir, None).await.unwrap();
        assert!(!fresh);
        let notes = s.startup_notes.lock().unwrap().clone();
        assert!(notes.iter().any(|n| n.contains("people_pkey") && n.contains("not enforced")), "{notes:?}");
        let a = s.connect("operational").unwrap();
        assert_eq!(one(&a, "SELECT name FROM people WHERE nid = '222'").await, vec![vec![Some("Ben".into())]]);
        let idx = one(&a, "SHOW INDEXES FROM people").await;
        assert!(idx.iter().any(|r| r[1].as_deref() == Some("people_pkey") && r[4].as_deref() == Some("no (duplicate keys)")));
        // The audit log came along.
        assert!(one(&a, "SHOW AUDIT 10").await.iter().any(|r| r[0].as_deref() == Some("1") && r[7].as_deref() == Some("people.nid")));
        // Remove the duplicate, then REINDEX: the key is enforced again.
        a.execute_one("DELETE FROM people WHERE name = 'Dup'").await.unwrap();
        a.execute_one("REINDEX TABLE people").await.unwrap();
        assert!(a.notes().iter().any(|n| n.contains("enforced again")));
        assert_eq!(a.execute_one("INSERT INTO people (id, name) VALUES (2, 'x')").await.unwrap_err().sqlstate, "23505");
        a.execute_one("INSERT INTO people (id, nid, name) VALUES (4, '444', 'Dan')").await.unwrap();
    }
    let db = Database::open(dir.join("data.redb")).unwrap();
    let r = db.begin_read().unwrap();
    let meta: TableDefinition<&str, &[u8]> = TableDefinition::new("dep_meta");
    assert_eq!(r.open_table(meta).unwrap().get("format").unwrap().unwrap().value(), b"3");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_transactions_are_ended() {
    let s = DepServer::new();
    clinic(&s).await;
    *s.idle_timeout.write().unwrap() = Some(std::time::Duration::from_secs(1));
    *s.lock_timeout.write().unwrap() = std::time::Duration::from_secs(10);
    s.start_reaper();
    let a = s.connect("operational").unwrap();
    a.execute_one("BEGIN").await.unwrap();
    a.execute_one("DELETE FROM appointments WHERE appointment_id = 101").await.unwrap();
    // a holds the write lock and goes quiet; b waits, and gets it when a's transaction is ended.
    let b = s.connect("operational").unwrap();
    let t = std::time::Instant::now();
    b.execute_one("DELETE FROM appointments WHERE appointment_id = 102").await.unwrap();
    assert!(t.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(a.execute_one("SELECT 1").await.unwrap_err().sqlstate, "25P03");
    assert!(!a.in_txn());
    // a's delete was rolled back.
    assert_eq!(one(&b, "SELECT appointment_id FROM appointments WHERE appointment_id = 101").await.len(), 1);
}

#[tokio::test]
async fn current_date_works_in_queries_writes_and_function_arguments() {
    let s = DepServer::new();
    clinic(&s).await;
    let a = s.connect("operational").unwrap();
    let today = chrono::Utc::now().date_naive().to_string();
    assert_eq!(one(&a, "SELECT current_date").await[0][0].as_deref(), Some(today.as_str()));
    assert_eq!(one(&a, "SELECT now() > TIMESTAMP '2020-01-01'").await[0][0].as_deref(), Some("true"));
    a.execute_one("INSERT INTO appointments VALUES (900, 1, now(), 'Walk-in')").await.unwrap();
    a.execute_one("UPDATE appointments SET starts_at = CAST(current_date AS TIMESTAMP) WHERE appointment_id = 900").await.unwrap();
    // A declared function called with current_date: the argument is folded before the body runs.
    let r = one(&a, "SELECT visit_type FROM monthly_fund_report(current_date) WHERE visit_type = 'Walk-in'").await;
    assert_eq!(r.len(), 1);
    let r = one(&a, "SELECT count(*) FROM appointment_reminders(current_date + 1)").await;
    assert!(r[0][0].is_some());
}

#[tokio::test]
async fn order_by_after_a_cast_filter_is_kept() {
    // DataFusion 55 infers from CAST(ts AS DATE) = d that ts is constant, and drops the sort.
    let s = DepServer::new();
    clinic(&s).await;
    let a = s.connect("operational").unwrap();
    let asc = one(&a, "SELECT a.appointment_id FROM appointments a JOIN patients p USING (patient_id) WHERE CAST(a.starts_at AS DATE) = DATE '2026-10-22' ORDER BY a.starts_at").await;
    assert_eq!(asc, vec![vec![Some("103".into())], vec![Some("104".into())], vec![Some("105".into())]]);
    let desc = one(&a, "SELECT appointment_id FROM appointments WHERE CAST(starts_at AS DATE) = DATE '2026-10-22' ORDER BY starts_at DESC").await;
    assert_eq!(desc, vec![vec![Some("105".into())], vec![Some("104".into())], vec![Some("103".into())]]);
    let f = one(&a, "SELECT starts_at FROM appointment_reminders(DATE '2026-10-22') ORDER BY starts_at").await;
    assert_eq!(f.iter().map(|r| r[0].clone().unwrap()).collect::<Vec<_>>(), vec!["2026-10-22T09:00:00", "2026-10-22T12:45:00", "2026-10-22T16:00:00"]);
}
