//! The data directory: state survives a restart, nothing in it is plaintext, and the key matters.

use datafusion::arrow::array::Array;
use datafusion::arrow::util::display::array_value_to_string;
use dep_engine::{persist, Output};
use std::path::PathBuf;

const CLINIC: &str = include_str!("../../public/examples/clinic.sql");

fn tempdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("depsql-test-{name}-{}", std::process::id()));
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

#[tokio::test]
async fn state_survives_a_restart() {
    let dir = tempdir("restart");
    {
        let (s, fresh) = persist::open(&dir, None).await.unwrap();
        assert!(fresh);
        let admin = s.connect("operational").unwrap();
        for r in admin.execute(CLINIC).await {
            r.unwrap();
        }
        let rec = s.connect_as("reception_app", "operational").unwrap();
        rec.execute_one("UPDATE patients SET full_name = 'Dana Cohen-Levi' WHERE national_id = '123456782'").await.unwrap();
        rec.execute_one("INSERT INTO patients (patient_id, national_id, phone, full_name, health_fund) VALUES (7, '789012348', '+972-50-0000007', 'Lior Katz', 'Clalit')").await.unwrap();
        admin.execute_one("REVOKE SELECT ON medical_records FROM privacy_admin").await.unwrap();
        s.set_password("admin", "secret").unwrap();
        persist::flush(&s).await.unwrap();
        // A second server cannot open the same directory while the first runs.
        assert_eq!(persist::open(&dir, None).await.err().unwrap().sqlstate, "55006");
    }
    let (s, fresh) = persist::open(&dir, None).await.unwrap();
    assert!(!fresh);
    assert!(s.check_password("admin", "secret"));
    assert!(s.check_password("reception_app", "demo"));
    let rec = s.connect_as("reception_app", "operational").unwrap();
    let o = rec.execute_one("SELECT full_name FROM patients WHERE national_id = '123456782'").await.unwrap();
    assert_eq!(rows(&o), vec![vec![Some("Dana Cohen-Levi".into())]]);
    assert_eq!(rows(&rec.execute_one("SELECT full_name FROM patients WHERE national_id = '789012348'").await.unwrap()).len(), 1);
    // Privileges, endpoints, and the owner's privileges on functions came back too.
    assert_eq!(rec.execute_one("SELECT phone FROM patients").await.unwrap_err().sqlstate, "42501");
    assert!(s.connect_as("reception_app", "analytics").is_err());
    let bill = s.connect_as("billing_job", "operational").unwrap();
    assert_eq!(rows(&bill.execute_one("SELECT * FROM monthly_fund_report(DATE '2026-10-01')").await.unwrap()).len(), 10);
    assert_eq!(bill.execute_one("SELECT * FROM fund_claims_report(DATE '2026-10-01')").await.unwrap_err().sqlstate, "42501");
    let bi = s.connect_as("bi_dashboard", "analytics").unwrap();
    // diagnoses_per_fund reads medical_records too, and its owner lost that privilege before the restart.
    assert_eq!(bi.execute_one("SELECT * FROM diagnoses_per_fund()").await.unwrap_err().sqlstate, "42501");
    assert_eq!(rows(&bi.execute_one("SELECT health_fund, count(*) FROM patients GROUP BY health_fund").await.unwrap()).len(), 4);
    let fns = s.describe_functions();
    assert!(fns.iter().any(|f| f.name == "monthly_fund_report" && f.owner == "privacy_admin" && f.comment.is_some()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_directory_holds_no_plaintext() {
    let dir = tempdir("plaintext");
    {
        let (s, _) = persist::open(&dir, None).await.unwrap();
        let admin = s.connect("operational").unwrap();
        for r in admin.execute(CLINIC).await {
            r.unwrap();
        }
    }
    for entry in walk(&dir) {
        if entry.file_name().map(|n| n == "master.key").unwrap_or(false) {
            continue;
        }
        let bytes = std::fs::read(&entry).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        for secret in ["123456782", "234567893", "+972-50-0000001"] {
            assert!(!text.contains(secret), "{secret} found in {}", entry.display());
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_key_is_required() {
    let dir = tempdir("key");
    let key = std::env::temp_dir().join(format!("depsql-test-keyfile-{}", std::process::id()));
    let _ = std::fs::remove_file(&key);
    {
        let (s, _) = persist::open(&dir, Some(&key)).await.unwrap();
        s.connect("operational").unwrap().execute_strict("CREATE TABLE t (id BIGINT, v TEXT PROTECTED MATCH DOMAIN d); INSERT INTO t VALUES (1, 'x')").await.unwrap();
    }
    assert!(!dir.join("master.key").exists());
    let moved = key.with_extension("moved");
    std::fs::rename(&key, &moved).unwrap();
    assert_eq!(persist::open(&dir, Some(&key)).await.err().unwrap().sqlstate, "58030");
    let (s, _) = persist::open(&dir, Some(&moved)).await.unwrap();
    let o = s.connect("operational").unwrap().execute_one("SELECT id FROM t WHERE v = 'x'").await.unwrap();
    assert_eq!(rows(&o).len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&moved);
}

fn walk(d: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(d).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

/// A data directory written by DEP Engine 0.3 (catalog.json and tables/*.arrow) is moved
/// into data.redb when it is first opened, and its old files are kept aside.
#[tokio::test]
async fn a_0_3_directory_is_migrated() {
    use datafusion::arrow::array::{Int64Array, StringArray};
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::arrow::ipc::writer::FileWriter;
    use datafusion::arrow::record_batch::RecordBatch;
    use std::sync::Arc;
    let dir = tempdir("legacy");
    std::fs::create_dir_all(dir.join("tables")).unwrap();
    let key = [7u8; 32];
    std::fs::write(dir.join("master.key"), hex::encode(key)).unwrap();
    let k = dep_engine::crypto::Keyring::from_bytes(key);
    let catalog = serde_json::json!({
        "format": 1,
        "endpoints": [["operational", "OPERATIONAL"], ["analytics", "MASKED_READ"]],
        "roles": [{"name": "admin", "login": true, "superuser": true, "password_md5": null}],
        "tables": [{"name": "t", "columns": [
            {"name": "id", "type_name": "BIGINT", "domain": null, "primary_key": true, "references": null},
            {"name": "email", "type_name": "TEXT", "domain": "email", "primary_key": false, "references": null}]}],
        "table_privs": [], "domain_privs": [], "endpoint_grants": [], "functions": [], "fn_execute": []
    });
    std::fs::write(dir.join("catalog.json"), serde_json::to_vec(&catalog).unwrap()).unwrap();
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new("email", DataType::Utf8, true),
        Field::new("email__ct", DataType::Utf8, true),
    ]));
    let emails = ["a@example.com", "b@example.com"];
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(emails.iter().map(|e| k.index_entry("email", e)).collect::<Vec<_>>())),
            Arc::new(StringArray::from(emails.iter().map(|e| k.encrypt("email", e)).collect::<Vec<_>>())),
        ],
    )
    .unwrap();
    let mut buf = Vec::new();
    {
        let mut w = FileWriter::try_new(&mut buf, &schema).unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();
    }
    std::fs::write(dir.join("tables/t.arrow"), buf).unwrap();

    {
        let (s, fresh) = persist::open(&dir, None).await.unwrap();
        assert!(!fresh);
        let c = s.connect("operational").unwrap();
        assert_eq!(rows(&c.execute_one("SELECT id FROM t WHERE email = 'b@example.com'").await.unwrap()), vec![vec![Some("2".into())]]);
        c.execute_one("INSERT INTO t VALUES (3, 'c@example.com')").await.unwrap();
    }
    assert!(dir.join("data.redb").exists());
    assert!(dir.join("legacy-0.3/catalog.json").exists());
    assert!(!dir.join("catalog.json").exists());
    let (s, _) = persist::open(&dir, None).await.unwrap();
    let c = s.connect("operational").unwrap();
    assert_eq!(rows(&c.execute_one("SELECT id FROM t ORDER BY id").await.unwrap()).len(), 3);
    assert_eq!(rows(&c.execute_one("SELECT id FROM t WHERE email = 'a@example.com'").await.unwrap()), vec![vec![Some("1".into())]]);
    let _ = std::fs::remove_dir_all(&dir);
}
