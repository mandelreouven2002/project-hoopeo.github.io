//! Roles, endpoints, and privileges, with the clinic's roles.

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

async fn run(s: &Arc<DepServer>, user: &str, ep: &str, sql: &str) -> dep_engine::Result<Output> {
    let c = s.connect_as(user, ep).unwrap();
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

/// The DEP error code, or the SQLSTATE of an ordinary error, or OK.
fn code(r: &dep_engine::Result<Output>) -> String {
    match r {
        Err(e) => e.code.map(|c| c.to_string()).unwrap_or_else(|| e.sqlstate.clone()),
        Ok(_) => "OK".into(),
    }
}

const OP: &str = "operational";
const MASKED: &str = "analytics";

#[tokio::test]
async fn endpoints_decide_who_connects_where() {
    let s = clinic().await;
    assert!(s.connect_as("reception_app", OP).is_ok());
    assert_eq!(s.connect_as("reception_app", MASKED).err().unwrap().sqlstate, "28000");
    assert!(s.connect_as("bi_dashboard", MASKED).is_ok());
    assert_eq!(s.connect_as("bi_dashboard", OP).err().unwrap().sqlstate, "28000");
    assert_eq!(s.connect_as("nobody", OP).err().unwrap().sqlstate, "28000");
}

#[tokio::test]
async fn passwords() {
    let s = clinic().await;
    assert!(s.check_password("reception_app", "demo"));
    assert!(!s.check_password("reception_app", "wrong"));
    assert!(!s.check_password("admin", "demo"));
    // A role may change its own password, and nothing else about itself.
    assert_eq!(code(&run(&s, "reception_app", OP, "ALTER ROLE reception_app PASSWORD 'new'").await), "OK");
    assert!(s.check_password("reception_app", "new"));
    assert_eq!(code(&run(&s, "reception_app", OP, "ALTER ROLE reception_app SUPERUSER").await), "42501");
    assert_eq!(code(&run(&s, "reception_app", OP, "CREATE ROLE x LOGIN").await), "42501");
}

#[tokio::test]
async fn column_privilege_is_the_privilege_to_match() {
    let s = clinic().await;
    let o = run(&s, "reception_app", OP, "SELECT full_name, national_id FROM patients WHERE national_id = '123456782'").await.unwrap();
    assert_eq!(rows(&o), vec![vec![Some("Dana Cohen".into()), None]]);
    // Reception holds no privilege on phone, so it cannot look patients up by phone.
    let r = run(&s, "reception_app", OP, "SELECT full_name FROM patients WHERE phone = '+972-50-0000001'").await;
    assert_eq!(code(&r), "42501");
    assert!(r.unwrap_err().message.contains("column phone of table patients"));
    // SELECT * names every column, phone included.
    assert_eq!(code(&run(&s, "reception_app", OP, "SELECT * FROM patients").await), "42501");
    // Doctors may join diagnoses to the patient in the room.
    let join = "SELECT m.diagnosis FROM patients AS p JOIN medical_records AS m ON m.patient_national_id = p.national_id WHERE p.national_id = '123456782'";
    assert_eq!(rows(&run(&s, "doctors_app", OP, join).await.unwrap()).len(), 2);
    assert_eq!(code(&run(&s, "reception_app", OP, join).await), "42501");
}

#[tokio::test]
async fn dep_rules_come_before_privileges() {
    let s = clinic().await;
    // The masked path refuses the match whatever the role's privileges.
    let r = run(&s, "bi_dashboard", MASKED, "SELECT health_fund FROM patients WHERE national_id = '123456782'").await;
    assert_eq!(code(&r), "DEP_ENDPOINT_OPERATION");
    let r = run(&s, "bi_dashboard", MASKED, "SELECT national_id FROM patients").await;
    assert_eq!(code(&r), "42501");
    let o = run(&s, "bi_dashboard", MASKED, "SELECT health_fund, count(*) AS n FROM patients GROUP BY health_fund ORDER BY health_fund").await.unwrap();
    assert_eq!(rows(&o).len(), 4);
}

#[tokio::test]
async fn execute_on_declared_functions() {
    let s = clinic().await;
    assert_eq!(rows(&run(&s, "bi_dashboard", MASKED, "SELECT * FROM diagnoses_per_fund()").await.unwrap()).len(), 4);
    assert_eq!(code(&run(&s, "scheduling_agent", MASKED, "SELECT * FROM diagnoses_per_fund()").await), "42501");
    // The billing job reads no table, and still receives the report: the body runs with the owner's privileges.
    let o = run(&s, "billing_job", OP, "SELECT * FROM monthly_fund_report(DATE '2026-10-01')").await.unwrap();
    assert_eq!(rows(&o).len(), 10);
    assert_eq!(code(&run(&s, "billing_job", OP, "SELECT full_name FROM patients").await), "42501");
    assert_eq!(code(&run(&s, "reception_app", OP, "SELECT * FROM monthly_fund_report(DATE '2026-10-01')").await), "42501");
    assert_eq!(code(&run(&s, "bi_dashboard", MASKED, "SELECT * FROM monthly_fund_report(DATE '2026-10-01')").await), "DEP_ENDPOINT_OPERATION");
}

#[tokio::test]
async fn owners_privileges_are_checked_at_each_call() {
    let s = clinic().await;
    let call = "SELECT * FROM fund_claims_report(DATE '2026-10-01')";
    assert_eq!(code(&run(&s, "billing_job", OP, call).await), "OK");
    run(&s, "admin", OP, "REVOKE SELECT ON medical_records FROM privacy_admin").await.unwrap();
    let r = run(&s, "billing_job", OP, call).await;
    assert_eq!(code(&r), "42501");
    assert!(r.unwrap_err().message.contains("privacy_admin"));
    // The other report does not read medical_records.
    assert_eq!(code(&run(&s, "billing_job", OP, "SELECT * FROM monthly_fund_report(DATE '2026-10-01')").await), "OK");
}

#[tokio::test]
async fn declaring_functions() {
    let s = clinic().await;
    let f = "CREATE FUNCTION leak() RETURNS TABLE (n TEXT) LANGUAGE sql DISCLOSES person_id AS $$ SELECT REVEAL(national_id) FROM patients $$";
    assert_eq!(code(&run(&s, "app_developer", OP, f).await), "DEP_DECLARATION_PRIVILEGE");
    let g = "CREATE OR REPLACE FUNCTION diagnoses_per_fund() RETURNS TABLE (health_fund TEXT, diagnosis TEXT, patients BIGINT) LANGUAGE sql AS $$ SELECT 'x', 'y', 1 $$";
    assert_eq!(code(&run(&s, "app_developer", OP, g).await), "42501");
    let p = "GRANT EXECUTE ON FUNCTION diagnoses_per_fund() TO PUBLIC";
    assert_eq!(code(&run(&s, "privacy_admin", OP, p).await), "DEP_PUBLIC_EXECUTE");
    assert_eq!(code(&run(&s, "app_developer", OP, "GRANT EXECUTE ON FUNCTION diagnoses_per_fund() TO app_developer").await), "42501");
    // The privacy administrator may declare a new disclosing function and grant it.
    let h = "CREATE FUNCTION patient_contact(id TEXT) RETURNS TABLE (full_name TEXT, phone TEXT) LANGUAGE sql DISCLOSES phone_number AS $$
        SELECT p.full_name, REVEAL(p.phone) FROM patients AS p WHERE p.national_id = id $$;
        GRANT EXECUTE ON FUNCTION patient_contact(TEXT) TO reception_app";
    assert_eq!(code(&run(&s, "privacy_admin", OP, h).await), "OK");
    let o = run(&s, "reception_app", OP, "SELECT * FROM patient_contact('123456782')").await.unwrap();
    assert_eq!(rows(&o)[0][1].as_deref(), Some("+972-50-0000001"));
}

#[tokio::test]
async fn writes_need_their_privileges() {
    let s = clinic().await;
    let up = "UPDATE patients SET full_name = 'Dana Cohen-Levi' WHERE national_id = '123456782'";
    assert_eq!(code(&run(&s, "reception_app", OP, up).await), "OK");
    assert_eq!(code(&run(&s, "doctors_app", OP, up).await), "42501");
    assert_eq!(code(&run(&s, "reception_app", OP, "DELETE FROM patients WHERE national_id = '123456782'").await), "42501");
    assert_eq!(code(&run(&s, "app_developer", OP, "CREATE TABLE t (a TEXT)").await), "42501");
    let ins = "INSERT INTO medical_records (record_id, patient_national_id, diagnosis, recorded_at) VALUES (207, '123456782', 'Flu', DATE '2026-10-21')";
    assert_eq!(code(&run(&s, "doctors_app", OP, ins).await), "OK");
    assert_eq!(code(&run(&s, "reception_app", OP, ins).await), "42501");
}

#[tokio::test]
async fn set_role() {
    let s = clinic().await;
    assert_eq!(code(&run(&s, "reception_app", OP, "SET ROLE admin").await), "42501");
    let c = s.connect("operational").unwrap();
    c.execute_one("SET ROLE reception_app").await.unwrap();
    assert_eq!(code(&c.execute_one("SELECT phone FROM patients").await), "42501");
    c.execute_one("RESET ROLE").await.unwrap();
    assert_eq!(code(&c.execute_one("SELECT phone FROM patients").await), "OK");
    // A role may only act on endpoints it may connect through.
    assert_eq!(code(&c.execute_one("SET ROLE bi_dashboard").await), "42501");
}
