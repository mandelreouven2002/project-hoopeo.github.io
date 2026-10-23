\echo ===== 1. Reception: lookups by ID, and nothing more =====
\c operational reception_app

-- Find a patient by the ID number she presents. Expected: one row, national_id NULL.
SELECT full_name, national_id, health_fund
FROM patients
WHERE national_id = '123456782';

-- The whole register. Expected: names, and NULL in place of every ID number.
SELECT full_name, national_id
FROM patients;

-- Reception holds no privilege on phone, so it cannot look patients up by phone.
-- Expected: permission denied for column phone.
SELECT full_name
FROM patients
WHERE phone = '+972-50-0000001';

-- The only operation on a protected value is full-value equality.
-- Expected: DEP_PROTECTED_OPERATION.
SELECT full_name
FROM patients
WHERE national_id LIKE '1234%';

-- So is a cast. Expected: DEP_PROTECTED_OPERATION.
SELECT CAST(national_id AS TEXT)
FROM patients;

-- Writes on the operational path. Expected: UPDATE 1, then the new name.
UPDATE patients SET full_name = 'Dana Cohen-Levi' WHERE national_id = '123456782';
SELECT full_name FROM patients WHERE national_id = '123456782';

-- A new patient. Expected: INSERT 0 1, then one row.
INSERT INTO patients (patient_id, national_id, phone, full_name, health_fund)
VALUES (7, '789012348', '+972-50-0000007', 'Lior Katz', 'Clalit');
SELECT full_name, health_fund FROM patients WHERE national_id = '789012348';

-- Reception may not delete. Expected: permission denied for table patients.
DELETE FROM patients WHERE national_id = '789012348';
