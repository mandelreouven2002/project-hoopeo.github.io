# DEP Engine

The Database-Enforced Privacy engine of Project Hoopeo: a library (`dep_engine`) and a server (`dep-engine`).

- **SQL** on Apache DataFusion. DEP SQL adds `PROTECTED MATCH DOMAIN` columns, declared functions
  (`DISCLOSES`, `LINKS`, `REVEAL`), endpoints with fixed access paths, and match privileges.
- **The DEP analyzer** rewrites every logical plan before DataFusion optimizes it: protected values are
  matched through blind indexes (HMAC-SHA256), stored as XChaCha20-Poly1305 ciphertexts, returned as NULL,
  and released only by declared functions. Every rule that applied becomes a DEP note.
- **Storage on disk** (redb): rows and indexes live in `data.redb` and are read when a query needs them, in
  batches, by one blocking reader per partition, so a database does not have to fit in memory and the server
  starts in the time it takes to read the catalog. Readers use snapshots: they never wait for writers.
- **Indexes.** Every primary key, unique key, reference and PROTECTED column has an index; `CREATE [UNIQUE]
  INDEX` adds more. An equality or IN on the first column of an index reads only the matching rows. On a
  protected column the index key is the blind index entry, so a lookup never needs a plaintext value.
- **Constraints**: PRIMARY KEY (single or composite), UNIQUE, NOT NULL and REFERENCES (NO ACTION) are
  enforced: 23505, 23502, 23503. For a protected column, the error names the column, never the value.
- **Transactions** (the DEP Transaction Manager): every statement runs in a transaction, its own or one
  opened with BEGIN. Reads see a snapshot; a transaction sees its own writes and nobody else does until
  COMMIT, which is one durable write. One writer at a time (others wait up to `--lock-timeout`, then 55P03);
  writing a table that changed after BEGIN fails with 40001; after an error the transaction is aborted
  (25P02) until ROLLBACK or ROLLBACK TO. **SAVEPOINT**, ROLLBACK TO SAVEPOINT and RELEASE SAVEPOINT work as in
  PostgreSQL. **Schema changes are transactional**: CREATE and DROP of tables and indexes, functions, roles,
  grants and endpoints happen on the transaction's own copy of the catalog. A transaction left idle is ended
  after `--idle-in-transaction-timeout` (25P03), so a client that went away does not hold the write lock.
- **Bounded memory**: queries run within `--memory-limit`; sorts and aggregations beyond it spill to
  temporary files that are **encrypted** (XChaCha20-Poly1305, with a key that exists only in the server's
  memory), since they can hold values a DISCLOSES function released.
- **Streaming**: query results are sent over the wire protocol as they are read.
- **Audit log** (`audit.redb`, group commit): every protected match (with the number of values), every call
  of a DISCLOSES or LINKS function, every refused statement, every rate-limited statement and every key
  rotation. No protected value and no statement text is recorded. Each statement returns only after its
  records are on disk. `SHOW AUDIT [n]` (superusers).
- **Match limits**: `ALTER ROLE r MATCH LIMIT 600 PER HOUR` (or `NONE`, `DEFAULT`), and a server default for
  roles that are not superusers (`--match-limit 1000/minute`). Over the limit: `DEP_MATCH_RATE_LIMITED` (ZD109).
- **Key rotation**: `ALTER SYSTEM ROTATE MASTER KEY` re-indexes and re-encrypts every protected value with a
  new key and rebuilds every index, in one commit. A crash at any point leaves either the old key and data or
  the new ones.
- **PostgreSQL wire protocol** (pgwire): the simple and the extended protocol (prepared statements, with
  parameter types inferred for queries and writes), typed columns, TLS optional or required, a cap on
  connections. The user is the role and the database is the endpoint. DEP notes arrive as notices; DEP
  errors carry `ZDxxx` SQLSTATEs. An error message never carries the value of a bound parameter.
- **HTTP API** (Axum), over HTTPS with the same certificate: `POST /v1/query`, `GET /v1/health`, `GET /metrics`.

## Running

```sh
dep-engine --listen 0.0.0.0:5433 --data-dir /var/lib/dep-engine/data --tls-self-signed --tls-required \
           --http-listen 0.0.0.0:8433 --memory-limit 8GB --cache-size 4GB --max-connections 500
dep-engine --help
```

