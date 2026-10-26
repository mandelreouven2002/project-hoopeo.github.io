BEGIN;
UPDATE patients SET health_fund = 'Maccabi' WHERE national_id = '123456782';
SAVEPOINT before_insert;
INSERT INTO patients (patient_id, national_id, full_name, health_fund) VALUES (7, '123456782', 'Someone Else', 'Clalit');
ROLLBACK TO SAVEPOINT before_insert;
COMMIT;
