//! The examples of the DEP SQL Playground, and the rules of the design proposal,
//! on both access paths.

use datafusion::arrow::array::Array;
use datafusion::arrow::util::display::array_value_to_string;
use dep_engine::{DepServer, Output};
use std::sync::Arc;

const CLINIC: &str = include_str!("../../examples/clinic.sql");

async fn clinic() -> Arc<DepServer> {
    let s = DepServer::new();
    let admin = s.connect("operational").unwrap();
    for (i, r) in admin.execute(CLINIC).await.into_iter().enumerate() {
        if let Err(e) = r {
            panic!("setup statement {i} failed: {e}");
        }
    }
    s
}

async fn run(s: &Arc<DepServer>, ep: &str, sql: &str) -> dep_engine::Result<Output> {
    let c = s.connect(ep).unwrap();
    let mut last = None;
    for r in c.execute(sql).await {
        last = Some(r);
    }
    last.unwrap()
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

fn code(r: &dep_engine::Result<Output>) -> &'static str {
    match r {
        Err(e) => e.code.unwrap_or("SQL"),
        Ok(_) => "OK",
    }
}

const OP: &str = "operational";
const MASKED: &str = "analytics";

#[tokio::test]
async fn select_returns_null_for_protected_columns() {
    let s = clinic().await;
    for ep in [OP, MASKED] {
        let o = run(&s, ep, "SELECT full_name, national_id, health_fund FROM patients").await.unwrap();
        let r = rows(&o);
        assert_eq!(r.len(), 6);
        assert!(r.iter().all(|x| x[1].is_none()));
    }
}

#[tokio::test]
async fn wildcard_hides_stored_columns() {
    let s = clinic().await;
    let o = run(&s, OP, "SELECT * FROM patients").await.unwrap();
    let Output::Rows { schema, .. } = &o else { panic!() };
    let names: Vec<_> = schema.fields().iter().map(|f| f.name().clone()).collect();
    assert_eq!(names, ["patient_id", "national_id", "phone", "full_name", "health_fund"]);
    assert!(rows(&o).iter().all(|x| x[1].is_none() && x[2].is_none()));
}

#[tokio::test]
async fn lookup_by_id() {
    let s = clinic().await;
    let r = run(&s, MASKED, "SELECT full_name FROM patients WHERE national_id = '123456782'").await;
    assert_eq!(code(&r), "DEP_ENDPOINT_OPERATION");
    let o = run(&s, OP, "SELECT full_name, national_id FROM patients WHERE national_id = '123456782'").await.unwrap();
    assert_eq!(rows(&o), vec![vec![Some("Dana Cohen".into()), None]]);
    let o = run(&s, OP, "SELECT full_name FROM patients WHERE national_id = '000000000'").await.unwrap();
    assert!(rows(&o).is_empty());
}

#[tokio::test]
async fn only_full_value_equality() {
    let s = clinic().await;
    for sql in [
        "SELECT full_name FROM patients WHERE national_id LIKE '1234%'",
        "SELECT full_name FROM patients WHERE substr(national_id, 6, 4) = '6782'",
        "SELECT CAST(national_id AS TEXT) FROM patients",
        "SELECT full_name FROM patients WHERE national_id > '2'",
        "SELECT full_name FROM patients ORDER BY national_id",
        "SELECT count(DISTINCT national_id) FROM patients",
        "SELECT upper(national_id) FROM patients",
    ] {
        for ep in [OP, MASKED] {
            assert_eq!(code(&run(&s, ep, sql).await), "DEP_PROTECTED_OPERATION", "{ep}: {sql}");
        }
    }
}

