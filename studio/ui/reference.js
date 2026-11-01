/* The DEP SQL quick reference and the error codes, for Help > DEP SQL Reference. */
(function () {
  'use strict';
  const ERRORS = [
    ['ZD001', 'DEP_REVEAL_OUTSIDE_DISCLOSURE', 'REVEAL may appear only in a function declared DISCLOSES for the value\'s match domain.'],
    ['ZD002', 'DEP_DECLARATION_PRIVILEGE', 'Declaring DISCLOSES or LINKS needs the DISCLOSE or LINK privilege on the match domain.'],
    ['ZD003', 'DEP_DECLARED_LANGUAGE', 'Declared functions are LANGUAGE sql, so the engine can analyze their bodies.'],
    ['ZD004', 'DEP_DECLARED_CALL', 'A declared function may call only functions declared for the same purpose and domain.'],
    ['ZD005', 'DEP_PUBLIC_EXECUTE', 'A declared function can never be executable by PUBLIC.'],
    ['ZD006', 'DEP_REFERENCE_MISMATCH', 'A reference and the column it references have the same protection and match domain.'],
    ['ZD101', 'DEP_PROTECTED_OPERATION', 'The only operation on a protected value is full-value equality.'],
    ['ZD102', 'DEP_ENDPOINT_OPERATION', 'The operation is valid, but not on this connection\'s access path.'],
    ['ZD103', 'DEP_MATCH_DOMAIN_MISMATCH', 'Values from different match domains cannot be compared.'],
    ['ZD104', 'DEP_DISCLOSED_VALUE_USE', 'A disclosed value may be returned, and not used to filter, join, group, or store.'],
    ['ZD105', 'DEP_UNQUALIFIED_ACCESS', 'This table requires every read or write to name one record.'],
    ['ZD106', 'DEP_PATH_IMMUTABLE', 'The access path is fixed when the connection opens.'],
    ['ZD107', 'DEP_UNANCHORED_MATCH', 'Ordinary queries match protected values one value at a time; whole datasets are linked only in LINKS functions.'],
    ['ZD108', 'DEP_UNSUPPORTED_FORM', 'The construct is not covered, so DEP rejects it rather than run it with uncertain protection.'],
    ['ZD109', 'DEP_MATCH_RATE_LIMITED', 'The identity made more protected lookups than its limit allows.'],
    ['ZD901', 'DEP_ERROR_WITHHELD', 'An engine error might have contained data, so its message was withheld.']
  ];
  const esc = (s) => String(s).replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c]));
  const pre = (s) => `<pre>${esc(s.trim())}</pre>`;
  const HTML = `
<h2>Protected columns</h2>
<p>A PROTECTED column is stored as a blind index and a ciphertext, never as plaintext. Every protected column belongs to a match domain; values can be compared only within one domain.</p>
${pre(`CREATE TABLE patients (
    patient_id  BIGINT PRIMARY KEY,
    national_id TEXT PROTECTED MATCH DOMAIN person_id,
    phone       TEXT PROTECTED MATCH DOMAIN phone_number,
    full_name   TEXT
);
CREATE TABLE medical_records (
    record_id           BIGINT PRIMARY KEY,
    patient_national_id TEXT PROTECTED REFERENCES patients (national_id),
    diagnosis           TEXT
);`)}
<h2>Matching</h2>
<p>The only operation on a protected value is full-value equality with a constant or a parameter (=, IN). A protected column in a result is returned as NULL.</p>
${pre(`SELECT full_name, national_id FROM patients WHERE national_id = '123456782';   -- national_id comes back NULL
SELECT full_name FROM patients WHERE national_id LIKE '1%';                     -- DEP_PROTECTED_OPERATION`)}
<p>A join between protected columns must be anchored to a single protected value; joining whole datasets is linking, which only a LINKS function may do.</p>
${pre(`SELECT p.full_name, m.diagnosis
FROM patients AS p JOIN medical_records AS m ON m.patient_national_id = p.national_id
WHERE p.national_id = '123456782';`)}
<h2>Declared functions</h2>
<p>DISCLOSES lets a function return a protected value in plaintext, with REVEAL. LINKS lets it join protected values without an anchor. Declaring needs the DISCLOSE or LINK privilege on the domain, and the function runs with its owner's privileges.</p>
${pre(`CREATE FUNCTION appointment_reminders(day DATE)
RETURNS TABLE (phone TEXT, starts_at TIMESTAMP)
LANGUAGE sql
DISCLOSES phone_number
AS $$
    SELECT REVEAL(p.phone), a.starts_at
    FROM appointments AS a JOIN patients AS p USING (patient_id)
    WHERE CAST(a.starts_at AS DATE) = day
