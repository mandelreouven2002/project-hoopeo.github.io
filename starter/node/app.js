'use strict';
const { Client } = require('pg');
async function main() {
  if (!process.env.DEP_DSN) throw new Error('Set DEP_DSN; see .env.example');
  const client = new Client({ connectionString: process.env.DEP_DSN });
  await client.connect();
  try {
    const result = await client.query('SELECT full_name, national_id FROM patients WHERE national_id = $1', ['123456782']);
    console.log(result.rows);
    try {
      await client.query('SELECT full_name FROM patients WHERE national_id LIKE $1', ['123%']);
      throw new Error('Partial protected match unexpectedly succeeded');
    } catch (error) {
      if (error.code !== 'ZD101') throw error;
      console.log('Expected refusal:', error.code);
    }
  } finally { await client.end(); }
}
main().catch((error) => { console.error(error.code || error.message); process.exitCode = 1; });
