\c operational reception_app
SELECT patient_id, full_name, national_id, health_fund FROM patients WHERE national_id = '123456782';
SELECT full_name, national_id FROM patients;
SELECT full_name FROM patients WHERE national_id LIKE '12%';
\c operational doctors_app
SELECT m.diagnosis, m.recorded_at FROM patients AS p JOIN medical_records AS m ON m.patient_national_id = p.national_id WHERE p.national_id = '123456782' ORDER BY m.recorded_at;
SELECT p.full_name, m.diagnosis FROM patients AS p JOIN medical_records AS m ON m.patient_national_id = p.national_id;
\c analytics bi_dashboard
SELECT health_fund, count(*) AS patients FROM patients GROUP BY health_fund ORDER BY 1;
SELECT patient_id FROM patients WHERE national_id = '123456782';
SELECT * FROM diagnoses_per_fund() ORDER BY 1, 2;
\c operational billing_job
SELECT * FROM monthly_fund_report(DATE '2026-10-01') ORDER BY visit_date LIMIT 3;
SELECT r.national_id FROM monthly_fund_report(DATE '2026-10-01') AS r JOIN patients AS p ON p.national_id = r.national_id;
