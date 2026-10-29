# Changelog

## Clear presence-test examples (2026-10-29)

- Distinguished a missing stored value from a protected value displayed as NULL.
- Added one permission table and separate examples for presence filters, returned flags, COUNT(column) and COUNT(*), with both access paths and implementation status.
- Aligned the glossary and server chapter with the explanation and linked them to the same examples.

## Rule precision and known differences (2026-10-29)

- Stated the presence rule everywhere it appears: on the masked read path a protected value cannot be tested for presence (`IS NULL`, `COUNT(col)`, ZD102); on the operational path presence may be tested in a condition and never returned (ZD101). This resolves a conflict between the SQL chapter and the guarantees in chapter 8.
- Stated the terminal rule precisely: a disclosed value may be returned, renamed or cast, and is not a `DISTINCT` key or the argument of a function or operator.
- DEP.js follows both rules, and no longer accepts a scalar subquery over a disclosed value as a sort key. The 38 simulator checks and the tutorial and book checks pass.
- Chapter 6: the native disclosed-sort gap is now confirmed by native testing of DEP Engine 0.5.1, and three further differences of DEP Engine 0.5.2 are documented (presence tests, computing on disclosed values, unchecked `VALUES` rows). Fixes exist in the Engine's development source and are not yet released.
- Corrected chapter numbers carried over from the design proposal, and the main navigation now reads Learn DEP, since the book covers more than the language.
- Homepage: the example match domain is `email_address`, the status link is named for what it opens, and internal links use a plain arrow.

## Code-led homepage introduction (2026-10-27)

- Paired the DEP introduction with a compact, highlighted users/email lookup and its protected NULL output.
- Moved the client-to-database enforcement diagram into How DEP works and kept the detailed clinic comparison below.

## Concept-first homepage (2026-10-27)

- Introduced Database-Enforced Privacy before the clinic, with a general explanation of value use, linkage and disclosure and a client-to-database enforcement diagram.
- Moved the four verified SQL comparisons and clinic schema into a worked-example section after the conceptual explanation.
- Kept the playground query links, code coloring and task paths, and updated homepage links to canonical book chapters.

## Unified documentation (2026-10-27)

- Consolidated the design proposal and implementation guides into eight ordered chapters, with chapter navigation, previous/next links and chapter-specific browser titles.
- Kept Privacy as Code as a beginner introduction leading to chapter 7; restored one clear main navigation and moved reference into a collapsible menu.
- Merged errors and DEP notes, embedded playground help, removed duplicate tutorial bodies, and preserved old URLs and section links with redirects.
- Retained all 30 illustration families, the architectural views and all 16 Mermaid definitions; diagrams render locally with licensed Mermaid Tiny.
- Corrected conflicting proposal material for current SQL behavior and identified future features and the existing native disclosed-sort difference.
- Added checks for the book structure, historical links, illustration inventory and executable function examples.

## Website update (2026-10-27)

Website and documentation changes. The native DEP Engine version remains 0.5.2.

Follow-up: restored GitHub and Install in the main navigation; added local SQL/DEP syntax highlighting, four verified interactive homepage comparisons, a query-reading guide, complete role commands, Node verification and troubleshooting.

- Added a demonstration-first homepage, task-based navigation, documentation search and code copying.
- Added a complete application tutorial and downloadable Python/Node.js starter, compatibility table and GDPR mapping.
- Corrected nested installation code blocks, service/manual-server instructions and the report sorting example.
- Added a five-step browser tour and aligned simulator disclosure sorting with the documented terminal rule.
- Restored the homepage wordmark and removed synthetic-data warning labels.
- Documented the reported native 0.5.1 disclosed-sort gap; added name, alias and ordinal compatibility cases and made the specification table explicit about sort keys. Native 0.5.2 has not been retested for this issue.
- Added automated tutorial checks; native Engine integration remains a separate, explicit verification step.


