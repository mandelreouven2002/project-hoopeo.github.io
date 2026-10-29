# DEP.js: the DEP SQL Playground

Part of Project Hoopeo. Licensed under the Apache License 2.0. This is the `playground/` folder of the public repository.

A browser simulator of DEP (Database-Enforced Privacy), targeting the DEP SQL specification and DEP Engine 0.5.2, with the known difference documented below. You write DEP SQL against the clinic from the design proposal and see the same statement run three times, side by side: in a regular database, on DEP's masked read path, and on DEP's operational path.

> **A simulator.** Everything runs in your browser, so there is no trust boundary and the cryptography is a stand-in: it illustrates the rules and protects nothing. For real enforcement, use DEP Engine, DEP CLI or Hoopeo Studio (binaries under the Project Hoopeo Evaluation License; see the [Releases](https://github.com/mandelreouven2002/project-hoopeo.github.io/releases)).

## Compatibility with DEP Engine

DEP.js implements DEP SQL rules, errors, SQLSTATEs, DEP notes and audit records. `compat.js` runs the same statements on a DEP Engine server and DEP.js, both loaded with `examples/clinic.sql`, and compares rows, command tags, full errors and DEP notes. The historical 280-statement result predates the current disclosed-sort behavior. The suite now includes explicit name, alias and ordinal sorting cases; native parity has not been re-established. See the [known difference](../docs/guides/compatibility.md#known-difference-sorting-disclosed-values).

What DEP.js has, as the engine does:

- **The DEP rules**: equality matching through blind indexes (also `IN` lists, `NOT IN`, `OR` and `CASE`), anchoring of joins between protected columns (also through subqueries and CTEs), `GROUP BY` and `DISTINCT` on protected columns only in a `LINKS` function, the masked read path, declared functions (`DISCLOSES`, `LINKS`, `REVEAL`, the declared-call rule), the terminal rule for disclosed values, and the column privileges that make up the privacy matrix.
- **Transactions**: `BEGIN`, `COMMIT`, `ROLLBACK`, `SAVEPOINT`, `ROLLBACK TO SAVEPOINT`, `RELEASE SAVEPOINT`, with transactional schema changes. Other connections read the last commit while a transaction writes; one transaction writes at a time; writing a table that another transaction changed after `BEGIN` fails with 40001; after an error the transaction is aborted (25P02) until `ROLLBACK` or `ROLLBACK TO`.
- **Constraints**: `PRIMARY KEY` (also on a protected column), `UNIQUE`, `NOT NULL` and `REFERENCES`, with the engine's messages: the key of an ordinary column is shown, a protected value never is.
- **Indexes**: the indexes the engine keeps for keys, references and protected columns, `CREATE [UNIQUE] INDEX`, `DROP INDEX`, `REINDEX`, and `EXPLAIN` with the engine's `Read:` lines (an index lookup, or every row).
- **Roles and endpoints**: `CREATE`, `ALTER` and `DROP ROLE` with `LOGIN` and `PASSWORD`, `GRANT CONNECT ON ENDPOINT`, `SET ROLE` and `RESET ROLE`, `CREATE ENDPOINT`, and match limits (`ALTER ROLE r MATCH LIMIT 100 PER MINUTE | NONE | DEFAULT`).
- **The audit log**: every protected match (with the number of values), every call of a declared function, every refused statement and every key rotation, without values, in `SHOW AUDIT [n]`.
- **Inspection**: `SHOW TABLES`, `COLUMNS FROM t`, `INDEXES`, `FUNCTIONS`, `ROLES`, `GRANTS`, `ENDPOINTS`, `PRIVACY MATRIX`, `DEP NOTES`, `AUDIT`, `STORAGE`, `dep.path`, `role`, and `ALTER SYSTEM ROTATE MASTER KEY`.

Where DEP.js differs from the engine, by design:

- `EXPLAIN` shows the SQLite statement DEP.js runs, then the engine's `Read:` lines; the engine shows DataFusion's plan there.
- A second writer fails at once with 55P03, where the engine waits up to `--lock-timeout`.
- Readers in a transaction see the latest commit rather than a snapshot taken at `BEGIN`.
- Results come from SQLite: dates and times are text, and a few functions behave like SQLite's (`/` by zero gives NULL, for example).
- `current_date` and `now()` are in the browser's time zone; the engine uses UTC.
- `\connect role [endpoint]` opens a new connection inside a script (a simulator command, like `\c` in DEP CLI).

## Using it

There are two ways to open it:

- **`site/`**: the page and the two compilers as separate files. Serve the folder (`npm start`, then open `http://localhost:8000`); browsers do not load WebAssembly from `file://`.
- **`dep-sql-playground.html`**: the same page with everything inlined, so it opens directly from disk and works offline.

The page has three areas:

- **Left: three result panes.** Every statement runs in all three, and each result shows its DEP notes:
  - **Regular SQL**: sql.js alone, without `depsql.js`, against an ordinary copy of the clinic (`clinicSqlite()` in `src/clinic.js`) that keeps ID numbers and phone numbers as plain text. SQLite has no `CREATE FUNCTION` in SQL, so in that copy `diagnoses_per_fund` and `monthly_fund_report` are views; tabs that call them run the equivalent query on the views and show it above the result. Tabs with no regular equivalent (the audit log, the privacy matrix) say so.
  - **DEP masked read path**: a connection as `admin` through the `analytics` endpoint (`MASKED_READ`).
  - **DEP operational path**: a connection as `admin` through the `operational` endpoint.

  A pane shows when its connection is in a transaction, or in a failed one.
- **Top right: the editor.** Tabs above it load ready examples (a plain select, a lookup by ID, a partial match, a list of IDs, an anchored and an unanchored join, a linking function, a disclosing function, a join on a disclosed value, an update, a transaction with a savepoint, constraints, indexes and `EXPLAIN`, roles, a match limit, the audit log and the privacy matrix), and **Your code** keeps your own statements in the browser. Run with **Run** or **Ctrl+Enter**.
- **Bottom right: the schema**, as the operational connection sees it: **Tables** (with row counts and indexes), the declared **Functions**, **Access** (roles, endpoints and grants), the **Audit** log, and the **Storage** as the DEP server keeps it (index entries and ciphertexts).

The two DEP panes are two connections to one DEP server, so a transaction open in one is isolated from the other. **Reset** rebuilds both databases from the setup script.

## How it works

The page runs on two compilers, each in its own file:

| File | What it does |
|---|---|
| `sql-wasm.js` + `sql-wasm.wasm` | [sql.js](https://github.com/sql-js/sql.js) 1.12.0: SQLite compiled to WebAssembly. It runs ordinary SQLite SQL. |
| `depsql.js` | The DEP SQL compiler and server, in front of sql.js. It turns DEP SQL into SQLite. |

`depsql.js` works in four stages:

1. **Tokenizer and parser** for DEP SQL: `SELECT` with joins (inner, left, right, full, cross, `USING`), `WITH`, subqueries, `UNION`, `EXCEPT`, `INTERSECT`, `VALUES`, window functions; `INSERT`, `UPDATE`, `DELETE`; `CREATE TABLE` with `PROTECTED MATCH DOMAIN`; `CREATE INDEX`; `CREATE FUNCTION ... DISCLOSES ... LINKS`; roles, grants and endpoints; transactions; `SHOW` and `EXPLAIN`.
2. **Analyzer**, which applies the engine's rules to the statement: the kinds of values (ordinary, protected, disclosed), the access paths, the match graph of each query block and its anchors, declared-function regions, privileges (each declared function runs with its owner's), and match limits. A statement that breaks a rule fails here and never reaches SQLite.
3. **Rewriter**, which turns an admitted statement into SQLite: a match on a protected column becomes a comparison of blind index entries, protected outputs become `NULL`, a declared function becomes a subquery, and `REVEAL` becomes a call to the decryption function, available only in a disclosing function.
4. **sql.js** runs the compiled statement inside the transaction manager: each statement in a savepoint, constraints checked before its writes are kept, and a copy of the database for the other connections while a transaction writes.

The clinic is not part of the compiler: it is `examples/clinic.sql` (as `clinicSetup()` in `src/clinic.js`, with its dates moved to today) and runs as `admin` when the page starts, through the same parser and analyzer as any query you write.

### Using `depsql.js` on its own

```js
const SQL = await initSqlJs();                                  // sql.js
const server = new DepSQL.Server(SQL, { setup: clinicSql });    // a DEP server; clinicSql is a DEP SQL script
const conn = server.connect('reception_app', 'operational');    // role and endpoint
const [r] = conn.exec('SELECT full_name FROM patients WHERE national_id = $1', ['123456782']);
// r: { ok, kind: 'rows' | 'command', columns, rows, tag, notes, error: { message, code, sqlstate, hint }, session }
conn.exec('BEGIN; ...; COMMIT');
conn.close();                                                   // rolls back an open transaction
```

`new DepSQL.Server(SQL, { setup, matchLimit: '1000/minute' })` sets a default match limit for roles that are not superusers, as `--match-limit` does. `server.describe(conn)`, `server.storage(table, conn)` and `server.auditLog` return the catalog, the stored form of a table and the audit records. `DepSQL.parse(sql)` returns the syntax tree. In Node, `require('./depsql.js')` returns the same object.

## Building and testing

```bash
npm install            # installs sql.js 1.12.0
npm test               # the playground's setup, transactions across connections, savepoints, audit, match limits, key rotation
npm run compat         # compares DEP.js with a running DEP Engine (see below)
npm run build          # writes site/ and dep-sql-playground.html from depsql.js and src/
npm start              # serves site/ at http://localhost:8000
```

The compatibility test needs a DEP Engine server with the clinic and its HTTP API. The `dep-engine` binary comes from the [Releases](https://github.com/mandelreouven2002/project-hoopeo.github.io/releases), under the Evaluation License; it is run here for testing only:

```bash
dep-engine --http-listen 127.0.0.1:8433 --init ../examples/clinic.sql --admin-password admin
DEP_ENGINE_URL=http://127.0.0.1:8433 DEP_ADMIN_PASSWORD=admin npm run compat
```

Start the server afresh for each run: the cases change its data, and expect the clinic as it was loaded.

**Known difference:** DEP.js rejects sorting disclosed values by name, alias or ordinal with ZD104, following specification section 8.9. A project review reports that DEP Engine 0.5.1 accepts all three; native 0.5.2 has not been retested here. These cases remain in `compat.js` and must report differences against an affected server. See [compatibility and limitations](../docs/guides/compatibility.md#known-difference-sorting-disclosed-values). An earlier 280/280 result does not establish parity for the updated simulator or these regressions.

## License

The code is licensed under the Apache License 2.0 (see [`LICENSE`](LICENSE) and [`../NOTICE`](../NOTICE)). sql.js is MIT-licensed; see [`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md).