$$;
GRANT EXECUTE ON FUNCTION appointment_reminders TO reminder_job;`)}
<h2>Endpoints and access paths</h2>
<p>The database name of a connection is its endpoint, and the endpoint fixes the access path. OPERATIONAL allows lookups and writes; MASKED_READ allows only reads that match no protected value, and functions marked safe for it.</p>
${pre(`CREATE ENDPOINT reporting MODE MASKED_READ;
GRANT CONNECT ON ENDPOINT reporting TO bi_dashboard;
SHOW dep.path;`)}
<h2>Roles and privileges</h2>
${pre(`CREATE ROLE reception_app LOGIN PASSWORD 'secret';
GRANT CONNECT ON ENDPOINT operational TO reception_app;
GRANT SELECT (patient_id, full_name, national_id), INSERT ON patients TO reception_app;
GRANT DISCLOSE ON MATCH DOMAIN person_id TO privacy_admin;
GRANT LINK ON MATCH DOMAIN person_id TO privacy_admin;
SET ROLE reception_app;   RESET ROLE;`)}
<p>A column SELECT privilege is also the privilege to match the column.</p>
<h2>Keys, constraints and indexes</h2>
<p>PRIMARY KEY, UNIQUE, NOT NULL and REFERENCES are enforced (23505, 23502, 23503). On a PROTECTED column, uniqueness and references are checked through blind index entries, and an error never shows the value: <code>Key (national_id) already exists</code>. Every protected column, key and reference has an index; an equality or IN on the first column of an index reads only the matching rows (EXPLAIN shows how each table is read).</p>
${pre(`CREATE TABLE cards (
    card_id BIGINT PRIMARY KEY,
    holder  TEXT PROTECTED MATCH DOMAIN person_id UNIQUE NOT NULL,
    label   TEXT
);
CREATE INDEX appointments_type ON appointments (visit_type);
CREATE UNIQUE INDEX cards_label ON cards (label);
DROP INDEX appointments_type;
REINDEX TABLE cards;          -- rebuilds; re-enforces a unique key whose duplicates were removed
SHOW INDEXES FROM cards;`)}
<h2>Transactions</h2>
<p>Each statement commits on its own unless a transaction is open. In a transaction, reads see a snapshot taken at BEGIN together with the transaction's own writes; other connections see nothing until COMMIT, which writes everything to disk in one durable step. One transaction writes at a time: another writer waits (up to the lock timeout, 55P03). Writing a table that another transaction changed after BEGIN fails with 40001; roll back and run it again. After an error the transaction is aborted (25P02) until ROLLBACK, or ROLLBACK TO a savepoint. Schema changes are transactional too: a table created, dropped or granted in a transaction is seen by others only after COMMIT. A transaction left idle holds the write lock; the server ends it after the idle-in-transaction timeout (25P03).</p>
${pre(`BEGIN;
INSERT INTO appointments (appointment_id, patient_id, starts_at, visit_type) VALUES (201, 1, '2026-11-02 09:00', 'Follow-up');
SAVEPOINT before_update;
UPDATE patients SET health_fund = 'Maccabi' WHERE national_id = '123456782';
ROLLBACK TO SAVEPOINT before_update;   -- undoes the UPDATE only
RELEASE SAVEPOINT before_update;
CREATE TABLE visits (visit_id BIGINT PRIMARY KEY, patient_id BIGINT REFERENCES patients (patient_id));
COMMIT;   -- or ROLLBACK`)}
<p><code>CREATE TEMPORARY TABLE</code> makes a table that is kept in memory only and is gone when the server stops.</p>
<h2>Audit log and match limits</h2>
<p>Every protected match (with the number of values), every call of a function declared DISCLOSES or LINKS, and every statement the DEP rules refused is recorded, without any protected value and without the statement text. A role can be limited to a number of protected matches per period; the server can set a default for roles that are not superusers (<code>--match-limit 1000/minute</code>).</p>
${pre(`SHOW AUDIT 100;                                       -- superusers
ALTER ROLE reception_app MATCH LIMIT 600 PER HOUR;
ALTER ROLE reception_app MATCH LIMIT NONE;            -- or DEFAULT`)}
<h2>Master key rotation</h2>
<p>A superuser can replace the master key. Every protected value is decrypted with the old key, then indexed and encrypted with the new one, in one commit; the old key no longer opens the data.</p>
${pre(`ALTER SYSTEM ROTATE MASTER KEY;`)}
<h2>Inspecting</h2>
<table><tr><th>Command</th><th>Shows</th></tr>
<tr><td><code>SHOW TABLES</code>, <code>SHOW COLUMNS FROM t</code></td><td>Tables (with row counts), columns, protected columns and constraints</td></tr>
<tr><td><code>SHOW INDEXES [FROM t]</code></td><td>Indexes: columns, unique or not, whether enforced, and whether their keys are blind index entries</td></tr>
<tr><td><code>SHOW FUNCTIONS</code></td><td>Declared functions: what they disclose or link, owner, who may execute them</td></tr>
<tr><td><code>SHOW ROLES</code>, <code>SHOW GRANTS</code>, <code>SHOW ENDPOINTS</code></td><td>Roles, privileges, endpoints and their paths</td></tr>
<tr><td><code>SHOW PRIVACY MATRIX</code></td><td>For each protected column and role: paths, whether it may match, and the functions that disclose or link its domain</td></tr>
<tr><td><code>SHOW DEP NOTES</code></td><td>How the DEP rules applied to the last statement</td></tr>
<tr><td><code>SHOW STORAGE t [n]</code></td><td>The first n rows (default 1000) as stored: blind indexes and ciphertexts (superusers)</td></tr>
<tr><td><code>SHOW AUDIT [n]</code></td><td>The latest audit records (superusers)</td></tr>
<tr><td><code>EXPLAIN query</code></td><td>The plan after the DEP rewrite, and how each table is read (index lookup or every row)</td></tr>
</table>
<h2 id="ref-errors">Error codes</h2>
<table><tr><th>SQLSTATE</th><th>Code</th><th>Meaning</th></tr>
${ERRORS.map((e) => `<tr><td><code>${e[0]}</code></td><td><code>${e[1]}</code></td><td>${esc(e[2])}</td></tr>`).join('')}
</table>
<h2>Keyboard</h2>
<table><tr><th>Key</th><th>Command</th></tr>
<tr><td>F8</td><td>Execute the selection, or the whole SQL window</td></tr>
<tr><td>Ctrl+Enter (Cmd+Enter)</td><td>Execute the statement at the cursor</td></tr>
<tr><td>F5</td><td>Explain plan of the statement at the cursor</td></tr>
<tr><td>F10, Shift+F10</td><td>Commit, Rollback</td></tr>
</table>`;
  window.DEP_REFERENCE = { html: HTML, errors: Object.fromEntries(ERRORS.map((e) => [e[1], e[2]])) };
})();
