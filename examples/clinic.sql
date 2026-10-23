-- The clinic from the DEP design proposal, for depsql on DataFusion.
-- Load it with:  dep-cli --init examples/clinic.sql
-- It runs as the superuser admin. Every role below has the password 'demo'.

-- Roles: one per application, job, and person
CREATE ROLE privacy_admin    LOGIN PASSWORD 'demo';
CREATE ROLE reception_app    LOGIN PASSWORD 'demo';
CREATE ROLE doctors_app      LOGIN PASSWORD 'demo';
CREATE ROLE reminder_job     LOGIN PASSWORD 'demo';
CREATE ROLE billing_job      LOGIN PASSWORD 'demo';
CREATE ROLE bi_dashboard     LOGIN PASSWORD 'demo';
CREATE ROLE scheduling_agent LOGIN PASSWORD 'demo';
CREATE ROLE app_developer    LOGIN PASSWORD 'demo';

-- Which role may connect through which endpoint (and so on which access path)
GRANT CONNECT ON ENDPOINT operational TO privacy_admin, reception_app, doctors_app, reminder_job, billing_job, app_developer;
GRANT CONNECT ON ENDPOINT analytics   TO bi_dashboard, scheduling_agent;

CREATE TABLE patients (
    patient_id  BIGINT PRIMARY KEY,
    national_id TEXT PROTECTED MATCH DOMAIN person_id UNIQUE,   -- unique, checked through blind index entries
    phone       TEXT PROTECTED MATCH DOMAIN phone_number,
    full_name   TEXT,
    health_fund TEXT
);

CREATE TABLE appointments (
    appointment_id BIGINT PRIMARY KEY,
    patient_id     BIGINT REFERENCES patients (patient_id),               -- link not sensitive: ordinary key
    starts_at      TIMESTAMP,
    visit_type     TEXT
);

CREATE TABLE medical_records (
    record_id           BIGINT PRIMARY KEY,
    patient_national_id TEXT PROTECTED REFERENCES patients (national_id),  -- link sensitive: protected key
    diagnosis           TEXT,
    recorded_at         DATE
);

-- Sample data (fictional)
INSERT INTO patients (patient_id, national_id, phone, full_name, health_fund) VALUES
    (1, '123456782', '+972-50-0000001', 'Dana Cohen',    'Clalit'),
    (2, '234567893', '+972-50-0000002', 'Yossi Levi',    'Maccabi'),
    (3, '345678904', '+972-50-0000003', 'Noa Mizrahi',   'Clalit'),
    (4, '456789015', '+972-50-0000004', 'Avi Peretz',    'Meuhedet'),
    (5, '567890126', '+972-50-0000005', 'Maya Friedman', 'Leumit'),
    (6, '678901237', '+972-50-0000006', 'Omer Biton',    'Maccabi');

INSERT INTO appointments (appointment_id, patient_id, starts_at, visit_type) VALUES
    (101, 1, '2026-10-21 10:30:00', 'Follow-up'),
    (102, 2, '2026-10-21 11:15:00', 'First visit'),
    (103, 3, '2026-10-22 09:00:00', 'Vaccination'),
    (104, 1, '2026-10-22 12:45:00', 'Lab results'),
    (105, 4, '2026-10-22 16:00:00', 'Follow-up'),
    (106, 5, '2026-10-18 10:00:00', 'First visit'),
    (107, 6, '2026-10-13 08:30:00', 'Follow-up'),
    (108, 2, '2026-10-09 14:00:00', 'Lab results'),
    (109, 3, '2026-10-01 09:45:00', 'Follow-up'),
    (110, 6, '2026-10-28 13:30:00', 'Vaccination');

INSERT INTO medical_records (record_id, patient_national_id, diagnosis, recorded_at) VALUES
    (201, '123456782', 'Asthma',           '2026-04-04'),
    (202, '123456782', 'Seasonal allergy', '2026-10-21'),
    (203, '234567893', 'Diabetes',         '2026-10-09'),
    (204, '345678904', 'Asthma',           '2026-10-01'),
    (205, '456789015', 'Hypertension',     '2026-08-22'),
    (206, '678901237', 'Diabetes',         '2026-10-13');