#[tokio::test]
async fn joins_must_be_anchored() {
    let s = clinic().await;
    let anchored = "SELECT p.full_name, m.diagnosis FROM patients AS p JOIN medical_records AS m ON m.patient_national_id = p.national_id WHERE p.national_id = '123456782'";
    assert_eq!(rows(&run(&s, OP, anchored).await.unwrap()).len(), 2);
    assert_eq!(code(&run(&s, MASKED, anchored).await), "DEP_ENDPOINT_OPERATION");

    let unanchored = "SELECT p.full_name, m.diagnosis FROM patients AS p JOIN medical_records AS m ON m.patient_national_id = p.national_id";
    assert_eq!(code(&run(&s, OP, unanchored).await), "DEP_UNANCHORED_MATCH");
    assert_eq!(code(&run(&s, MASKED, unanchored).await), "DEP_ENDPOINT_OPERATION");

    // An anchor under OR is not an anchor.
    let or = "SELECT m.diagnosis FROM patients AS p JOIN medical_records AS m ON m.patient_national_id = p.national_id WHERE p.national_id = '123456782' OR p.full_name = 'x'";
    assert_eq!(code(&run(&s, OP, or).await), "DEP_UNANCHORED_MATCH");

    // A subquery is a join written differently.
    let sub = "SELECT full_name FROM patients WHERE national_id IN (SELECT patient_national_id FROM medical_records)";
    assert_eq!(code(&run(&s, OP, sub).await), "DEP_UNANCHORED_MATCH");

    // A list of values is a set of lookups, not an anchor, and matches no join.
    let list = "SELECT full_name FROM patients WHERE national_id IN ('123456782', '234567893', '999')";
    assert_eq!(rows(&run(&s, OP, list).await.unwrap()).len(), 2);

    // An anchored derived table anchors the outer join.
    let derived = "SELECT m.diagnosis FROM (SELECT national_id FROM patients WHERE national_id = '123456782') AS d JOIN medical_records AS m ON m.patient_national_id = d.national_id";
    assert_eq!(rows(&run(&s, OP, derived).await.unwrap()).len(), 2);

    // GROUP BY a protected column links every row that shares a value.
    assert_eq!(code(&run(&s, OP, "SELECT patient_national_id, count(*) FROM medical_records GROUP BY patient_national_id").await), "DEP_UNANCHORED_MATCH");
}

#[tokio::test]
async fn match_domains_do_not_mix() {
    let s = clinic().await;
    let r = run(&s, OP, "SELECT full_name FROM patients WHERE national_id = phone").await;
    assert_eq!(code(&r), "DEP_MATCH_DOMAIN_MISMATCH");
}

#[tokio::test]
async fn protection_survives_ctes_and_aliases() {
    let s = clinic().await;
    let o = run(&s, OP, "WITH t AS (SELECT national_id AS x FROM patients) SELECT x FROM t").await.unwrap();
    assert!(rows(&o).iter().all(|r| r[0].is_none()));
    let r = run(&s, OP, "WITH t AS (SELECT national_id AS x FROM patients) SELECT x FROM t WHERE x LIKE '1%'").await;
    assert_eq!(code(&r), "DEP_PROTECTED_OPERATION");
}

#[tokio::test]
async fn linking_function() {
    let s = clinic().await;
    for ep in [OP, MASKED] {
        let o = run(&s, ep, "SELECT * FROM diagnoses_per_fund() ORDER BY health_fund, diagnosis").await.unwrap();
        assert_eq!(rows(&o)[0], vec![Some("Clalit".into()), Some("Asthma".into()), Some("2".into())]);
    }
}

#[tokio::test]
async fn disclosing_function_and_terminal_rule() {
    let s = clinic().await;
    let report = "SELECT * FROM monthly_fund_report(DATE '2026-10-01')";
    assert_eq!(code(&run(&s, MASKED, report).await), "DEP_ENDPOINT_OPERATION");
    let o = run(&s, OP, report).await.unwrap();
    assert_eq!(rows(&o).len(), 10);
    assert!(rows(&o).iter().any(|r| r[0].as_deref() == Some("123456782")));

    let join_back =
        "SELECT r.national_id, m.diagnosis FROM monthly_fund_report(DATE '2026-10-01') AS r JOIN medical_records AS m ON m.patient_national_id = r.national_id";
    assert_eq!(code(&run(&s, OP, join_back).await), "DEP_DISCLOSED_VALUE_USE");
    let filter = "SELECT * FROM monthly_fund_report(DATE '2026-10-01') AS r WHERE r.national_id = '123456782'";
    assert_eq!(code(&run(&s, OP, filter).await), "DEP_DISCLOSED_VALUE_USE");
    let store = "CREATE TABLE copy (nid TEXT); INSERT INTO copy SELECT national_id FROM monthly_fund_report(DATE '2026-10-01')";
    assert_eq!(code(&run(&s, OP, store).await), "DEP_DISCLOSED_VALUE_USE");
}

