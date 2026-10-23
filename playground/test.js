/* Tests of DEP.js. The rules themselves are compared with DEP Engine by compat.js; these cover the
   playground's setup and what one HTTP request cannot show: several connections at once
   (snapshots, the write lock, serialization failures), savepoints, and the audit log. */
'use strict';
const fs = require('fs');
const path = require('path');
const initSqlJs = require('sql.js');
const D = require('./depsql.js');
const C = require('./src/clinic.js');

let pass = 0, fail = 0;
function check(name, ok, info) {
  if (ok) pass++; else fail++;
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${!ok && info !== undefined ? '\n      ' + (typeof info === 'string' ? info : JSON.stringify(info)) : ''}`);
}
const last = (rs) => rs[rs.length - 1];
const err = (r) => (r.ok ? null : r.error.code || r.error.sqlstate);
const show = (r) => (r.ok ? (r.rows ? JSON.stringify(r.rows) : r.tag) : `${r.error.sqlstate} ${r.error.code || ''} ${r.error.message}`);

(async () => {
  const SQL = await initSqlJs();

  // The playground's clinic is examples/clinic.sql with its dates moved to today.
  {
    const file = fs.readFileSync(path.join(__dirname, '..', 'examples', 'clinic.sql'), 'utf8');
    const dates = (s) => s.replace(/'\d{4}-\d{2}-\d{2}( \d{2}:\d{2}:\d{2})?'/g, "'DATE'");
    check('the playground setup is examples/clinic.sql', dates(C.clinicSetup()) === dates(file));
  }

  const server = new D.Server(SQL, { setup: C.clinicSetup });
  const bad = server.setupResults.filter((r) => !r.ok);
  check(`setup: ${server.setupResults.length} statements run`, bad.length === 0, bad.map((b) => b.error.message + ' :: ' + b.text.slice(0, 80)));

  const op = server.connect('admin', 'operational');
  const op2 = server.connect('admin', 'operational');
  const masked = server.connect('admin', 'analytics');
  const reception = server.connect('reception_app', 'operational');

  // The rules, briefly (compat.js compares them with the engine in detail).
  {
    const r = last(reception.exec('SELECT full_name, national_id FROM patients WHERE national_id = $1', ['123456782']));
    check('a lookup by a PROTECTED value; the value comes back NULL', r.ok && r.rows.length === 1 && r.rows[0][0] === 'Dana Cohen' && r.rows[0][1] === null && r.columns[1].tag === 'protected', show(r));
    check('the lookup has the engine\'s DEP notes', r.notes.length === 2 && /matched by equality through the blind index of MATCH DOMAIN person_id/.test(r.notes[0]), r.notes);
    check('IN on a PROTECTED column is a list of lookups', last(op.exec("SELECT full_name FROM patients WHERE national_id IN ('123456782', '345678904')")).rows.length === 2);
    check('LIKE on a PROTECTED column is refused (ZD101)', last(op.exec("SELECT full_name FROM patients WHERE national_id LIKE '1%'")).error.sqlstate === 'ZD101');
    check('an unanchored join of PROTECTED columns is refused (ZD107)', err(last(op.exec('SELECT p.full_name FROM patients p JOIN medical_records m ON m.patient_national_id = p.national_id'))) === 'DEP_UNANCHORED_MATCH');
    check('the masked path does not match (ZD102)', err(last(masked.exec("SELECT full_name FROM patients WHERE national_id = '123456782'"))) === 'DEP_ENDPOINT_OPERATION');
    check('a linking function runs on the masked path', last(masked.exec('SELECT * FROM diagnoses_per_fund()')).ok);
    check('current_date as a function argument', last(op.exec('SELECT * FROM monthly_fund_report(current_date)')).ok);
    const ex = last(op.exec("EXPLAIN SELECT full_name FROM patients WHERE national_id = '123456782'"));
    check('EXPLAIN shows an index lookup', ex.ok && ex.rows.some((x) => x[0] === '  patients: index patients_national_id_key, 1 key(s)'), show(ex));
  }

  // Constraints: the value of an ordinary key is in the message, a PROTECTED one never is.
  {
    const a = last(op.exec("INSERT INTO patients (patient_id, full_name) VALUES (1, 'Again')"));
    check('primary key (23505) shows the ordinary key', a.error && a.error.sqlstate === '23505' && a.error.message.includes('Key (patient_id)=(1)'), show(a));
    const b = last(op.exec("INSERT INTO patients (patient_id, national_id, full_name) VALUES (7, '123456782', 'Twin')"));
    check('unique PROTECTED column (23505) without its value', b.error && b.error.sqlstate === '23505' && !b.error.message.includes('123456782') && b.error.message.includes('Key (national_id)'), show(b));
    const c = last(op.exec("INSERT INTO medical_records VALUES (300, '000000000', 'x', NULL)"));
    check('PROTECTED reference (23503) without its value', c.error && c.error.sqlstate === '23503' && !c.error.message.includes('000000000'), show(c));
    const d = last(op.exec('DELETE FROM patients WHERE patient_id = 1'));
    check('a referenced row cannot be deleted (23503)', d.error && d.error.sqlstate === '23503' && d.error.message.includes('still referenced from table "appointments"'), show(d));
    check('a failed statement wrote nothing', last(op.exec('SELECT count(*) FROM patients')).rows[0][0] === 6);
  }

  // Transactions: other connections read the last commit; one writer at a time.
  {
    op.exec('BEGIN');
    op.exec("UPDATE patients SET full_name = 'Dana C.' WHERE patient_id = 1");
    check('the writer sees its own change', last(op.exec('SELECT full_name FROM patients WHERE patient_id = 1')).rows[0][0] === 'Dana C.');
    check('another connection reads the last commit', last(op2.exec('SELECT full_name FROM patients WHERE patient_id = 1')).rows[0][0] === 'Dana Cohen');
    check('the masked path reads the last commit too', last(masked.exec('SELECT full_name FROM patients WHERE patient_id = 1')).rows[0][0] === 'Dana Cohen');
    const w = last(op2.exec("UPDATE appointments SET visit_type = 'x' WHERE appointment_id = 101"));
    check('a second writer waits for the lock (55P03 here, at once)', w.error && w.error.sqlstate === '55P03', show(w));
    op.exec('COMMIT');
    check('after COMMIT everyone reads the change', last(op2.exec('SELECT full_name FROM patients WHERE patient_id = 1')).rows[0][0] === 'Dana C.');
    op.exec("UPDATE patients SET full_name = 'Dana Cohen' WHERE patient_id = 1");
  }
  {
    op2.exec('BEGIN');
    op2.exec('SELECT count(*) FROM patients');
    op.exec("UPDATE patients SET health_fund = 'Leumit' WHERE patient_id = 5");
    const r = last(op2.exec("UPDATE patients SET health_fund = 'Clalit' WHERE patient_id = 5"));
    check('a table changed after BEGIN cannot be written (40001)', r.error && r.error.sqlstate === '40001', show(r));
    const r2 = last(op2.exec('SELECT 1'));
    check('after an error the transaction is aborted (25P02)', r2.error && r2.error.sqlstate === '25P02', show(r2));
    const r3 = last(op2.exec('COMMIT'));
    check('COMMIT of a failed transaction rolls it back', r3.ok && r3.tag === 'ROLLBACK' && r3.notes[0] === 'The transaction had failed, so it was rolled back.', show(r3));
  }
  {
    const rs = op.exec("BEGIN; INSERT INTO appointments VALUES (500, 1, '2026-12-01 09:00:00', 'Tx'); SAVEPOINT a; INSERT INTO appointments VALUES (501, 1, '2026-12-01 10:00:00', 'Tx'); SELECT 1/0 + nope FROM appointments; ROLLBACK TO a; COMMIT");
    check('ROLLBACK TO a savepoint ends the failed state', rs.every((r, i) => (i === 4 ? !r.ok : r.ok)), rs.map(show));
    check('and undoes only what came after the savepoint', JSON.stringify(last(op.exec("SELECT appointment_id FROM appointments WHERE visit_type = 'Tx'")).rows) === '[[500]]');
    op.exec("DELETE FROM appointments WHERE visit_type = 'Tx'");
  }
  {
    const rs = op.exec("BEGIN; CREATE TABLE notes (id BIGINT PRIMARY KEY, holder TEXT PROTECTED MATCH DOMAIN person_id); INSERT INTO notes VALUES (1, '123456782'); SAVEPOINT s; CREATE UNIQUE INDEX notes_u ON notes (holder); ROLLBACK TO s; SHOW INDEXES FROM notes");
    check('schema changes run in the transaction', rs.every((r) => r.ok), rs.map(show));
    check('ROLLBACK TO undoes a CREATE INDEX', last(rs).rows.length === 2, show(last(rs)));
    check('other connections do not see a table created in an open transaction', last(op2.exec('SELECT * FROM notes')).error.sqlstate === '42P01');
    op.exec('ROLLBACK');
    check('ROLLBACK removes the table', last(op.exec('SELECT * FROM notes')).error.sqlstate === '42P01');
  }
  {
    const c = server.connect('admin', 'operational');
    c.exec("BEGIN; UPDATE patients SET full_name = 'Gone' WHERE patient_id = 2");
    c.close();
    check('closing a connection rolls back its transaction and releases the lock', last(op.exec('SELECT full_name FROM patients WHERE patient_id = 2')).rows[0][0] === 'Yossi Levi' && last(op.exec("UPDATE patients SET full_name = 'Yossi Levi' WHERE patient_id = 2")).ok);
  }

  // Roles, match limits and the audit log.
  {
    op.exec('ALTER ROLE reception_app MATCH LIMIT 2 PER MINUTE');
    const r = last(reception.exec("SELECT full_name FROM patients WHERE national_id IN ('123456782', '234567893', '345678904')"));
    check('a match limit refuses a statement that would pass it (ZD109)', err(r) === 'DEP_MATCH_RATE_LIMITED' && /has made \d+ protected matches of the 2 PER MINUTE/.test(r.error.message), show(r));
    op.exec('ALTER ROLE reception_app MATCH LIMIT DEFAULT');
    const audit = last(op.exec('SHOW AUDIT 5'));
    const kinds = audit.rows.map((x) => x[6]);
    check('SHOW AUDIT has the engine\'s columns', audit.columns.map((c) => c.name).join(',') === 'seq,at,session_user,role,endpoint,path,kind,object,domain,count,detail');
    check('the refusal is audited, without values', kinds.includes('rate_limited') && !JSON.stringify(audit.rows).includes('123456782'), audit.rows);
    check('SET ROLE checks the endpoint', err(last(masked.exec('SET ROLE reception_app'))) === '42501');
    check('a role without the grant cannot connect', (() => { try { server.connect('reception_app', 'analytics'); return false; } catch (e) { return e.sqlstate === '28000'; } })());
  }

  // Key rotation: every protected value is indexed and encrypted again, and lookups still work.
  {
    const before = server.storage('patients').rows[0][1];
    const r = last(op.exec('ALTER SYSTEM ROTATE MASTER KEY'));
    const after = server.storage('patients').rows[0][1];
    check('key rotation re-indexes the stored values', r.ok && before !== after && /protected values were indexed and encrypted again/.test(r.notes[0]), show(r));
    check('lookups and disclosures work with the new key', last(op.exec("SELECT full_name FROM patients WHERE national_id = '123456782'")).rows.length === 1 && last(op.exec('SELECT national_id FROM monthly_fund_report(current_date)')).ok);
  }

  console.log(`\n${pass} passed, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})();
