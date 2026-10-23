\echo ===== 4. Billing: the one place ID numbers leave =====
\c operational billing_job

-- A declared DISCLOSES function. Expected: ID numbers in clear, for this month's visits.
SELECT * FROM monthly_fund_report(DATE '2026-10-01');

-- Disclosed values are terminal: they cannot be joined back to diagnoses.
-- Expected: DEP_DISCLOSED_VALUE_USE.
SELECT r.national_id, m.diagnosis
FROM monthly_fund_report(DATE '2026-10-01') AS r
JOIN medical_records AS m ON m.patient_national_id = r.national_id;

-- Nor used to filter. Expected: DEP_DISCLOSED_VALUE_USE.
SELECT *
FROM monthly_fund_report(DATE '2026-10-01') AS r
WHERE r.national_id = '123456782';

-- The billing job reads no table directly. Expected: permission denied for column full_name.
SELECT full_name FROM patients;