#[tokio::test]
async fn reveal_only_in_disclosing_functions() {
    let s = clinic().await;
    let r = run(&s, OP, "SELECT full_name, REVEAL(national_id) FROM patients").await;
    assert_eq!(code(&r), "DEP_REVEAL_OUTSIDE_DISCLOSURE");
    let wrong_domain = "CREATE FUNCTION f(id TEXT) RETURNS TABLE (n TEXT) LANGUAGE sql DISCLOSES phone_number AS $$ SELECT REVEAL(p.national_id) FROM patients AS p WHERE p.national_id = id $$";
    assert_eq!(code(&run(&s, OP, wrong_domain).await), "DEP_REVEAL_OUTSIDE_DISCLOSURE");
    let plpgsql = "CREATE FUNCTION g() RETURNS TABLE (n TEXT) LANGUAGE plpgsql DISCLOSES person_id AS $$ SELECT 'x' $$";
    assert_eq!(code(&run(&s, OP, plpgsql).await), "DEP_DECLARED_LANGUAGE");
}

#[tokio::test]
async fn a_new_disclosing_function() {
    let s = clinic().await;
    let sql = "CREATE FUNCTION patient_contact(id TEXT) RETURNS TABLE (full_name TEXT, phone TEXT) LANGUAGE sql DISCLOSES phone_number AS $$
        SELECT p.full_name, REVEAL(p.phone) FROM patients AS p WHERE p.national_id = id $$;
        SELECT * FROM patient_contact('123456782')";
    let o = run(&s, OP, sql).await.unwrap();
    assert_eq!(rows(&o), vec![vec![Some("Dana Cohen".into()), Some("+972-50-0000001".into())]]);
    assert_eq!(code(&run(&s, MASKED, "SELECT * FROM patient_contact('123456782')").await), "DEP_ENDPOINT_OPERATION");
    // A function taking a protected input is a lookup, even when it only links.
    let lk = "CREATE FUNCTION visits_of(id TEXT) RETURNS TABLE (n BIGINT) LANGUAGE sql LINKS person_id AS $$
        SELECT COUNT(*) FROM medical_records AS m JOIN patients AS p ON p.national_id = m.patient_national_id WHERE p.national_id = id $$";
    run(&s, OP, lk).await.unwrap();
    assert_eq!(rows(&run(&s, OP, "SELECT * FROM visits_of('123456782')").await.unwrap()), vec![vec![Some("2".into())]]);
    assert_eq!(code(&run(&s, MASKED, "SELECT * FROM visits_of('123456782')").await), "DEP_ENDPOINT_OPERATION");
}

