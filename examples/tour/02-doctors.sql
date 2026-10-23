\echo ===== 2. Doctors: one patient's records, never everyone's =====
\c operational doctors_app

-- A join anchored to one ID number. Expected: Dana's two diagnoses.
SELECT p.full_name, m.diagnosis, m.recorded_at
FROM patients AS p
JOIN medical_records AS m ON m.patient_national_id = p.national_id
WHERE p.national_id = '123456782';

-- The same join for every patient. Expected: DEP_UNANCHORED_MATCH.
SELECT p.full_name, m.diagnosis
FROM patients AS p
JOIN medical_records AS m ON m.patient_national_id = p.national_id;

-- An anchor under OR is not an anchor. Expected: DEP_UNANCHORED_MATCH.
SELECT m.diagnosis
FROM patients AS p
JOIN medical_records AS m ON m.patient_national_id = p.national_id
WHERE p.national_id = '123456782' OR p.full_name = 'Yossi Levi';

-- A subquery is a join written differently. Expected: DEP_UNANCHORED_MATCH.
SELECT full_name
FROM patients
WHERE national_id IN (SELECT patient_national_id FROM medical_records);

-- Grouping by a protected column links every row that shares a value.
-- Expected: DEP_UNANCHORED_MATCH.
SELECT patient_national_id, count(*)
FROM medical_records
GROUP BY patient_national_id;

-- A list of ID numbers is a set of lookups. Expected: two rows.
SELECT full_name
FROM patients
WHERE national_id IN ('123456782', '234567893');

-- A new diagnosis for the patient in the room. Expected: INSERT 0 1.
INSERT INTO medical_records (record_id, patient_national_id, diagnosis, recorded_at)
VALUES (207, '123456782', 'Influenza', DATE '2026-10-21');
