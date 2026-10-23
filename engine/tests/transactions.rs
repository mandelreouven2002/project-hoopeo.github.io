//! The DEP Transaction Manager on redb, the audit log, match limits, key rotation and
//! temporary tables.

use datafusion::arrow::array::Array;
use datafusion::arrow::util::display::array_value_to_string;
use dep_engine::{persist, DepServer, Output};
use std::path::PathBuf;
use std::sync::Arc;

const CLINIC: &str = include_str!("../../public/examples/clinic.sql");

fn tempdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hoopeo-tx-{name}-{}", std::process::id()));
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

async fn count(c: &dep_engine::Connection, sql: &str) -> usize {
    rows(&c.execute_one(sql).await.unwrap()).len()
}

const NEW_PATIENT: &str = "INSERT INTO patients (patient_id, national_id, phone, full_name, health_fund) VALUES (70, '111111118', '+972-50-7000000', 'Tal Peretz', 'Clalit')";

#[tokio::test]
async fn commit_rollback_and_isolation() {
    let s = DepServer::new();
    clinic(&s).await;
    let a = s.connect("operational").unwrap();
    let b = s.connect("operational").unwrap();
    let before = count(&b, "SELECT patient_id FROM patients").await;

    // Rolled back: nobody ever sees the row.
    a.execute_one("BEGIN").await.unwrap();
    assert!(a.in_txn());
    a.execute_one(NEW_PATIENT).await.unwrap();
    assert_eq!(count(&a, "SELECT patient_id FROM patients").await, before + 1, "a transaction sees its own writes");
    assert_eq!(count(&b, "SELECT patient_id FROM patients").await, before, "others do not see uncommitted writes");
    a.execute_one("ROLLBACK").await.unwrap();
    assert!(!a.in_txn());
    assert_eq!(count(&a, "SELECT patient_id FROM patients").await, before);

    // Committed: everyone sees it, and protected values in it can be matched.
    a.execute_one("BEGIN").await.unwrap();
    a.execute_one(NEW_PATIENT).await.unwrap();
    a.execute_one("UPDATE patients SET health_fund = 'Maccabi' WHERE national_id = '111111118'").await.unwrap();
    a.execute_one("COMMIT").await.unwrap();
    let o = b.execute_one("SELECT full_name, health_fund FROM patients WHERE national_id = '111111118'").await.unwrap();
    assert_eq!(rows(&o), vec![vec![Some("Tal Peretz".into()), Some("Maccabi".into())]]);

    // A transaction reads a snapshot: a commit by another connection after BEGIN is not seen.
    a.execute_one("BEGIN").await.unwrap();
    let seen = count(&a, "SELECT patient_id FROM patients").await;
    b.execute_one("DELETE FROM patients WHERE national_id = '111111118'").await.unwrap();
    assert_eq!(count(&a, "SELECT patient_id FROM patients").await, seen);
    // Writing a table that changed since the snapshot is a serialization failure.
    let e = a.execute_one("UPDATE patients SET full_name = 'x' WHERE national_id = '123456782'").await.unwrap_err();
    assert_eq!(e.sqlstate, "40001");
    // The transaction is now aborted until it ends.
    assert_eq!(a.execute_one("SELECT 1").await.unwrap_err().sqlstate, "25P02");
    assert!(matches!(a.execute_one("COMMIT").await.unwrap(), Output::Command(t) if t == "ROLLBACK"));
    assert_eq!(count(&a, "SELECT patient_id FROM patients").await, before);

    // Schema changes are transactional: a rolled-back CREATE TABLE leaves nothing behind.
    a.execute_one("BEGIN").await.unwrap();
    a.execute_one("CREATE TABLE t (id BIGINT)").await.unwrap();
    a.execute_one("INSERT INTO t VALUES (1)").await.unwrap();
    assert_eq!(count(&a, "SELECT id FROM t").await, 1);
    assert_eq!(b.execute_one("SELECT id FROM t").await.unwrap_err().sqlstate, "42P01", "others do not see an uncommitted table");
    a.execute_one("ROLLBACK").await.unwrap();
    assert_eq!(a.execute_one("SELECT id FROM t").await.unwrap_err().sqlstate, "42P01");

    // COMMIT without BEGIN is harmless, with a note.
    a.execute_one("COMMIT").await.unwrap();
    assert!(a.notes().iter().any(|n| n.contains("no transaction in progress")));
}