#[tokio::test]
async fn writes() {
    let s = clinic().await;
    let up = "UPDATE patients SET full_name = 'Dana Cohen-Levi' WHERE national_id = '123456782'";
    assert_eq!(code(&run(&s, MASKED, up).await), "DEP_ENDPOINT_OPERATION");
    assert!(matches!(run(&s, OP, up).await.unwrap(), Output::Command(ref t) if t == "UPDATE 1"));
    let o = run(&s, OP, "SELECT full_name FROM patients WHERE national_id = '123456782'").await.unwrap();
    assert_eq!(rows(&o)[0][0].as_deref(), Some("Dana Cohen-Levi"));

    // Changing a protected value re-indexes and re-encrypts it.
    run(&s, OP, "UPDATE patients SET phone = '+972-50-9999999' WHERE national_id = '123456782'").await.unwrap();
    let o = run(&s, OP, "SELECT full_name FROM patients WHERE phone = '+972-50-9999999'").await.unwrap();
    assert_eq!(rows(&o).len(), 1);
    let o = run(&s, OP, "SELECT * FROM appointment_reminders(DATE '2026-10-21')").await.unwrap();
    assert!(rows(&o).iter().any(|r| r[0].as_deref() == Some("+972-50-9999999")));

    let ins =
        "INSERT INTO patients (patient_id, national_id, phone, full_name, health_fund) VALUES (7, '789012348', '+972-50-0000007', 'New Patient', 'Clalit')";
    assert_eq!(code(&run(&s, MASKED, ins).await), "DEP_ENDPOINT_OPERATION");
    run(&s, OP, ins).await.unwrap();
    assert_eq!(rows(&run(&s, OP, "SELECT full_name FROM patients WHERE national_id = '789012348'").await.unwrap()).len(), 1);

    assert!(matches!(run(&s, OP, "DELETE FROM patients WHERE national_id = '789012348'").await.unwrap(), Output::Command(ref t) if t == "DELETE 1"));
    assert_eq!(rows(&run(&s, OP, "SELECT full_name FROM patients").await.unwrap()).len(), 6);
}

#[tokio::test]
async fn schema_rules() {
    let s = clinic().await;
    let plain_copy = "CREATE TABLE lab_results (result_id BIGINT PRIMARY KEY, patient_national_id TEXT REFERENCES patients (national_id), result TEXT)";
    assert_eq!(code(&run(&s, OP, plain_copy).await), "DEP_REFERENCE_MISMATCH");
    let ok = "CREATE TABLE lab_results (result_id BIGINT PRIMARY KEY, patient_national_id TEXT PROTECTED REFERENCES patients (national_id), result TEXT)";
    run(&s, OP, ok).await.unwrap();
    assert_eq!(code(&run(&s, MASKED, "CREATE TABLE t2 (a TEXT)").await), "DEP_ENDPOINT_OPERATION");
}

#[tokio::test]
async fn path_is_fixed() {
    let s = clinic().await;
    for ep in [OP, MASKED] {
        assert_eq!(code(&run(&s, ep, "SET dep.path = 'OPERATIONAL'").await), "DEP_PATH_IMMUTABLE");
    }
}

#[tokio::test]
async fn stored_columns_and_internal_functions_are_unreachable() {
    let s = clinic().await;
    assert_eq!(run(&s, OP, "SELECT national_id__ct FROM patients").await.unwrap_err().sqlstate, "42703");
    assert_eq!(run(&s, OP, "SELECT dep_decrypt('person_id', 'x')").await.unwrap_err().sqlstate, "42883");
}

#[tokio::test]
async fn storage_holds_no_plaintext() {
    let s = clinic().await;
    let o = s.storage("patients").await.unwrap();
    let Output::Rows { batches, .. } = &o else { panic!() };
    for b in batches {
        for c in 0..b.num_columns() {
            for r in 0..b.num_rows() {
                let v = array_value_to_string(b.column(c), r).unwrap();
                assert!(!v.contains("123456782") && !v.contains("+972-50"), "plaintext in storage: {v}");
            }
        }
    }
    let idx = s.keys().index_entry("person_id", "123456782");
    let found = rows(&o).iter().any(|r| r[1].as_deref() == Some(idx.as_str()));
    assert!(found);
}

#[tokio::test]
async fn matches_outside_top_level_conjuncts_still_count() {
    let s = clinic().await;
    // A match in the select list links rows as much as a join does.
    let r = run(&s, OP, "SELECT p.national_id = m.patient_national_id FROM patients AS p CROSS JOIN medical_records AS m").await;
    assert_eq!(code(&r), "DEP_UNANCHORED_MATCH");
    let r =
        run(&s, OP, "SELECT p.full_name FROM patients AS p CROSS JOIN medical_records AS m WHERE p.national_id = m.patient_national_id OR p.full_name = 'x'")
            .await;
    assert_eq!(code(&r), "DEP_UNANCHORED_MATCH");
    let r = run(&s, OP, "SELECT national_id IN (SELECT patient_national_id FROM medical_records) FROM patients").await;
    assert_eq!(code(&r), "DEP_UNANCHORED_MATCH");
    let r =
        run(&s, OP, "SELECT full_name FROM patients AS p WHERE EXISTS (SELECT 1 FROM medical_records AS m WHERE m.patient_national_id = p.national_id)").await;
    assert_eq!(code(&r), "DEP_UNSUPPORTED_FORM");
}

