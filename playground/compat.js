/* Compatibility test: DEP.js against DEP Engine.

   Runs the same statements on a DEP Engine server (through its HTTP API) and on DEP.js, and
   compares every result: rows, command tags, errors (SQLSTATE, DEP code, message, hint), and
   DEP notes. Both start from examples/clinic.sql.

     dep-engine --http-listen 127.0.0.1:8433 --init ../examples/clinic.sql --admin-password admin
     DEP_ENGINE_URL=http://127.0.0.1:8433 DEP_ADMIN_PASSWORD=admin node compat.js

   Each case is one HTTP request and one DEP.js connection: the statements run in order, stop at
   the first error, and a transaction left open is rolled back (as the HTTP API does). Rows are
   compared as a set unless the statement has ORDER BY. EXPLAIN is compared on its Read: lines,
   and SHOW AUDIT without its sequence numbers and times. */
'use strict';
const fs = require('fs');
const path = require('path');
const initSqlJs = require('sql.js');
const D = require('./depsql.js');

const URL_BASE = process.env.DEP_ENGINE_URL || 'http://127.0.0.1:8433';
const ADMIN_PW = process.env.DEP_ADMIN_PASSWORD || 'admin';
const ROLE_PW = process.env.DEP_ROLE_PASSWORD || 'demo';
const CLINIC = fs.readFileSync(path.join(__dirname, '..', 'examples', 'clinic.sql'), 'utf8');

const P = (role, ep, sql, params) => ({ role, ep, sql, params: params || [] });
const A = (sql, params) => P('admin', 'operational', sql, params);
const M = (sql) => P('admin', 'analytics', sql);