#[tokio::test]
async fn one_writer_at_a_time() {
    let s = DepServer::new();
    clinic(&s).await;
    *s.lock_timeout.write().unwrap() = std::time::Duration::from_millis(300);
    let a = s.connect("operational").unwrap();
    let b = s.connect("operational").unwrap();
    a.execute_one("BEGIN").await.unwrap();
    a.execute_one(NEW_PATIENT).await.unwrap();
    // b waits for the write lock, then gives up.
    let e = b.execute_one("DELETE FROM appointments WHERE appointment_id = 101").await.unwrap_err();
    assert_eq!(e.sqlstate, "55P03");
    // Reads never wait.
    assert!(count(&b, "SELECT patient_id FROM patients").await > 0);
    a.execute_one("COMMIT").await.unwrap();
    b.execute_one("DELETE FROM appointments WHERE appointment_id = 101").await.unwrap();
    // A connection dropped in a transaction rolls it back and releases the lock.
    {
        let c = s.connect("operational").unwrap();
        c.execute_one("BEGIN").await.unwrap();
        c.execute_one("DELETE FROM appointments").await.unwrap();
    }
    assert!(count(&b, "SELECT appointment_id FROM appointments").await > 0);
    b.execute_one("DELETE FROM appointments WHERE appointment_id = 102").await.unwrap();
}

