\c analytics
SELECT full_name, national_id, health_fund FROM patients ORDER BY patient_id LIMIT 3;
SELECT full_name FROM patients WHERE national_id = '123456782';
SELECT * FROM diagnoses_per_fund() ORDER BY 1, 2 LIMIT 4;