const CASES = [
  // reading, matching and anchoring
  A('SELECT full_name, national_id, health_fund FROM patients'),
  A("SELECT full_name, national_id FROM patients WHERE national_id = '123456782'"),
  A('SELECT full_name FROM patients WHERE national_id = $1', ['234567893']),
  A("SELECT full_name FROM patients WHERE national_id LIKE '1234%'"),
  A("SELECT full_name FROM patients WHERE national_id IN ('123456782', '234567893', '000000000')"),
  A("SELECT full_name FROM patients WHERE national_id NOT IN ('123456782')"),
  A("SELECT full_name FROM patients WHERE national_id = '123456782' OR national_id = '234567893'"),
  A("SELECT full_name FROM patients WHERE national_id = '123456782' OR full_name = 'Noa Mizrahi'"),
  A('SELECT full_name FROM patients WHERE national_id IS NULL'),
  A('SELECT full_name FROM patients WHERE phone IS NOT NULL ORDER BY full_name'),
  A("SELECT p.full_name, m.diagnosis FROM patients AS p JOIN medical_records AS m ON m.patient_national_id = p.national_id WHERE p.national_id = '123456782'"),
  A('SELECT p.full_name, m.diagnosis FROM patients AS p JOIN medical_records AS m ON m.patient_national_id = p.national_id'),
  A("SELECT p.full_name, m.diagnosis FROM patients AS p JOIN medical_records AS m ON m.patient_national_id = p.national_id AND p.national_id = '345678904'"),
  A("SELECT p.full_name, m.diagnosis FROM patients AS p LEFT JOIN medical_records AS m ON m.patient_national_id = p.national_id WHERE p.national_id IN ('123456782')"),
  A("SELECT p.full_name, m.diagnosis FROM patients AS p, medical_records AS m WHERE m.patient_national_id = p.national_id AND m.patient_national_id = '678901237'"),
  A("SELECT full_name FROM patients WHERE national_id IN (SELECT patient_national_id FROM medical_records WHERE patient_national_id = '123456782')"),
  A('SELECT full_name FROM patients WHERE national_id IN (SELECT patient_national_id FROM medical_records)'),
  A('SELECT full_name FROM patients p WHERE EXISTS (SELECT 1 FROM medical_records m WHERE m.patient_national_id = p.national_id)'),
  A("SELECT full_name FROM patients WHERE national_id = (SELECT patient_national_id FROM medical_records WHERE record_id = 201)"),
  A('SELECT national_id FROM patients ORDER BY national_id'),
  A('SELECT health_fund, national_id FROM patients GROUP BY health_fund, national_id'),
  A('SELECT DISTINCT national_id FROM patients'),
  A('SELECT count(national_id) FROM patients'),
  A('SELECT count(DISTINCT national_id) FROM patients'),
  A('SELECT CAST(national_id AS TEXT) FROM patients'),
  A('SELECT upper(national_id) FROM patients'),
  A('SELECT full_name FROM patients WHERE national_id = full_name'),
  A("SELECT full_name FROM patients WHERE national_id <> '1'"),
  A("SELECT full_name FROM patients WHERE phone = '+972-50-0000003'"),
  A('SELECT p.full_name FROM patients p JOIN patients q ON p.national_id = q.phone'),
  A('SELECT REVEAL(national_id) FROM patients'),
  A('WITH x AS (SELECT national_id, full_name FROM patients) SELECT * FROM x ORDER BY full_name'),
  A("SELECT s.full_name FROM (SELECT national_id, full_name FROM patients WHERE national_id = '123456782') AS s JOIN medical_records m ON m.patient_national_id = s.national_id"),
  A('SELECT full_name FROM patients UNION SELECT visit_type FROM appointments'),
  A('SELECT national_id FROM patients UNION SELECT patient_national_id FROM medical_records'),
  A('SELECT health_fund, count(*) AS n FROM patients GROUP BY health_fund ORDER BY health_fund'),
  A("SELECT visit_type, count(*) FROM appointments WHERE visit_type <> 'x' GROUP BY visit_type HAVING count(*) > 1 ORDER BY 1"),
  A('SELECT a.appointment_id, p.full_name FROM appointments a JOIN patients p USING (patient_id) WHERE a.appointment_id = 101'),
  A('SELECT p.full_name, a.visit_type FROM patients p RIGHT JOIN appointments a ON a.patient_id = p.patient_id WHERE a.appointment_id IN (101, 102) ORDER BY a.visit_type'),
  A('SELECT full_name, row_number() OVER (ORDER BY full_name) AS n FROM patients ORDER BY n LIMIT 3'),
  A("SELECT CASE WHEN national_id = '123456782' THEN 'yes' ELSE 'no' END AS hit FROM patients ORDER BY patient_id"),
  A('SELECT national_id__ct FROM patients'),
  A("SELECT dep_index('person_id', '1')"),
  A('SELECT nope FROM patients'),
  A('SELECT * FROM nothing_here'),
  A('SELECT patient_id + 1 AS next, upper(full_name) AS u FROM patients WHERE patient_id < 3 ORDER BY 1'),
  A('SELECT x.nope FROM patients'),
  A('SELECT p.nope FROM patients p'),
  A("WITH s AS (SELECT national_id FROM patients WHERE national_id = '123456782') SELECT m.diagnosis FROM s JOIN medical_records m ON m.patient_national_id = s.national_id ORDER BY 1"),
  A('WITH s AS (SELECT national_id FROM patients) SELECT m.diagnosis FROM s JOIN medical_records m ON m.patient_national_id = s.national_id'),
  A('SELECT (SELECT national_id FROM patients WHERE patient_id = 1) AS x'),
  A('SELECT full_name FROM patients ORDER BY patient_id LIMIT 2 OFFSET 1'),
  A("VALUES (1, 'a'), (2, 'b')"),
  A('SELECT health_fund FROM patients INTERSECT SELECT health_fund FROM patients WHERE patient_id < 3'),
  A('SELECT national_id IS NULL AS missing FROM patients ORDER BY patient_id'),
  A("SELECT national_id = '123456782' AS hit FROM patients ORDER BY patient_id"),
  A("SELECT count(*) FROM monthly_fund_report(DATE '2026-10-01') GROUP BY national_id"),
  A("SELECT national_id FROM monthly_fund_report(DATE '2026-10-01') ORDER BY national_id LIMIT 2"),
  // Terminal-rule regressions: keep native differences visible until the Engine is fixed.
  A("SELECT national_id FROM monthly_fund_report(DATE '2026-10-01') ORDER BY national_id"),
  A("SELECT national_id AS exposed_id FROM monthly_fund_report(DATE '2026-10-01') ORDER BY exposed_id"),
  A("SELECT national_id FROM monthly_fund_report(DATE '2026-10-01') ORDER BY 1"),
  A("SELECT * FROM monthly_fund_report(DATE '2026-10-01') WHERE national_id IS NULL"),
  A("SELECT max(national_id) FROM monthly_fund_report(DATE '2026-10-01')"),
  A('SELECT full_name, count(*) OVER (PARTITION BY national_id) FROM patients'),
  A('SELECT full_name FROM patients WHERE national_id IN (full_name)'),
  A('SELECT full_name FROM patients WHERE full_name IN (national_id)'),
  A("SELECT full_name FROM patients WHERE national_id = lower('123456782')"),
  A("SELECT full_name FROM patients WHERE national_id = '123456782' AND patient_id = 1"),
  A("SELECT p.full_name FROM patients p JOIN medical_records m ON m.patient_national_id = p.national_id JOIN patients q ON q.national_id = m.patient_national_id WHERE q.national_id = '123456782'"),
  A('SELECT p.full_name FROM patients p JOIN medical_records m ON m.patient_national_id = p.national_id AND m.record_id = 201'),
  A('SELECT 1 + 1, 7 / 2, 2.5 * 2'),
  A("SELECT 'a' || 'b', lower('AB'), upper(full_name) FROM patients WHERE patient_id = 1"),
  A("SELECT date_trunc('month', starts_at) AS m, count(*) AS n FROM appointments GROUP BY 1 ORDER BY 1"),
  A('SELECT EXTRACT(YEAR FROM recorded_at) AS y FROM medical_records WHERE record_id = 201'),
  A('SELECT recorded_at + 1 AS next FROM medical_records WHERE record_id = 201'),
  A("SELECT recorded_at + INTERVAL '1 month' AS a, starts_at + INTERVAL '2 hours' AS b FROM medical_records, appointments WHERE record_id = 201 AND appointment_id = 101"),
  A("SELECT appointment_id FROM appointments WHERE starts_at >= TIMESTAMP '2026-10-22 00:00:00' ORDER BY starts_at DESC"),
  A("SELECT appointment_id FROM appointments WHERE CAST(starts_at AS DATE) = DATE '2026-10-22' ORDER BY starts_at DESC"),
  // the masked read path
  M('SELECT full_name, national_id, health_fund FROM patients'),
  M("SELECT full_name FROM patients WHERE national_id = '123456782'"),
  M("SELECT full_name FROM patients WHERE national_id IN ('123456782')"),
  M('SELECT * FROM monthly_fund_report(current_date)'),
  M('SELECT * FROM diagnoses_per_fund()'),
  M("UPDATE patients SET full_name = 'x' WHERE patient_id = 1"),
  M('CREATE TABLE t_masked (a BIGINT)'),
  M('SHOW dep.path'),
  M('SET dep.path = OPERATIONAL'),
  // declared functions
  A('SELECT * FROM diagnoses_per_fund()'),
  A('SELECT * FROM monthly_fund_report(current_date)'),
  A("SELECT * FROM monthly_fund_report(DATE '2026-10-01') ORDER BY visit_date, visit_type"),
  A("SELECT * FROM fund_claims_report(DATE '2026-10-05') ORDER BY recorded_at"),
  A("SELECT * FROM appointment_reminders(DATE '2026-10-22') ORDER BY starts_at"),
  A("SELECT r.national_id, m.diagnosis FROM monthly_fund_report(DATE '2026-10-01') AS r JOIN medical_records AS m ON m.patient_national_id = r.national_id"),
  A("SELECT national_id FROM monthly_fund_report(DATE '2026-10-01') WHERE national_id = '123456782'"),
  A("SELECT upper(visit_type), national_id FROM monthly_fund_report(DATE '2026-10-01') ORDER BY national_id, 1"),
  A("SELECT count(national_id) FROM monthly_fund_report(DATE '2026-10-01')"),
  A('SELECT * FROM monthly_fund_report()'),
  A('SELECT * FROM no_such_function()'),
  A('SELECT diagnoses_per_fund()'),
  P('reception_app', 'operational', 'SELECT * FROM diagnoses_per_fund()'),
  P('bi_dashboard', 'analytics', 'SELECT * FROM diagnoses_per_fund()'),
  P('billing_job', 'operational', "SELECT * FROM monthly_fund_report(DATE '2026-10-01') ORDER BY visit_date, visit_type"),
  P('reminder_job', 'operational', "SELECT phone FROM appointment_reminders(DATE '2026-10-21') ORDER BY phone"),
  // privileges
  P('reception_app', 'operational', "SELECT full_name, national_id FROM patients WHERE national_id = '123456782'"),
  P('reception_app', 'operational', 'SELECT phone FROM patients'),
  P('reception_app', 'operational', 'SELECT * FROM patients'),
  P('reception_app', 'operational', "SELECT full_name FROM patients WHERE phone = '1'"),
  P('reception_app', 'operational', 'SELECT * FROM medical_records'),
  P('reception_app', 'operational', 'SELECT count(*) FROM medical_records'),
  P('reception_app', 'operational', 'SHOW PRIVACY MATRIX'),
  P('reception_app', 'operational', 'SHOW AUDIT'),
  P('reception_app', 'operational', 'SHOW STORAGE patients'),
  P('reception_app', 'operational', 'SET ROLE admin'),
  P('reception_app', 'operational', "UPDATE patients SET health_fund = 'Clalit' WHERE national_id = '345678904'"),
  P('reception_app', 'operational', 'DELETE FROM patients WHERE patient_id = 6'),
  P('doctors_app', 'operational', "INSERT INTO medical_records VALUES (210, '567890126', 'Flu', DATE '2026-10-20')"),
  P('doctors_app', 'operational', 'CREATE TABLE t_doc (a BIGINT)'),
  A('SET ROLE reception_app; SHOW role; SELECT phone FROM patients'),
  A('SET ROLE scheduling_agent'),
  A('SHOW session_authorization; SHOW TRANSACTION ISOLATION LEVEL; SHOW dep.path'),
  // writes and constraints
  A("INSERT INTO patients VALUES (7, '123456782', NULL, 'Dup', 'X')"),
  A("INSERT INTO patients VALUES (1, '999', NULL, 'Dup', 'X')"),
  A("INSERT INTO patients (national_id) VALUES ('999')"),
  A("INSERT INTO medical_records VALUES (300, '000000000', 'x', current_date)"),
  A("INSERT INTO appointments VALUES (300, 99, TIMESTAMP '2026-11-01 10:00:00', 'x')"),
  A('DELETE FROM patients WHERE patient_id = 1'),
  A("INSERT INTO patients VALUES (8, '811111118', '+972-50-0000008', 'Eight', 'Leumit'); SELECT full_name, national_id FROM patients WHERE national_id = '811111118'"),
  A("UPDATE patients SET phone = national_id WHERE patient_id = 8"),
  A("UPDATE patients SET full_name = national_id WHERE patient_id = 8"),
  A("UPDATE patients SET national_id = '811111118' WHERE patient_id = 1"),
  A("UPDATE patients SET national_id = '822222228' WHERE national_id = '811111118'; SELECT patient_id FROM patients WHERE national_id = '822222228'"),
  A('INSERT INTO patients (patient_id, full_name) VALUES (9)'),
  A("INSERT INTO patients (patient_id, full_name, nope) VALUES (9, 'a', 'b')"),
  A("INSERT INTO medical_records (record_id, patient_national_id, diagnosis) SELECT 400, national_id, 'Copy' FROM patients WHERE patient_id = 8"),
  A("INSERT INTO medical_records (record_id, patient_national_id, diagnosis) SELECT 401, phone, 'Copy' FROM patients WHERE patient_id = 8"),
  A("INSERT INTO appointments (appointment_id, visit_type) SELECT 402, national_id FROM patients WHERE patient_id = 8"),
  A('DELETE FROM medical_records WHERE record_id = 400; DELETE FROM patients WHERE patient_id = 8'),
  A("INSERT INTO patients VALUES (10, '100000001', NULL, 'Ten', 'X') ON CONFLICT DO NOTHING"),
  A("UPDATE patients SET full_name = 'Dana Cohen' WHERE national_id = '123456782'"),
  // transactions and savepoints
  A("BEGIN; SAVEPOINT a; UPDATE patients SET full_name = 'X' WHERE patient_id = 2; ROLLBACK TO SAVEPOINT a; SELECT full_name FROM patients WHERE patient_id = 2; RELEASE a; COMMIT"),
  A('SAVEPOINT a'),
  A('COMMIT'),
  A('ROLLBACK'),
  A('BEGIN; BEGIN; ROLLBACK'),
  A('BEGIN; ROLLBACK TO nope'),
  A("BEGIN; CREATE TABLE t_tx (id BIGINT PRIMARY KEY, nid TEXT PROTECTED MATCH DOMAIN person_id UNIQUE); INSERT INTO t_tx VALUES (1, '123456782'); SELECT id FROM t_tx WHERE nid = '123456782'; ROLLBACK; SELECT * FROM t_tx"),
  A("BEGIN; SAVEPOINT s; CREATE TABLE t_sp (a BIGINT); ROLLBACK TO s; SELECT * FROM t_sp"),
  A("BEGIN; INSERT INTO appointments VALUES (500, 1, TIMESTAMP '2026-12-01 09:00:00', 'Tx'); SAVEPOINT s; INSERT INTO appointments VALUES (501, 1, TIMESTAMP '2026-12-01 10:00:00', 'Tx'); ROLLBACK TO s; COMMIT; SELECT appointment_id FROM appointments WHERE visit_type = 'Tx' ORDER BY 1"),
  A("DELETE FROM appointments WHERE visit_type = 'Tx'"),
  // indexes and EXPLAIN
  A("EXPLAIN SELECT full_name FROM patients WHERE national_id = '123456782'"),
  A("EXPLAIN SELECT full_name FROM patients WHERE full_name = 'x'"),
  A('EXPLAIN SELECT full_name FROM patients WHERE patient_id IN (1, 2, 3)'),
  A('EXPLAIN SELECT full_name FROM patients WHERE patient_id = 1 OR patient_id = 2'),
  A("EXPLAIN SELECT p.full_name, m.diagnosis FROM patients p JOIN medical_records m ON m.patient_national_id = p.national_id WHERE p.national_id = '123456782'"),
  A("EXPLAIN UPDATE patients SET full_name = 'x'"),
  A('CREATE INDEX ON patients (health_fund); CREATE UNIQUE INDEX phone_u ON patients (phone); SHOW INDEXES FROM patients'),
  A("EXPLAIN SELECT full_name FROM patients WHERE health_fund = 'Clalit'"),
  A('DROP INDEX patients_national_id_key'),
  A('DROP INDEX patients_health_fund_idx; REINDEX TABLE patients; REINDEX INDEX phone_u; DROP INDEX phone_u'),
  A('DROP INDEX nope'),
  A('CREATE UNIQUE INDEX fund_u ON patients (health_fund)'),
  A('CREATE INDEX ON patients (lower(full_name))'),
  A('CREATE INDEX x ON patients (nope)'),
  A('REINDEX SCHEMA public'),
  // schema
  A("CREATE TABLE cards (holder TEXT PROTECTED MATCH DOMAIN person_id PRIMARY KEY, label TEXT DEFAULT 'x')"),
  A("CREATE TABLE cards (holder TEXT PROTECTED MATCH DOMAIN person_id PRIMARY KEY, label TEXT CHECK (label <> ''))"),
  A('CREATE TABLE cards (holder TEXT PROTECTED MATCH DOMAIN person_id PRIMARY KEY, label TEXT, FOREIGN KEY (label) REFERENCES patients (full_name))'),
  A("CREATE TABLE cards (holder TEXT PROTECTED MATCH DOMAIN person_id PRIMARY KEY, label TEXT NOT NULL); INSERT INTO cards VALUES ('123', 'a'); INSERT INTO cards VALUES ('123', 'b')"),
  A("INSERT INTO cards VALUES ('124', NULL)"),
  A('SHOW COLUMNS FROM cards; SHOW INDEXES FROM cards'),
  A('CREATE TABLE cards (a BIGINT)'),
  A('CREATE TABLE IF NOT EXISTS cards (a BIGINT)'),
  A('CREATE TABLE c3 (x INTEGER PROTECTED MATCH DOMAIN person_id)'),
  A('CREATE TABLE c3 (x TEXT PROTECTED)'),
  A('CREATE TABLE c3 (x TEXT REFERENCES patients (national_id))'),
  A('CREATE TABLE c3 (x TEXT PROTECTED REFERENCES patients (full_name))'),
  A('CREATE TABLE c3 (x TEXT PROTECTED MATCH DOMAIN phone_number REFERENCES patients (national_id))'),
  A('CREATE TABLE c3 (x BIGINT REFERENCES patients (patient_id) ON DELETE CASCADE)'),
  A('CREATE TABLE c3 (x MONEY)'),
  A('CREATE TABLE c3 AS SELECT 1'),
  A('CREATE VIEW v AS SELECT 1'),
  A('ALTER TABLE patients ADD COLUMN x TEXT'),
  A('DROP TABLE patients'),
  A('DROP TABLE cards; DROP TABLE nope'),
  A('DROP TABLE IF EXISTS nope'),
  // roles, grants and endpoints
  A("CREATE ROLE tester LOGIN PASSWORD 'demo'; GRANT CONNECT ON ENDPOINT analytics TO tester; SHOW ROLES"),
  A('CREATE ROLE tester'),
  A('GRANT SELECT (nope) ON patients TO tester'),
  A('GRANT SELECT ON nope TO tester'),
  A('GRANT SELECT ON patients TO nobody'),
  A('GRANT EXECUTE ON FUNCTION diagnoses_per_fund() TO public'),
  A('GRANT DISCLOSE ON MATCH DOMAIN person_id TO public'),
  A('GRANT DISCLOSE ON MATCH DOMAIN nope TO tester'),
  A('GRANT CONNECT ON ENDPOINT nope TO tester'),
  A('GRANT INSERT ON ENDPOINT analytics TO tester'),
  A('GRANT FLY ON patients TO tester'),
  A('ALTER ROLE tester MATCH LIMIT 2 PER MINUTE; SHOW ROLES'),
  A('ALTER ROLE tester MATCH LIMIT 2 PER FORTNIGHT'),
  A('ALTER ROLE admin NOSUPERUSER'),
  A('CREATE ENDPOINT reports MODE MASKED_READ; CREATE ENDPOINT reports MODE OPERATIONAL'),
  A('CREATE ENDPOINT bad MODE SOMETIMES'),
  A('SHOW ENDPOINTS; SHOW GRANTS'),
  A('SHOW PRIVACY MATRIX'),
  A('DROP ENDPOINT reports; REVOKE CONNECT ON ENDPOINT analytics FROM tester; DROP ROLE tester'),
  A('DROP ROLE admin'),
  A('DROP ROLE nope'),
  // match limits
  A("ALTER ROLE reception_app MATCH LIMIT 3 PER MINUTE"),
  P('reception_app', 'operational', "SELECT full_name FROM patients WHERE national_id IN ('123456782', '234567893')"),
  P('reception_app', 'operational', "SELECT full_name FROM patients WHERE national_id IN ('123456782', '234567893')"),
  A('ALTER ROLE reception_app MATCH LIMIT DEFAULT'),
  // functions: declaration rules
  A("CREATE FUNCTION f_lang() RETURNS TABLE (a TEXT) LANGUAGE plpgsql AS $$ SELECT 'a' $$"),
  A("CREATE FUNCTION diagnoses_per_fund() RETURNS TABLE (a TEXT) LANGUAGE sql LINKS person_id AS $$ SELECT 'a' $$"),
  A("CREATE FUNCTION f_scalar() RETURNS TEXT LANGUAGE sql AS $$ SELECT 'a' $$"),
  A("CREATE FUNCTION f_nodomain() RETURNS TABLE (a TEXT) LANGUAGE sql DISCLOSES nope AS $$ SELECT 'a' $$"),
  A('CREATE FUNCTION f_reveal() RETURNS TABLE (a TEXT) LANGUAGE sql AS $$ SELECT REVEAL(national_id) FROM patients $$'),
  A('CREATE FUNCTION f_count() RETURNS TABLE (a TEXT, b TEXT) LANGUAGE sql DISCLOSES person_id AS $$ SELECT REVEAL(national_id) FROM patients $$'),
  A("CREATE FUNCTION f_plain(n BIGINT) RETURNS TABLE (name TEXT) LANGUAGE sql AS $$ SELECT full_name FROM patients WHERE patient_id = n $$; SELECT * FROM f_plain(2); DROP FUNCTION f_plain"),
  A("CREATE FUNCTION f_lookup(id TEXT) RETURNS TABLE (name TEXT) LANGUAGE sql LINKS person_id AS $$ SELECT full_name FROM patients WHERE national_id = id $$; SHOW FUNCTIONS"),
  M("SELECT * FROM f_lookup('123456782')"),
  A("SELECT * FROM f_lookup('123456782')"),
  A("CREATE FUNCTION f_call() RETURNS TABLE (n TEXT) LANGUAGE sql DISCLOSES person_id AS $$ SELECT name FROM f_lookup('1') $$"),
  A('DROP FUNCTION f_lookup; DROP FUNCTION nope; DROP FUNCTION IF EXISTS nope'),
  P('app_developer', 'operational', "CREATE FUNCTION f_dev() RETURNS TABLE (a TEXT) LANGUAGE sql DISCLOSES person_id AS $$ SELECT 'a' $$"),
  P('app_developer', 'operational', "CREATE FUNCTION f_dev() RETURNS TABLE (a TEXT) LANGUAGE sql AS $$ SELECT full_name FROM patients $$"),
  P('app_developer', 'operational', "COMMENT ON FUNCTION diagnoses_per_fund() IS 'x'"),
  // the audit log
  A('SHOW AUDIT 12'),
  // key rotation
  A("BEGIN; ALTER SYSTEM ROTATE MASTER KEY"),
  A("ALTER SYSTEM ROTATE MASTER KEY; SELECT full_name FROM patients WHERE national_id = '345678904'; SELECT * FROM fund_claims_report(DATE '2026-10-05') ORDER BY recorded_at"),
  P('reception_app', 'operational', 'ALTER SYSTEM ROTATE MASTER KEY')
];