## 0.5.2

Licensing and distribution. DEP Engine, DEP CLI and Hoopeo Studio are now distributed as binaries under the
Project Hoopeo Evaluation License; DEP.js, the specification and the examples are unchanged. No changes to the
behavior of DEP Engine or DEP CLI.

- The source of DEP Engine, DEP CLI and Hoopeo Studio is not published. The installers are published as
  GitHub Releases of the public repository, with `SHA256SUMS`, and the Docker image of DEP Engine as
  `ghcr.io/mandelreouven2002/dep-engine`.
- The AGPL 3.0 and the additional permission for DEP CLI no longer apply. Every installer carries the
  Evaluation License, the notice and the third-party notices.
- Hoopeo Studio: Help > Licenses and Notices shows the Evaluation License, the notice and the third-party
  notices.
- New: guides and a reference (`docs/guides`, `docs/reference`), and a website built from this repository.
- The playground loads its fonts from its own files instead of Google Fonts.

## 0.5.1

DEP Engine
- Fixed: `current_date`, `now()` and the other functions of the current time failed in every statement
  ("invoke should not be called on a simplified current_date() function"). Each statement now has its start
  time, and the arguments of a declared function are folded to values before its body runs, so
  `monthly_fund_report(current_date)` works.
- Fixed: an ORDER BY could be ignored when the query also had a filter on a cast column, such as
  `WHERE CAST(starts_at AS DATE) = d ORDER BY starts_at` (DataFusion 55 inferred that the column was
  constant). The cast's input in a filter now goes through an internal function that stops the inference.