-- MATCH cells: privilege on exactly the protected columns each role may match on
GRANT SELECT (patient_id, national_id, full_name, health_fund) ON patients TO reception_app, doctors_app;
GRANT INSERT, UPDATE ON patients TO reception_app;
GRANT SELECT ON appointments TO reception_app, doctors_app;
GRANT SELECT, INSERT ON medical_records TO doctors_app;

-- Masked consumers read ordinary fields only
GRANT SELECT (patient_id, health_fund) ON patients TO bi_dashboard, scheduling_agent;
GRANT SELECT ON appointments TO bi_dashboard, scheduling_agent;

-- Who may declare functions, per match domain. Declared functions run with
-- their owner's privileges, so the privacy administrator also reads the tables.
GRANT DISCLOSE ON MATCH DOMAIN person_id    TO privacy_admin;
GRANT LINK     ON MATCH DOMAIN person_id    TO privacy_admin;
GRANT DISCLOSE ON MATCH DOMAIN phone_number TO privacy_admin;
GRANT SELECT ON patients, appointments, medical_records TO privacy_admin;

SET ROLE privacy_admin;

-- Reminders: phone x Reminders = DISCLOSE
CREATE FUNCTION appointment_reminders(day DATE)
RETURNS TABLE (phone TEXT, starts_at TIMESTAMP)
LANGUAGE sql
DISCLOSES phone_number
AS $$
    SELECT REVEAL(p.phone), a.starts_at
    FROM appointments AS a
    JOIN patients AS p USING (patient_id)
    WHERE CAST(a.starts_at AS DATE) = day
$$;

-- Visit report: national_id x Visit report = DISCLOSE
CREATE FUNCTION monthly_fund_report(report_month DATE)
RETURNS TABLE (national_id TEXT, visit_date DATE, visit_type TEXT, health_fund TEXT)
LANGUAGE sql
DISCLOSES person_id
AS $$
    SELECT REVEAL(p.national_id), CAST(a.starts_at AS DATE), a.visit_type, p.health_fund
    FROM appointments AS a
    JOIN patients AS p USING (patient_id)
    WHERE date_trunc('month', a.starts_at) = date_trunc('month', CAST(report_month AS TIMESTAMP))
$$;

-- Statistics: national_id and medical_records x Statistics = LINK
CREATE FUNCTION diagnoses_per_fund()
RETURNS TABLE (health_fund TEXT, diagnosis TEXT, patients BIGINT)
LANGUAGE sql
LINKS person_id
AS $$
    SELECT p.health_fund, m.diagnosis, COUNT(*)
    FROM patients AS p
    JOIN medical_records AS m ON m.patient_national_id = p.national_id
    GROUP BY p.health_fund, m.diagnosis
$$;

-- Claims: national_id x Claims = DISCLOSE + LINK
CREATE FUNCTION fund_claims_report(report_month DATE)
RETURNS TABLE (national_id TEXT, diagnosis TEXT, recorded_at DATE)
LANGUAGE sql
DISCLOSES person_id
LINKS person_id
AS $$
    SELECT REVEAL(p.national_id), m.diagnosis, m.recorded_at
    FROM medical_records AS m
    JOIN patients AS p ON p.national_id = m.patient_national_id
    WHERE date_trunc('month', CAST(m.recorded_at AS TIMESTAMP)) = date_trunc('month', CAST(report_month AS TIMESTAMP))
$$;

COMMENT ON FUNCTION appointment_reminders(DATE) IS 'Purpose: SMS appointment reminders. Owner: operations.';
COMMENT ON FUNCTION monthly_fund_report(DATE)   IS 'Purpose: monthly visit report to health funds. Owner: billing.';
COMMENT ON FUNCTION diagnoses_per_fund()        IS 'Purpose: diagnosis statistics per health fund. Owner: management.';
COMMENT ON FUNCTION fund_claims_report(DATE)    IS 'Purpose: monthly diagnosis claims to health funds. Owner: billing.';

-- DISCLOSE and LINK cells: who may run each declared function
GRANT EXECUTE ON FUNCTION appointment_reminders(DATE) TO reminder_job;
GRANT EXECUTE ON FUNCTION monthly_fund_report(DATE)   TO billing_job;
GRANT EXECUTE ON FUNCTION fund_claims_report(DATE)    TO billing_job;
GRANT EXECUTE ON FUNCTION diagnoses_per_fund()        TO bi_dashboard;

RESET ROLE;