/* ---------- normalization */
const norm = (v) => (typeof v === 'string' ? v.replace(/^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2}:\d{2})(\.0+)?$/, '$1 $2') : v);
function normRows(stmt, rows) {
  const r = rows.map((row) => row.map(norm).map((v) => (typeof v === 'number' ? String(v) : v)));
  if (/^\s*SHOW\s+AUDIT/i.test(stmt)) return r.map((x) => x.slice(2));
  if (/^\s*EXPLAIN/i.test(stmt)) { const i = r.findIndex((x) => x[0] === 'Read:'); return i < 0 ? [] : r.slice(i).map((x) => x[0]).sort(); }
  if (/\border\s+by\b/i.test(stmt)) return r;
  return r.map((x) => JSON.stringify(x)).sort().map((x) => JSON.parse(x));
}
function view(stmt, x) {
  const o = { notes: x.notes || [] };
  if (x.error) o.error = { sqlstate: x.error.sqlstate, code: x.error.code || null, message: String(x.error.message).replace(/\s*Did you mean '[^']*'\?/g, ''), hint: x.error.hint || null };
  else if (x.command !== undefined) o.command = x.command;
  else { o.columns = /^\s*(EXPLAIN|SHOW\s+AUDIT)/i.test(stmt) ? null : x.columns; o.rows = normRows(stmt, x.rows || []); }
  if (/^\s*EXPLAIN/i.test(stmt)) o.notes = o.notes.slice();
  return o;
}

async function engine(c) {
  const pw = c.role === 'admin' ? ADMIN_PW : ROLE_PW;
  const res = await fetch(`${URL_BASE}/v1/query?endpoint=${c.ep}`, {
    method: 'POST', headers: { 'Content-Type': 'application/json', Authorization: 'Basic ' + Buffer.from(`${c.role}:${pw}`).toString('base64') },
    body: JSON.stringify({ sql: c.sql, params: c.params })
  });
  const j = await res.json();
  if (!j.results) return [{ statement: '(connect)', notes: [], error: j.error }];
  return j.results.map((r) => ({ statement: r.statement, ...view(r.statement, r) }));
}

function depjs(server, c) {
  let conn;
  try { conn = server.connect(c.role, c.ep); } catch (e) { return [{ statement: '(connect)', notes: [], error: { sqlstate: e.sqlstate, code: e.code || null, message: e.message, hint: e.hint || null } }]; }
  const out = [];
  for (const st of D.split(c.sql)) {
    const [r] = conn.exec(st, c.params);
    const x = r.ok ? (r.kind === 'rows' ? { columns: r.columns.map((k) => k.name), rows: r.rows, notes: r.notes } : { command: r.tag, notes: r.notes }) : { error: r.error, notes: r.notes };
    out.push({ statement: r.text, ...view(r.text, x) });
    if (!r.ok) break;
  }
  conn.close();
  return out;
}

(async () => {
  const SQL = await initSqlJs();
  const server = new D.Server(SQL, { setup: CLINIC });
  const bad = server.setupResults.filter((r) => !r.ok);
  if (bad.length) { console.error('DEP.js could not run clinic.sql:', bad.map((b) => b.error.message)); process.exit(2); }
  try { await fetch(`${URL_BASE}/v1/health`); } catch (e) { console.error(`No DEP Engine at ${URL_BASE}: ${e.message}`); process.exit(2); }
  let same = 0, diff = 0;
  for (const c of CASES) {
    const a = await engine(c), b = depjs(server, c);
    const n = Math.max(a.length, b.length);
    for (let i = 0; i < n; i++) {
      const x = a[i], y = b[i];
      const sx = JSON.stringify(x && { ...x, statement: undefined }), sy = JSON.stringify(y && { ...y, statement: undefined });
      if (sx === sy) { same++; continue; }
      diff++;
      console.log(`\nDIFF  ${c.role}@${c.ep}: ${(x || y).statement.replace(/\s+/g, ' ').slice(0, 140)}`);
      console.log('  engine:', sx);
      console.log('  dep.js:', sy);
    }
  }
  console.log(`\n${same} results the same, ${diff} different`);
  process.exit(diff ? 1 : 0);
})();