#[tokio::test]
async fn commits_survive_a_restart_and_rollbacks_do_not() {
    let dir = tempdir("restart");
    {
        let (s, _) = persist::open(&dir, None).await.unwrap();
        clinic(&s).await;
        let a = s.connect("operational").unwrap();
        a.execute_one("BEGIN").await.unwrap();
        a.execute_one(NEW_PATIENT).await.unwrap();
        a.execute_one("COMMIT").await.unwrap();
        a.execute_one("BEGIN").await.unwrap();
        // Appointments reference patients, so the patients cannot all go first.
        assert_eq!(a.execute_one("DELETE FROM patients").await.unwrap_err().sqlstate, "23503");
        a.execute_one("ROLLBACK").await.unwrap();
        a.execute_one("BEGIN").await.unwrap();
        a.execute_one("DELETE FROM appointments").await.unwrap();
        a.execute_one("DELETE FROM medical_records").await.unwrap();
        a.execute_one("DELETE FROM patients WHERE patient_id <> 70").await.unwrap();
        a.execute_one("ROLLBACK").await.unwrap();
        a.execute_one("UPDATE patients SET full_name = 'Dana C.' WHERE national_id = '123456782'").await.unwrap();
        a.execute_one("CREATE TEMPORARY TABLE scratch (id BIGINT, v TEXT PROTECTED MATCH DOMAIN d)").await.unwrap();
        a.execute_one("INSERT INTO scratch VALUES (1, 'x')").await.unwrap();
        assert_eq!(count(&a, "SELECT id FROM scratch WHERE v = 'x'").await, 1);
        // An open transaction at shutdown is lost.
        a.execute_one("BEGIN").await.unwrap();
        a.execute_one("DELETE FROM appointments").await.unwrap();
    }
    assert!(dir.join("data.redb").exists());
    let (s, fresh) = persist::open(&dir, None).await.unwrap();
    assert!(!fresh);
    let c = s.connect("operational").unwrap();
    assert_eq!(rows(&c.execute_one("SELECT full_name FROM patients WHERE national_id = '111111118'").await.unwrap()), vec![vec![Some("Tal Peretz".into())]]);
    assert_eq!(rows(&c.execute_one("SELECT full_name FROM patients WHERE national_id = '123456782'").await.unwrap()), vec![vec![Some("Dana C.".into())]]);
    assert_eq!(count(&c, "SELECT patient_id FROM patients").await, 7);
    assert!(count(&c, "SELECT appointment_id FROM appointments").await > 0);
    // Temporary tables are not kept.
    assert_eq!(c.execute_one("SELECT id FROM scratch").await.unwrap_err().sqlstate, "42P01");
    // New rows get new row ids after the restart.
    c.execute_one("INSERT INTO patients (patient_id, full_name) VALUES (71, 'After')").await.unwrap();
    assert_eq!(count(&c, "SELECT patient_id FROM patients").await, 8);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_audit_log_records_uses_not_values() {
    let dir = tempdir("audit");
    {
        let (s, _) = persist::open(&dir, None).await.unwrap();
        clinic(&s).await;
        let rec = s.connect_as("reception_app", "operational").unwrap();
        rec.execute_one("SELECT full_name FROM patients WHERE national_id IN ('123456782', '234567893')").await.unwrap();
        let _ = rec.execute_one("SELECT full_name FROM patients WHERE national_id LIKE '1%'").await;
        let bill = s.connect_as("billing_job", "operational").unwrap();
        bill.execute_one("SELECT * FROM monthly_fund_report(DATE '2026-10-01')").await.unwrap();
    }
    let (s, _) = persist::open(&dir, None).await.unwrap();
    let admin = s.connect("operational").unwrap();
    let log = rows(&admin.execute_one("SHOW AUDIT 50").await.unwrap());
    // seq, at, session_user, role, endpoint, path, kind, object, domain, count, detail
    let find = |kind: &str, role: &str| log.iter().find(|r| r[6].as_deref() == Some(kind) && r[3].as_deref() == Some(role)).cloned();
    let m = find("match", "reception_app").expect("match record");
    assert_eq!(m[7].as_deref(), Some("patients.national_id"));
    assert_eq!(m[8].as_deref(), Some("person_id"));
    assert_eq!(m[9].as_deref(), Some("2"));
    let d = find("denied", "reception_app").expect("denied record");
    assert_eq!(d[10].as_deref(), Some("DEP_PROTECTED_OPERATION"));
    let disc = find("disclose", "billing_job").expect("disclose record");
    assert_eq!(disc[7].as_deref(), Some("monthly_fund_report()"));
    // Only superusers read it.
    let rec = s.connect_as("reception_app", "operational").unwrap();
    assert_eq!(rec.execute_one("SHOW AUDIT").await.unwrap_err().sqlstate, "42501");
    // No protected value anywhere in the data directory, the audit log included.
    let bytes = std::fs::read(dir.join("data.redb")).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    for secret in ["123456782", "234567893", "+972-50-0000001"] {
        assert!(!text.contains(secret), "{secret} found in data.redb");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn match_limits() {
    let s = DepServer::new();
    clinic(&s).await;
    let admin = s.connect("operational").unwrap();
    admin.execute_one("ALTER ROLE reception_app MATCH LIMIT 3 PER MINUTE").await.unwrap();
    let rec = s.connect_as("reception_app", "operational").unwrap();
    rec.execute_one("SELECT full_name FROM patients WHERE national_id IN ('123456782', '234567893')").await.unwrap();
    rec.execute_one("SELECT full_name FROM patients WHERE national_id = '345678904'").await.unwrap();
    let e = rec.execute_one("SELECT full_name FROM patients WHERE national_id = '456789015'").await.unwrap_err();
    assert_eq!(e.code, Some("DEP_MATCH_RATE_LIMITED"));
    assert_eq!(e.sqlstate, "ZD109");
    // Reads that match no protected value are not counted.
    rec.execute_one("SELECT full_name FROM patients").await.unwrap();
    let roles = rows(&admin.execute_one("SHOW ROLES").await.unwrap());
    assert!(roles.iter().any(|r| r[0].as_deref() == Some("reception_app") && r[4].as_deref() == Some("3 PER MINUTE")));
    admin.execute_one("ALTER ROLE reception_app MATCH LIMIT NONE").await.unwrap();
    rec.execute_one("SELECT full_name FROM patients WHERE national_id = '456789015'").await.unwrap();
    // A server default applies to roles without their own limit, and never to superusers.
    *s.default_match_limit.write().unwrap() = dep_engine::audit::MatchLimit::parse("1/hour");
    let doc = s.connect_as("doctors_app", "operational").unwrap();
    doc.execute_one("SELECT diagnosis FROM medical_records WHERE patient_national_id = '123456782'").await.unwrap();
    assert_eq!(doc.execute_one("SELECT diagnosis FROM medical_records WHERE patient_national_id = '234567893'").await.unwrap_err().sqlstate, "ZD109");
    admin.execute_one("SELECT full_name FROM patients WHERE national_id = '123456782'").await.unwrap();
    admin.execute_one("SELECT full_name FROM patients WHERE national_id = '234567893'").await.unwrap();
}

#[tokio::test]
async fn key_rotation() {
    let dir = tempdir("rotate");
    let old_key;
    {
        let (s, _) = persist::open(&dir, None).await.unwrap();
        clinic(&s).await;
        old_key = std::fs::read_to_string(dir.join("master.key")).unwrap();
        let admin = s.connect("operational").unwrap();
        let rec = s.connect_as("reception_app", "operational").unwrap();
        assert_eq!(rec.execute_one("ALTER SYSTEM ROTATE MASTER KEY").await.unwrap_err().sqlstate, "42501");
        admin.execute_one("ALTER SYSTEM ROTATE MASTER KEY").await.unwrap();
        assert!(admin.notes().iter().any(|n| n.contains("re-encrypted") || n.contains("encrypted again")));
        // Everything still works with the new key, in this process.
        assert_eq!(count(&rec, "SELECT full_name FROM patients WHERE national_id = '123456782'").await, 1);
        let bill = s.connect_as("billing_job", "operational").unwrap();
        let o = bill.execute_one("SELECT * FROM monthly_fund_report(DATE '2026-10-01')").await.unwrap();
        assert!(rows(&o).iter().any(|r| r[0].as_deref() == Some("123456782")));
    }
    let new_key = std::fs::read_to_string(dir.join("master.key")).unwrap();
    assert_ne!(old_key, new_key);
    // After a restart, with the new key.
    {
        let (s, _) = persist::open(&dir, None).await.unwrap();
        let rec = s.connect_as("reception_app", "operational").unwrap();
        assert_eq!(count(&rec, "SELECT full_name FROM patients WHERE national_id = '123456782'").await, 1);
    }
    // The old key no longer opens the data.
    std::fs::write(dir.join("master.key"), old_key).unwrap();
    assert_eq!(persist::open(&dir, None).await.err().unwrap().sqlstate, "28P01");
    // A crash after the commit but before the key file moved is finished at the next start.
    std::fs::write(dir.join("master.key.new"), &new_key).unwrap();
    let (s, _) = persist::open(&dir, None).await.unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("master.key")).unwrap(), new_key);
    assert!(!dir.join("master.key.new").exists());
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}