| Option | Environment | Meaning |
|---|---|---|
| `--listen` | `DEP_LISTEN` | Address of the wire protocol (default 127.0.0.1:5433) |
| `--data-dir` | `DEP_DATA_DIR` | Keep the data there (without it, everything is in memory) |
| `--key-file` | `DEP_KEY_FILE` | The master key (default DATA_DIR/master.key) |
| `--init` | `DEP_INIT` | A DEP SQL script run as `admin` once, when the database is new. Repeat the option for several; in `DEP_INIT`, separate them with colons |
| `--admin-password` | `DEP_ADMIN_PASSWORD` | Sets the admin password at every start |
| `--init-admin-password-file` | `DEP_INIT_ADMIN_PASSWORD_FILE` | The admin password of a new database, from a file |
| `--tls-cert`, `--tls-key` | `DEP_TLS_CERT`, `DEP_TLS_KEY` | PEM certificate chain and key |
| `--tls-self-signed`, `--tls-host` | `DEP_TLS_SELF_SIGNED`, `DEP_TLS_HOSTS` | Create a self-signed certificate in DATA_DIR/tls, for localhost and each host name (in `DEP_TLS_HOSTS`, separated by commas) |
| `--tls-required` | `DEP_TLS_REQUIRED` | Refuse connections without TLS |
| `--http-listen` | `DEP_HTTP_LISTEN` | Also serve the HTTP API there |
| `--match-limit` | `DEP_MATCH_LIMIT` | Default match limit, for example `1000/minute` |
| `--lock-timeout` | `DEP_LOCK_TIMEOUT` | Seconds a writer waits for the write lock (default 30) |
| `--memory-limit` | `DEP_MEMORY_LIMIT` | Memory for running queries (default a quarter of RAM) |
| `--cache-size` | `DEP_CACHE_SIZE` | Page cache of the data file (default an eighth of RAM, at most 4 GB) |
| `--idle-in-transaction-timeout` | `DEP_IDLE_IN_TRANSACTION_TIMEOUT` | Seconds before an idle transaction is rolled back (default 300; 0: never) |
| `--max-connections` | `DEP_MAX_CONNECTIONS` | Client connections served at once (default 200; 0: no limit) |

Older data directories are converted the first time they are opened: a 0.4 directory in place (rows are
re-encoded, indexes are built, the audit log moves to `audit.redb`), a 0.3 directory into a new `data.redb`
with its old files kept in `legacy-0.3/`. A primary or unique key whose existing rows have duplicates (0.4 did
not enforce keys) is kept, but not enforced; the server says so when it starts, and `SHOW INDEXES` shows it.
Remove the duplicates, then run `REINDEX TABLE t`.

## HTTP API

```sh
curl -u reception_app:demo -H 'Content-Type: application/json' \
     https://db.example.com:8433/v1/query?endpoint=operational \
     -d '{"sql": "SELECT full_name, national_id FROM patients WHERE national_id = $1", "params": ["123456782"]}'
```

The statements of one request run in order on one connection (so `BEGIN; ...; COMMIT;` works), stop at the
first error, and a transaction left open is rolled back. Each result has its statement as it was sent (with
`$1`, never with the values), its columns and rows (or its command tag), its DEP notes, and its error.

## Inspecting

`SHOW TABLES` (with row counts), `SHOW COLUMNS FROM t`, `SHOW INDEXES [FROM t]`, `SHOW FUNCTIONS`,
`SHOW ROLES`, `SHOW GRANTS`, `SHOW ENDPOINTS`, `SHOW PRIVACY MATRIX`, `SHOW DEP NOTES`, `SHOW AUDIT [n]`,
`SHOW STORAGE t [n]`, `SHOW dep.path`, `SHOW TRANSACTION ISOLATION LEVEL`, `EXPLAIN` (with how each table is
read: an index lookup, or every row). `/metrics` adds index lookups, full scans, rows read, spilled bytes and
idle transactions ended.

## Tests

`cargo test -p dep-engine`: the rules, privileges, persistence and the 0.3 and 0.4 migrations, transactions
and savepoints, transactional schema changes, constraints on ordinary and protected columns, index lookups,
a table larger than the query memory (with encrypted spilling), idle transactions, concurrent clients over the
wire, the audit log, match limits, key rotation, the wire protocol (simple and extended, typed, TLS optional
and required) and the HTTP API.

## Performance

Measured with pgbench (PostgreSQL's benchmark tool, over the wire protocol with prepared statements) on a
2-CPU virtual machine, against a table of 200,000 rows with a primary key and a unique PROTECTED column:

| Workload | 1 client | 16 clients |
|---|---|---|
| Lookup by primary key | 1.1 ms | 1,850 per second |
| Lookup by a PROTECTED column (blind index, audited) | | 1,490 per second |
| Update by primary key (each a durable commit) | 2.4 ms | 370 per second |
| Loading 200,000 rows in one INSERT ... SELECT | about 3 s | |

The server's memory stayed under 200 MB throughout. Reads scale with CPUs; writes are one at a time.

## Limits of this version

One transaction writes at a time (readers are never blocked). Hash joins do not spill in DataFusion, so a
join whose smaller side is larger than the query memory fails with 53200; sorts and aggregations spill.
References support NO ACTION only (no ON DELETE CASCADE), and DEFAULT and CHECK are not supported.

License: the binaries are distributed under the Project Hoopeo Evaluation License (`../LICENSES/LicenseRef-Hoopeo-Evaluation.txt`); a commercial licence is available. See `../LICENSING.md`.