DEP.js
- Follows DEP Engine 0.5: the same rules, errors (with the engine's SQLSTATEs and messages), DEP notes and
  audit records. `compat.js` originally compared 280 statements on DEP Engine and DEP.js with matching
  results. The updated simulator follows the specification for disclosed sorting, and added regression cases
  expose a reported native 0.5.1 gap. See the compatibility guide; current full parity is not claimed.
- Transactions: BEGIN, COMMIT, ROLLBACK, savepoints and transactional schema changes; other connections
  read the last commit; one writer at a time (55P03), 40001 and 25P02 as in the engine.
- Constraints (PRIMARY KEY, also on a protected column, UNIQUE, NOT NULL, REFERENCES) with the engine's
  messages; indexes (CREATE [UNIQUE] INDEX, DROP INDEX, REINDEX); EXPLAIN with the engine's Read: lines.
- IN lists, NOT IN and OR on protected columns are lookups, as in the engine; IS NULL is allowed.
- Roles with LOGIN and PASSWORD, GRANT CONNECT ON ENDPOINT (instead of the `\hba` command), SET ROLE,
  match limits per role, the audit log (SHOW AUDIT), the privacy matrix, key rotation, and the other SHOW
  commands.
- The playground's clinic is examples/clinic.sql (with its dates moved to today). New example tabs
  (list of IDs, transaction, constraints, indexes, roles, match limit, audit, who can match); each result
  shows its DEP notes; new Access and Audit tabs.
- Removed: qualified access (`ALTER TABLE ... SET (qualified_access)`), `ALTER TABLE ... ADD COLUMN`, the
  `plain` server option and the statement log, which the engine does not have.

## 0.5.0

DEP Engine
- Storage on disk: rows and indexes are read from data.redb when queries need them, in batches and in
  parallel partitions; nothing about a table is kept in memory, and starting the server reads only the
  catalog. Rows use a compact binary encoding. Writes change only the rows they touch.
- Indexes: every primary key, unique key, reference and PROTECTED column has one; CREATE [UNIQUE] INDEX,
  DROP INDEX, REINDEX, SHOW INDEXES. Equality and IN on an indexed column read only the matching rows; on a
  protected column the key is the blind index entry. EXPLAIN shows how each table is read.
- Constraints enforced: PRIMARY KEY (also composite), UNIQUE, NOT NULL, REFERENCES (NO ACTION), with
  PostgreSQL's SQLSTATEs (23505, 23502, 23503). Errors on protected columns never show the value.
- Savepoints: SAVEPOINT, ROLLBACK TO SAVEPOINT, RELEASE SAVEPOINT.
- Transactional schema changes: tables, indexes, functions, roles, grants and endpoints can be created and
  dropped inside a transaction; others see them only after COMMIT, and ROLLBACK undoes them.
- Bounded query memory (--memory-limit), with spilling to encrypted temporary files.
- Query results stream to the client over the wire protocol.
- The audit log moved to audit.redb, written with group commit.
- Idle transactions are ended after --idle-in-transaction-timeout (default 300 s); --max-connections
  (default 200); --cache-size for the data file.
- The extended protocol infers parameter types for INSERT, UPDATE and DELETE.
- Values of bound parameters are removed from every error message, on the wire protocol and the HTTP API, and
  the HTTP API returns each statement as sent ($1), never with the values.
- New metrics: index lookups, full scans, rows read, spilled bytes, idle transactions ended, query memory.
- 0.4 data directories are converted on first open (keys with existing duplicates are kept but not
  enforced, until REINDEX).

- examples/clinic.sql: patients.national_id is UNIQUE (enforced through blind index entries).

DEP CLI
- `\di [TABLE]` lists indexes.

Hoopeo Studio
- No demo database: the Demo Clinic profiles, the Open Demo Clinic command and the option to load the demo
  into a new database are gone (examples/clinic.sql is still in the source distribution).
- The Output tab's error and DEP-note badges count only the latest run, and explain themselves on hover.
- Tools > Indexes; the object browser shows row counts, unique keys and references.

## 0.4.0

DEP Engine
- Storage in redb (`data.redb`): the catalog, rows by row id, the audit log and the key fingerprint. Each
  commit writes only the changed rows, in one durable write transaction. Data directories from 0.3 are
  migrated on first open (the old files are kept in `legacy-0.3/`).
- The DEP Transaction Manager: BEGIN, COMMIT, ROLLBACK; snapshot reads; one writer at a time with a lock
  timeout (55P03); write conflicts (40001); aborted transactions (25P02); schema changes outside
  transactions (25001).
- `CREATE TEMPORARY TABLE`: tables kept in memory only.
- The audit log (`SHOW AUDIT`): protected matches, disclosures, linking, refusals, rate limits, key rotation;
  never a protected value or the statement text.
- Match limits per role (`ALTER ROLE ... MATCH LIMIT n PER MINUTE | NONE | DEFAULT`) and a server default
  (`--match-limit`); `DEP_MATCH_RATE_LIMITED` (ZD109) is now enforced.
- Master key rotation (`ALTER SYSTEM ROTATE MASTER KEY`), crash-safe, with a key fingerprint check at start.
- PostgreSQL wire protocol: the extended protocol (prepared statements and parameters), typed columns,
  transaction status, one connection per client socket, `--tls-required`.
- HTTP API on Axum (`--http-listen`): `/v1/query`, `/v1/health`, `/metrics` (Prometheus), over HTTPS with the
  server's certificate.
- A missing table is now SQLSTATE 42P01.

DEP CLI
- The prompt shows the transaction state (`=*>`, `=!>`); `\audit`.

Hoopeo Studio
- Session > Begin Transaction; the status bar shows an open or failed transaction; Tools > Audit Log; the
  reference covers transactions, the audit log, match limits and key rotation.

## 0.3.0

- Project Hoopeo: DEP Engine, DEP CLI and Hoopeo Studio (renamed from DEP Developer).
- DEP notes, the privacy matrix, client and server TLS.
- Hoopeo Studio rebuilt: native menus, square design, splash screen, connections to remote, local and
  served databases.
- Installers for macOS (script), Linux, Docker and source; layered licensing.
