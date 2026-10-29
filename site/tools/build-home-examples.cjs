/* Capture the homepage's examples from the actual browser simulator at build time. */
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const root = path.resolve(__dirname, '../..');
const init = require(path.join(root, 'playground/node_modules/sql.js'));
const D = require(path.join(root, 'playground/depsql.js'));
const C = require(path.join(root, 'playground/src/clinic.js'));
(async () => {
  const SQL = await init();
  const server = new D.Server(SQL, {setup:C.clinicSetup});
  assert.ok(server.setupResults.every(result => result.ok), 'clinic initialization');
  const ordinary = new SQL.Database(); ordinary.exec(C.clinicSqlite());
  const definitions = JSON.parse(fs.readFileSync(path.join(root,'site/home-examples.json'),'utf8'));
  const examples = definitions.map(example => {
    const connection = server.connect(example.role,'operational');
    const result = connection.exec(example.sql).at(-1);
    connection.close();
    const plain = ordinary.exec(example.sql)[0];
    assert.ok(plain && plain.values.length, example.id + ': regular SQL returns records');
    if (example.expectedError) assert.equal(result.error?.sqlstate,example.expectedError,example.id);
    else {
      assert.equal(result.ok,true,example.id);
      assert.equal(result.rows.length,example.expectedRows,example.id);
      if (example.expectedNull) {
        const i = result.columns.findIndex(column => column.name === 'national_id');
        assert.ok(result.rows.every(row => row[i] === null),example.id + ': protected output');
        assert.ok(plain.values.every(row => row[i] !== null),example.id + ': ordinary output');
      }
    }
    return {...example,regular:{columns:plain.columns,rows:plain.values},dep:result.ok ? {columns:result.columns.map(c=>c.name),rows:result.rows,notes:result.notes} : {error:result.error}};
  });
  fs.writeFileSync(process.argv[2],JSON.stringify(examples));
  ordinary.close();
  console.log('PASS: four homepage scenarios captured and verified against DEP.js and SQLite.');
})().catch(error=>{console.error(error);process.exitCode=1;});
