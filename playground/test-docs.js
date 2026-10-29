/* Verify the downloadable tutorial's schema and privacy boundaries with DEP.js.
   Native Engine integration is deliberately separate: starter/python/verify.py. */
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const init = require('sql.js');
const D = require('./depsql.js');
(async () => {
  const SQL = await init();
  const setup = fs.readFileSync(path.join(__dirname, '../starter/schema.sql'), 'utf8');
  const server = new D.Server(SQL, { setup: () => setup });
  assert.deepEqual(server.setupResults.filter((r) => !r.ok), [], 'starter schema must initialize');
  const app = server.connect('demo_app', 'operational');
  const last = (conn, sql, params) => conn.exec(sql, params).at(-1);
  assert.deepEqual(last(app, 'SELECT full_name, national_id FROM patients WHERE national_id = $1', ['123456782']).rows, [['Dana Cohen', null]]);
  assert.equal(last(app, "SELECT full_name FROM patients WHERE national_id LIKE '123%'").error.sqlstate, 'ZD101');
  assert.equal(last(app, 'SELECT p.full_name, r.diagnosis FROM patients p JOIN records r ON r.patient_national_id = p.national_id').error.sqlstate, 'ZD107');
  assert.deepEqual(last(app, "SELECT p.full_name, r.diagnosis FROM patients p JOIN records r ON r.patient_national_id = p.national_id WHERE p.national_id = '123456782'").rows, [['Dana Cohen', 'Synthetic example']]);
  assert.equal(last(app, 'SELECT * FROM patient_identifier(1)').error.sqlstate, '42501');
  const analytics = server.connect('demo_analytics', 'analytics');
  assert.equal(last(analytics, "SELECT full_name FROM patients WHERE national_id = '123456782'").error.sqlstate, 'ZD102');
  const exporter = server.connect('export_job', 'operational');
  assert.deepEqual(last(exporter, 'SELECT * FROM patient_identifier(1)').rows, [['123456782']]);
  assert.equal(last(exporter, 'SELECT * FROM patient_identifier(1) ORDER BY national_id').error.sqlstate, 'ZD104');
  assert.equal(last(exporter, 'SELECT national_id AS exposed_id FROM patient_identifier(1) ORDER BY exposed_id').error.sqlstate, 'ZD104');
  assert.equal(last(exporter, 'SELECT national_id FROM patient_identifier(1) ORDER BY 1').error.sqlstate, 'ZD104');
  console.log('PASS: tutorial schema plus 10 application, grant and privacy checks (DEP.js).');
})().catch((error) => { console.error(error); process.exitCode = 1; });
