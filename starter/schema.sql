-- Synthetic local tutorial, DEP Engine 0.5.2. Run once as admin in a new database.
CREATE TABLE patients (
  patient_id BIGINT PRIMARY KEY,
  national_id TEXT PROTECTED MATCH DOMAIN person_id UNIQUE,
  full_name TEXT
);
CREATE TABLE records (
  record_id BIGINT PRIMARY KEY,
  patient_national_id TEXT PROTECTED REFERENCES patients (national_id),
  diagnosis TEXT
);
INSERT INTO patients VALUES (1, '123456782', 'Dana Cohen');
INSERT INTO records VALUES (1, '123456782', 'Synthetic example');

CREATE ROLE demo_app LOGIN PASSWORD 'demo-local';
GRANT CONNECT ON ENDPOINT operational TO demo_app;
GRANT SELECT (patient_id, full_name, national_id) ON patients TO demo_app;
GRANT SELECT ON records TO demo_app;
ALTER ROLE demo_app MATCH LIMIT 600 PER HOUR;

CREATE ROLE demo_analytics LOGIN PASSWORD 'demo-local';
GRANT CONNECT ON ENDPOINT analytics TO demo_analytics;
GRANT SELECT (patient_id, full_name, national_id) ON patients TO demo_analytics;

CREATE ROLE privacy_owner NOLOGIN;
CREATE ROLE export_job LOGIN PASSWORD 'export-local';
GRANT CONNECT ON ENDPOINT operational TO export_job;
GRANT CONNECT ON ENDPOINT operational TO privacy_owner;
GRANT SELECT ON patients TO privacy_owner;
GRANT DISCLOSE ON MATCH DOMAIN person_id TO privacy_owner;
SET ROLE privacy_owner;
CREATE FUNCTION patient_identifier(id BIGINT)
RETURNS TABLE (national_id TEXT)
LANGUAGE sql
DISCLOSES person_id
AS $$
  SELECT REVEAL(p.national_id) FROM patients AS p WHERE p.patient_id = id
$$;
COMMENT ON FUNCTION patient_identifier(BIGINT)
IS 'Purpose: approved single-patient identifier export for this local demo.';
GRANT EXECUTE ON FUNCTION patient_identifier(BIGINT) TO export_job;
RESET ROLE;
