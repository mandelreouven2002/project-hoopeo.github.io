/* Run the actual function definitions taught in chapter 3, on the full clinic.
   This checks DEP.js behavior, not native Engine conformance. */
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const init = require('sql.js');
const D = require('./depsql.js');

(async () => {
  const SQL = await init();
  const clinic = fs.readFileSync(path.join(__dirname, '../examples/clinic.sql'), 'utf8');
  const server = new D.Server(SQL, { setup: clinic });
  assert.equal(server.setupResults.filter((r) => !r.ok).length, 0);
  const admin = server.connect('admin', 'operational');
  const chapter = fs.readFileSync(path.join(__dirname, '../docs/book/03-functions.md'), 'utf8');
  const names = new Set(['monthly_fund_report', 'appointment_reminders', 'diagnoses_per_fund', 'fund_claims_report']);
  let definitions = 0;
  for (const block of chapter.matchAll(/```sql\n([\s\S]*?)```/g)) {
    const name = block[1].match(/^CREATE FUNCTION (\w+)/m)?.[1];
    if (!names.has(name)) continue;
    const sql = block[1].replace(/CREATE FUNCTION /g, 'CREATE OR REPLACE FUNCTION ');
    for (const result of admin.exec(sql)) assert.equal(result.ok, true, name + ': ' + JSON.stringify(result.error));
    definitions++;
  }
  assert.equal(definitions, 4, 'the four executable definitions in the book must be checked');
  const app = server.connect('doctors_app', 'operational');
  assert.deepEqual(app.exec("SELECT full_name, national_id FROM patients WHERE national_id IN ('123456782','234567893')").at(-1).rows,
    [['Dana Cohen', null], ['Yossi Levi', null]]);
  assert.equal(app.exec('SELECT p.full_name,m.diagnosis FROM patients p JOIN medical_records m ON m.patient_national_id=p.national_id').at(-1).error.sqlstate, 'ZD107');
  const billing = server.connect('billing_job', 'operational');
  assert.equal(billing.exec("SELECT * FROM monthly_fund_report(DATE '2026-10-01') ORDER BY visit_date").at(-1).ok, true);
  assert.equal(billing.exec("SELECT national_id AS exposed FROM monthly_fund_report(DATE '2026-10-01') ORDER BY exposed").at(-1).error.sqlstate, 'ZD104');
  console.log('PASS: four book function definitions, IN-list lookup, protected-join refusal and disclosure sorting (DEP.js).');
})().catch((error) => { console.error(error); process.exitCode = 1; });
