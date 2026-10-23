\echo ===== 6. Administrator: storage, plans, and privileges =====
\c operational admin

-- Roles, the endpoints they may use, and every grant.
\du
\dp

-- What the server stores: keyed index entries and ciphertexts, never the values.
\storage patients

-- The plan after the DEP rewrite: the ID number becomes an index entry.
EXPLAIN SELECT full_name FROM patients WHERE national_id = '123456782';

-- A superuser is bound by the DEP rules too. Expected: DEP_UNANCHORED_MATCH.
SELECT p.full_name, m.diagnosis
FROM patients AS p
JOIN medical_records AS m ON m.patient_national_id = p.national_id;

-- A function runs with its owner's privileges, checked at every call.
-- Take SELECT on medical_records away from the owner of fund_claims_report.
REVOKE SELECT ON medical_records FROM privacy_admin;
\c operational billing_job
-- Expected: permission denied for function fund_claims_report: its owner can no longer read it.
SELECT * FROM fund_claims_report(DATE '2026-10-01');
-- The other report does not read medical_records. Expected: rows.
SELECT count(*) AS visits FROM monthly_fund_report(DATE '2026-10-01');

\c operational admin
GRANT SELECT ON medical_records TO privacy_admin;

-- SET ROLE: act as reception without reconnecting. Expected: permission denied for column phone.
SET ROLE reception_app;
SHOW ROLE;
SELECT phone FROM patients;
RESET ROLE;
SHOW ROLE;