#[tokio::test]
async fn function_arguments_are_values() {
    let s = clinic().await;
    let r = run(&s, OP, "SELECT * FROM monthly_fund_report((SELECT max(recorded_at) FROM medical_records))").await;
    assert!(r.is_err());
}

#[tokio::test]
async fn dep_notes_explain_how_the_rules_applied() {
    let s = clinic().await;
    let c = s.connect("operational").unwrap();
    let notes = |c: &dep_engine::Connection| c.notes();

    c.execute_one("SELECT p.full_name, m.diagnosis FROM patients AS p JOIN medical_records AS m ON m.patient_national_id = p.national_id WHERE p.national_id = '123456782'").await.unwrap();
    let n = notes(&c);
    assert!(n.iter().any(|x| x.contains("is anchored to a single protected value")), "{n:?}");
    assert!(n.iter().any(|x| x.contains("PROTECTED column patients.national_id is matched by equality")), "{n:?}");

    c.execute_one("SELECT * FROM monthly_fund_report('2026-03-01')").await.unwrap();
    let n = notes(&c);
    assert!(n.iter().any(|x| x.contains("monthly_fund_report() is declared DISCLOSES person_id")), "{n:?}");
    assert!(n.iter().any(|x| x.contains("is disclosed")), "{n:?}");

    // SHOW DEP NOTES reports the notes of the statement before it, and keeps them.
    let shown = rows(&c.execute_one("SHOW DEP NOTES").await.unwrap());
    assert_eq!(shown.len(), n.len());
    assert_eq!(notes(&c), n);

    // Plain SQL has no notes.
    c.execute_one("SELECT full_name, health_fund FROM patients").await.unwrap();
    assert!(notes(&c).iter().all(|x| !x.contains("matched")), "{:?}", notes(&c));
    c.execute_one("SELECT 1").await.unwrap();
    assert!(notes(&c).is_empty());

    // Writes say how protected values are stored.
    c.execute_one("INSERT INTO patients (patient_id, national_id, phone, full_name, health_fund) VALUES (99, '000000018', '050-0000000', 'Test', 'Clalit')").await.unwrap();
    assert!(notes(&c).iter().any(|x| x.contains("stored as a blind index and ciphertext")), "{:?}", notes(&c));
}

#[tokio::test]
async fn privacy_matrix() {
    let s = clinic().await;
    let admin = s.connect("operational").unwrap();
    let m = rows(&admin.execute_one("SHOW PRIVACY MATRIX").await.unwrap());
    let row = |col: &str, role: &str| m.iter().find(|r| r[1].as_deref() == Some(col) && r[2].as_deref() == Some(role)).cloned().unwrap_or_else(|| panic!("{col} {role}"));
    let r = row("patients.national_id", "reception_app");
    assert_eq!(r[0].as_deref(), Some("person_id"));
    assert_eq!(r[4].as_deref(), Some("yes"));
    let bi = row("patients.national_id", "bi_dashboard");
    assert!(bi[3].as_deref().unwrap().contains("analytics (MASKED_READ)"));
    assert!(bi[6].as_deref().unwrap().contains("diagnoses_per_fund()"), "{bi:?}");
    // A role that is not a superuser sees its own rows only.
    let rec = s.connect_as("reception_app", "operational").unwrap();
    let mine = rows(&rec.execute_one("SHOW PRIVACY MATRIX").await.unwrap());
    assert!(!mine.is_empty() && mine.iter().all(|r| r[2].as_deref() == Some("reception_app")));
}
