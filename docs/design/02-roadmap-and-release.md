# DEP: Database-Enforced Privacy

## Roadmap and Release

### One possible path from idea to implementation

**Reuven Mandelovich**

> **Status:** v1.0 · October 2026 · Licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/)
> **DEP is presented as an idea, and this document is not a commitment to build it.** It records one way DEP could be built and released, for anyone, including the author, who decides to. Where it says "will", read "would".

**About this document.** The companion document, [the DEP design proposal](01-dep-concept.md), defines what DEP is, how it is written, and how it works. This document sketches how DEP could be built and released. Two appendices complete the picture: how the design maps onto Apache DataFusion and PostgreSQL ([Appendix A](#appendix-a-mapping-the-design-to-real-engines)), and how DEP relates to existing products and research ([Appendix B](#appendix-b-how-dep-relates-to-existing-approaches)). What actually ships will be recorded in a separate, living document ([chapter 9](#9-after-the-release-what-shipped)).

---

## Contents

1. [Strategy: one engine end to end, then a second](#1-strategy-one-engine-end-to-end-then-a-second)
2. [Project components](#2-project-components)
3. [Milestones](#3-milestones)
4. [Distribution](#4-distribution)
5. [Before publishing: ownership, licensing, and naming](#5-before-publishing-ownership-licensing-and-naming)
6. [Publishing plan](#6-publishing-plan)
7. [Open questions](#7-open-questions)
8. [Publication text](#8-publication-text)
9. [After the release: what shipped](#9-after-the-release-what-shipped)

**Appendices**

- [Appendix A: Mapping the design to real engines](#appendix-a-mapping-the-design-to-real-engines)
- [Appendix B: How DEP relates to existing approaches](#appendix-b-how-dep-relates-to-existing-approaches)
- [References](#references)

---

## 1. Strategy: one engine end to end, then a second

Two full implementations of a database privacy model are two large projects. DEP therefore commits to **one engine first, end to end**, and treats the second as a later, conditional track.

**Track A: The reference server on Apache DataFusion.** A DEP server built on DataFusion, wrapped in a server process that speaks the PostgreSQL wire protocol and listens on one endpoint per access path. It accepts the **full syntax of the design proposal from day one**, because in DataFusion the parser and the logical plan are under the project's control. Its job is to prove one complete path: declaring protected fields, anchored matching, matching rejected on the masked path, linkage and disclosure through declared functions, and no leakage through output or errors.

**Track B: PostgreSQL, later and conditional.** PostgreSQL is reliable, familiar, and already trusted, and it remains the intended foundation for a production DEP Server. Track B begins only after Track A is complete and published, and it begins with a short gap analysis, not an implementation. Whether it proceeds, and how far, depends on what that analysis finds.

### Why DataFusion first

- **Control.** In PostgreSQL, DEP's rules are inserted into an engine through hooks, and most of the effort goes into making sure no path was missed. In DataFusion, the rules for `PROTECTED`, `REVEAL`, anchoring, linking regions, and disclosed values are part of the logical plan from the first line of code.
- **Syntax.** A PostgreSQL extension cannot add grammar. DataFusion's parser can be extended, so the reference server is the only place where the planned syntax can exist from the start.
- **Speed to a convincing demonstration.** A working server that rejects attacks in front of the audience is more persuasive than any document.

### What the reference server proves, and what it does not

It proves that the **model** is coherent and usable (the rules can be stated precisely, the syntax is natural, the errors make sense), and that the **trust boundary** holds: a client connected on the masked path cannot reach identifiers, because it holds nothing but a SQL connection.

It does **not** prove that the rules can be enforced inside a production database without bypasses. Several leakage channels of a real database (server logs, persistent statistics, bulk export, backup and restore) simply do not exist in it. That claim requires PostgreSQL. Every public statement about the reference server says so plainly:

> **A reference implementation that proves the model and the trust boundary, not a production database.**

### The PostgreSQL question

Anyone evaluating DEP seriously (a database vendor, a cloud provider running managed PostgreSQL, a security team) will ask whether it fits an engine they already run. The answer at first release is honest and specific: the design maps onto PostgreSQL's extension mechanisms as described in [Appendix A.2](#a2-postgresql), and the gap analysis (Milestone 5) will establish where engine patches are required. Promising more than that before Track A is complete would split the project in two.

---

## 2. Project components

| Name | Category | Role |
| --- | --- | --- |
| **Design proposal** | Document | [The DEP design proposal](01-dep-concept.md): what DEP is, why, and how it works. Explanatory, not normative. |
| **Normative contract** | Specification | A separate, short document derived from the design proposal: exactly what is accepted, what is returned, and which error is raised. Versioned independently (contract 0.1). |
| **`dep-conformance`** | Engine-independent test suite | Statements, roles, paths, and expected results or errors, derived from the normative contract. Any implementation can run it. |
| **`dep-core`** | Rust library | The rules, the matching index, anchoring analysis, declared-function tracking, the output boundary, and the errors. Knows nothing about networking. |
| **`dep-engine`** | Server process on DataFusion | Endpoints over the PostgreSQL protocol, authentication, roles, column and function privileges, the audit log, rate limits, and the catalog. The reference server. |
| **Contract explorer** | Optional interactive web page | A small in-browser simulator for showing the rules side by side. **Offers no protection** ([4.1](#41-the-contract-explorer)). |
| **`dep` extension** | PostgreSQL extension (pgrx) | Track B: implements the contract inside PostgreSQL. |
| **DEP Server** | Server distribution | Track B: PostgreSQL, the extension, configuration, and any required patches, packaged for deployment. |
| **Client drivers** | Standard PostgreSQL drivers | No custom SDK is required. A driver is never the enforcement boundary. |

DEP is not a new storage engine and not a new procedural language.

---

## 3. Milestones

### Milestone 0: Write the normative contract

Derive a short, normative contract from the design proposal. It fixes the two access paths, the semantics of protected text, match domains, the rules for anchored and unanchored matching, the rules for `DISCLOSES` and `LINKS`, protected foreign keys, how column privileges govern matching, and the error codes. It must specify projection behavior, input normalization, `NULL` semantics, duplicate values, protected literals, and which predicates count as anchored. It must publish a **supported-forms list**: every SQL form the first version accepts on protected values. Everything else fails closed.

Not every sentence of the design proposal becomes a rule. The contract contains only what an implementation must do and what a test can check.

**Deliverables:** the normative contract, the supported-forms list, and the first version of `dep-conformance`, written in the final syntax.
**Exit criterion:** every example in the design proposal that the contract covers has an expected result or a specific expected error, before any implementation begins.

### Milestone 1: Feasibility spike on DataFusion

A short, time-boxed spike on the things most likely to go wrong:

1. **The parser.** A DEP dialect of `sqlparser-rs` that accepts the new constructs (`PROTECTED MATCH DOMAIN`, the `DISCLOSES` and `LINKS` function attributes, `GRANT DISCLOSE`, `GRANT LINK`, and `CREATE ENDPOINT`) while every other statement parses exactly as before.
2. **A declared function.** A `LANGUAGE sql` function returning a table, expanded into the calling plan, with the `REVEAL` marker and the linking region surviving expansion and optimization.
3. **Anchoring analysis.** Distinguishing an anchored join from an unanchored one on the resolved plan, including `IN` lists and subqueries.
4. **An update by protected value.** `UPDATE ... WHERE national_id = $1` through a custom table provider with hidden internal columns.

**Deliverables:** minimal working prototypes and a short written report.
**Exit criterion:** each item works, or has a documented workaround (a parser patch, a pre-parsing step, a different expansion strategy). If one cannot be solved, the plan is revised before Milestone 2.

### Milestone 2: The reference server

Build `dep-core` and `dep-engine`: protected columns and match domains, keyed-index matching, protected literals, two endpoints over `pgwire`, authentication with a fixed path per session, roles with column privileges, `GRANT EXECUTE`, `GRANT DISCLOSE`, and `GRANT LINK`, declared functions created without public execution, uniqueness and foreign keys on protected columns, anchoring analysis, disclosure and linkage tracking, the audit log for every protected match, per-identity rate limits, the output boundary, every error in the design proposal, and scrubbing of the engine's own errors. Data lives in memory, loaded at startup or through `INSERT`. Keys are loaded from a server-side location the client cannot reach.

Out of scope: durable storage, transactions, full at-rest encryption, partial disclosure, deferred constraints, and every SQL form not on the supported-forms list.

**Deliverables:** the reference server, the clinic demonstration from the design proposal, and a passing conformance suite.
**Exit criterion:**
- the same anchored lookup succeeds on the operational endpoint and fails with `DEP_ENDPOINT_OPERATION` on the masked endpoint;
- an anchored join on protected columns succeeds on the operational path, and the same join without the anchor fails with `DEP_UNANCHORED_MATCH`;
- `diagnoses_per_fund` returns counts on the masked path;
- `monthly_fund_report` works, while joining its output to another table on the disclosed column fails with `DEP_DISCLOSED_VALUE_USE`;
- a role without privilege on `phone` cannot match on it;
- a table whose foreign key references a protected column without being `PROTECTED` is rejected with `DEP_REFERENCE_MISMATCH`, and an insert referencing a missing ID number fails without revealing the value and is counted as a protected lookup;
- every error in the design proposal is reproduced by at least one test;
- **canary tests pass**: synthetic protected values are planted in the data and also sent in queries, both as parameters and as literals, and an automated check confirms they never appear in any result, error, notice, audit record, query statistic, or server log, except through declared disclosures.

### Milestone 3: First public release (reference server v0.1)

Release only after the ownership and licensing decisions in [chapter 5](#5-before-publishing-ownership-licensing-and-naming) are made.

**Deliverables:** the public repository, a container image, the documentation site, the demonstration script and a short screen recording, and the release notes with the list of known limitations.
**Exit criterion:** a newcomer can run the demonstration in a few minutes with only a container runtime and `psql`, and see both the successful operational workflow and the rejected attacks.

### Milestone 4: Attack the reference server

Test casts, aliases, function wrappers, views, CTEs, function inlining, subqueries that try to smuggle unanchored matching, prepared statements, role changes, connection reuse, nested results, binary output, error messages, `EXPLAIN`, and engine error paths. Add adversarial tests and fuzzing. Invite outside review through `SECURITY.md`.

**Deliverables:** an adversarial test suite and an updated limitations document.
**Exit criterion:** no known bypass remains within the supported-forms list, and unsupported cases reliably fail closed. Passing the suite is evidence, not proof of complete security.

This completes Track A.

### Milestone 5: PostgreSQL gap analysis (Track B begins)

A short, written analysis, with a minimal spike only where a question cannot be answered on paper, of how each rule in the normative contract maps onto one supported PostgreSQL major version, using the integration points in [Appendix A.2](#a2-postgresql). The hardest rules to check are the output boundary in both result formats, anchoring analysis on the analyzed query tree, the immutable path, and rejection of `REVEAL` outside declared functions.

**Exit criterion:** for every rule, the analysis states whether an extension can enforce it or an engine patch is required. A decision follows: proceed with Track B, narrow it, or stop at the reference server.

### Milestone 6: The contract on PostgreSQL

Implement the contract in a PostgreSQL extension, using an interim syntax (a protected type such as `dep.protected_text` and administrative functions), because an extension cannot add grammar. Add a thin translation layer so that `dep-conformance` can run against the extension.

**Exit criterion:** the conformance results on PostgreSQL match those of the reference server, including the canary tests.

### Milestone 7: Attack the boundary on PostgreSQL

Everything in Milestone 4, plus the channels a production database adds: `RETURNING`, `COPY`, server logs, optimizer statistics, query statistics, restores with function-body validation disabled, encrypted storage, backup recovery, restarts, and key-version transitions.

**Exit criterion:** no known bypass remains within the supported-forms list.

### Decision point: Extension or fork

If the missing checks require a small set of engine changes, maintain them as a documented patch set. If protection depends on pervasive changes, reassess the supported SQL subset and the implementation strategy before expanding the product. This decision determines the long-term maintenance cost of DEP Server.

### Milestone 8: Package DEP Server

Package PostgreSQL, the mandatory extension, configuration, and any required patches as a reproducible server image. A missing or disabled DEP component must cause a startup failure.

**Exit criterion:** a new developer can start the server and reproduce the documented behavior with a standard PostgreSQL driver and no custom SDK.

### Milestone 9: Measure usefulness and cost

Benchmark lookup latency, join latency, throughput, storage, and index size against an equivalent plain PostgreSQL workload, on the same hardware, data, permissions, and query shapes. Performance targets are chosen before the evaluation, not after seeing the results.

**Exit criterion:** reproducible results with stated limitations, an independent technical review, and one bounded pilot that genuinely benefits from the contract.

### Milestone 10: Native syntax on PostgreSQL

Introduce DEP's syntax natively in PostgreSQL through engine patches, along with schema migration, introspection, backup behavior, and upgrade procedures.

**Exit criterion:** native syntax and interim declarations produce equivalent behavior, and the conformance suite runs without its translation layer.

### Beyond the first release

- **Partial disclosure forms** such as `DISCLOSES person_id AS SUFFIX(4)`.
- **More protected types and operations**, with a careful study of whether any comparison beyond equality can be offered without revealing too much.
- **Explicit linkage tokens**, a separately designed capability for correlating results across queries.
- **A formal model of the rules**, precise enough to reason about protection, not only test it.
- **A safe gateway for AI agents**, exposing the database to agents exclusively through the masked path and approved linking functions.
- **Aggregation thresholds and differential privacy for linking functions** called from the masked path, borrowing from data clean rooms ([Appendix B.8](#b8-what-dep-can-borrow)).
- **A schema check for open paths**: a tool that reads the foreign-key map and reports any path made only of ordinary keys between identities and sensitive data.
- **`dep-core` compiled to WebAssembly**, so the contract explorer runs the real rules instead of a simulation.
- **Other engines** implementing the contract and measured by `dep-conformance`.

### Immediate next actions

1. Decide on ownership and licensing ([chapter 5](#5-before-publishing-ownership-licensing-and-naming)).
2. Write the normative contract, the supported-forms list, and the first conformance tests.
3. Run the DataFusion feasibility spike.
4. Build the reference server until the conformance and canary tests pass.
5. Publish the first release with the clinic demonstration.

---

## 4. Distribution

| Package | Form | Audience | Notes |
| --- | --- | --- | --- |
| **Reference server** | Container image | Anyone who wants to try DEP and see it hold | The first public artifact. Clients connect with `psql` or any PostgreSQL driver. |
| **`dep-conformance`** | Separate repository and Rust crate | Implementers | Lets any implementation prove it meets the contract. |
| **Contract explorer** | Web page | Anyone, in thirty seconds | Optional; labeled as a simulation. |
| **`dep` extension** | Built per PostgreSQL major version and platform; distributed through PGXN | Teams running their own PostgreSQL | Track B. Requires `shared_preload_libraries`, not only `CREATE EXTENSION`. |
| **DEP Server** | Container image | Teams that want a ready configuration | Track B. The convenient path for real use, and the only path if engine patches are required. |

**A constraint to state openly:** managed PostgreSQL services allow only the extensions their provider has approved. A compiled extension loaded at server start cannot be installed there unless a provider adopts it. In practice, DEP on PostgreSQL would first run only where teams operate PostgreSQL themselves.

### 4.1 The contract explorer

> **A first prototype exists:** the [DEP SQL Playground](../playground/), a simulator built on sql.js that accepts DEP SQL, applies the rules of the design proposal, and shows the result an ordinary database would return for comparison. Unlike the explorer described below, it lets visitors create tables and declared functions. Like it, it is a simulation and protects nothing.

A single web page in which a visitor writes a query and sees, side by side, what an ordinary database returns, what the DEP operational path returns, and what the DEP masked path returns, with a list of ready-made attempts: a lookup by ID, a `CAST`, an unanchored join, a join on a disclosed value. It is the fastest way to show the idea, and fits a launch post or a link sent to a reviewer.

It can be built quickly with sql.js, SQLite compiled to WebAssembly, with a DEP rule layer written in JavaScript. That choice comes with constraints, which the explorer must respect:

- **It is a simulation, not a protection.** Everything runs in the visitor's browser, so there is no trust boundary. Every page says so: *a simulator. Everything runs in your browser; it illustrates the rules and protects nothing.*
- **It is a third implementation of the rules,** and could drift from the reference server. To keep it honest, the schema is fixed (the clinic), the visitor runs queries but does not create tables, declared functions are pre-defined, and the explorer passes the subset of `dep-conformance` it supports.
- **SQLite is not PostgreSQL.** It lacks SQL-defined table functions, roles, and several types and functions used in the examples, so the explorer translates a curated set of queries rather than accepting arbitrary SQL.

The long-term replacement is `dep-core` itself compiled to WebAssembly, so the explorer runs the same code as the server, still without a trust boundary, but no longer a separate implementation. DataFusion supports compiling to WebAssembly; bundle size needs to be evaluated.

---

## 5. Before publishing: ownership, licensing, and naming

### 5.1 Decide ownership first

Publishing the design or the code under an open-source license gives the idea to the world. That is excellent for reputation and adoption, but afterwards there is little left to sell. If a patent or a company might ever be wanted, that decision must be made **before the first public disclosure**: in many jurisdictions, a public disclosure before filing harms the ability to obtain a patent, and publishing the design proposal counts as disclosure.

If the project is carried out within the Technion (as a course, a final project, or with a faculty advisor) the institution's intellectual-property policy may give it rights in the work. This should be checked in advance, for example with the Technion's technology-transfer office. These are legal questions and should be confirmed with a qualified advisor.

### 5.2 Licensing

| Component | License |
| --- | --- |
| PostgreSQL | PostgreSQL License (permissive, BSD/MIT-style) |
| Apache DataFusion | Apache License 2.0 |
| pgrx | MIT |
| sql.js | MIT |

All of them permit building on, modifying, and redistributing them, commercially as well. For DEP itself, **Apache License 2.0** is recommended: it is compatible with all of them, and includes an explicit patent grant, which matters for a project that touches security and cryptography. "PostgreSQL" is a trademark; "based on PostgreSQL" is appropriate, but the product should not carry a name that sounds official.

### 5.3 Naming

"DEP" is already a well-known security term: Data Execution Prevention in Windows. Searches for "DEP database security" will mostly find it, and some readers will misread the title. The concept can keep its name, but the repository, crates, and images need a distinctive identifier (the project is now named Project Hoopeo, with the identifier `project-hoopeo`), checked for availability on GitHub, crates.io, and container registries before the first release. Names are hard to change after publication.

---

## 6. Publishing plan

### 6.1 When

DEP is published first as an idea: the design proposal, a short paper, and a post, in a public repository with a citable DOI. Publishing the idea does not depend on any implementation. A demonstration (the contract explorer, and later a reference server, if one is ever built) can follow and be added to the same repository.

### 6.2 Repository structure

```text
project-hoopeo/
├── README.md
├── LICENSE                  # Apache-2.0
├── NOTICE
├── SECURITY.md              # how to report a privacy bypass
├── CONTRIBUTING.md
├── CHANGELOG.md
├── docs/
│   ├── 01-dep-concept.md        # design proposal
│   ├── 02-roadmap-and-release.md
│   └── contract.md              # normative contract
├── crates/
│   ├── dep-core/
│   ├── dep-engine/
│   └── dep-conformance/
├── explorer/                # optional contract explorer
├── examples/
│   └── clinic/              # schema, data, and the demonstration script
└── Dockerfile
```

`SECURITY.md` matters especially for a project whose central claim is protection. It gives anyone who finds a bypass a private way to report it, instead of a public issue. The people who find bypasses are exactly the audience the project wants.

### 6.3 The README

Most visitors read only the README, so it must work in thirty seconds:

1. **One sentence** stating the promise: *Protected values remain inside the database unless released through an explicitly declared disclosure.*
2. **A recording**: the same query succeeding on one endpoint and failing on the other, and an unanchored join being refused.
3. **Quick start**: one container command and two `psql` commands.
4. **What it proves and what it does not**, honestly and briefly, with a clear label: reference implementation, not for production. Including: permitted matching can still reveal that a record exists.
5. **Links** to the design proposal, the contract, the explorer, and the conformance results.

### 6.4 Automation and versions

- **Continuous integration** runs the unit tests, `dep-conformance`, and the canary tests on every change, and shows the result as a badge in the README.
- **Two version numbers:** the contract version (contract 0.1) and the implementation version (`dep-engine` 0.1.0). The contract changes slowly, implementations quickly, and the separation allows other implementations of the same contract.
- **Crate names** are reserved early, but crates are published only when their interfaces stabilize.
- **The documentation site** is built from `docs/` with mdBook and served from GitHub Pages.

### 6.5 Launch

One concentrated launch day works better than a trickle: a post that explains the idea and links to the repository and the explorer, shared on Hacker News (Show HN), r/PostgreSQL, r/rust, the DataFusion community channels, LinkedIn, and a local PostgreSQL meetup if one is near. Be available that day to answer questions and criticism; the first hours shape much of the impression.

After launch, label a few issues as good first issues and answer every report quickly. A responsive project attracts contributors; a silent one is forgotten.

### 6.6 Research and industry routes

Large companies rarely accept unsolicited ideas, and many have explicit policies against it for legal reasons. The routes that work are public:

- **Open publication**: the repository, a post, a talk at a PostgreSQL conference. Companies come to the project, not the other way round.
- **A demonstration paper**: the main database conferences, SIGMOD and VLDB, have demonstration tracks that fit a project like this. A faculty advisor adds both guidance and credibility.
- **A portfolio**: a project like this opens doors at database and privacy companies even if they never adopt it as is.

In every route, what an organization is most likely to take is not the code but the **contract**: the rules, the error model, and the conformance suite, which it would then implement in its own engine. The documents deserve as much care as the code.

---

## 7. Open questions

These decisions are left open for the normative contract, and are good topics for review.

1. **Anchoring through another protected column.** Does a join on `national_id` restricted by `WHERE phone = $1` count as anchored? The proposal is yes.
2. **Anchoring through an ordinary key.** Does `WHERE patient_id = 42` anchor a join on protected columns? The proposal is no, or at least that it counts against the rate limit.
3. **Qualified access.** Which tables should require every operational read or write to be anchored, and should that be the default for tables with protected columns?
4. **Is `NULL`-ness protected?** Knowing whether a person has an identifier on record is itself information. Should `IS NULL` and `COUNT(national_id)` be allowed, and on which path?
5. **How is suppression signaled?** Standard PostgreSQL result metadata has no field for "this value was hidden". Options include a catalog view, a companion function, or a protocol-level extension.
6. **Normalization per match domain.** Who defines normalization rules, and how are they versioned without breaking existing indexes?
7. **Foreign-key checks and rate limits.** Should a failed reference check count the same as a lookup, or more, given that it is a cheaper way to test identifiers?
8. **Uniqueness and conflicts.** How do `UNIQUE` constraints and `ON CONFLICT` behave on protected columns, and should the inference they allow be reduced?
9. **Purpose as a clause.** Is `COMMENT ON FUNCTION` enough, or should purpose become a first-class, machine-checked part of a declaration?
10. **Nested declared functions.** May one declared function call another, and how do disclosed columns and linking regions compose?
11. **Rate-limit semantics.** Per identity, per column, or per match domain, and how are anchored joins counted?
12. **Naming of match domains.** PostgreSQL already uses the word *domain* for `CREATE DOMAIN`. Should `MATCH DOMAIN` be renamed, for example to `MATCH CLASS`?
13. **Interim syntax on PostgreSQL.** How should a match domain be expressed in an extension that cannot add grammar?
14. **Error-code class.** The design proposal suggests the SQLSTATE class `ZD`. It must be confirmed against the classes PostgreSQL already uses.
15. **Timing.** Should a later version require constant-time matching, and at what cost?
16. **Governance** of the contract once other implementations exist.

---

## 8. Publication text

### One-sentence definition

DEP is a database server in which `PROTECTED` is a built-in property of data: ordinary queries can match on protected values one at a time but never return them, whole datasets are linked through them only in declared `LINKS` functions, and they leave the database only through declared `DISCLOSES` functions.

### Short description

DEP, Database-Enforced Privacy, is a proposed database server that separates four things ordinary databases fold into one privilege: matching a sensitive value, linking datasets through it, returning it, and disclosing it. A `PROTECTED` field supports lookups and joins anchored to a single value on an operational connection, while a masked read connection cannot match on it at all. Linking whole datasets and releasing values are allowed only in functions declared `LINKS` and `DISCLOSES`, the database's equivalent of Rust's `unsafe`, so every such place is explicit, reviewable, and listed in the catalog.

### Announcement

We are introducing DEP, Database-Enforced Privacy: a database server that makes the permitted use of sensitive fields part of query execution.

The idea starts with a practical distinction. A clinic's reception desk needs a patient's ID number to find her; her doctor needs it to see her diagnoses. That need does not mean either application must ever receive ID numbers back from the database, and it certainly does not mean either should be able to join every patient to every diagnosis in a single query.

DEP proposes a `PROTECTED` declaration and two access paths. On the operational path, applications find, update, and connect records one protected value at a time, and every such match is recorded. On the masked read path (the natural home for analysts, BI tools, and AI agents) protected values cannot be matched at all. Ordinary queries on either path never return protected values. Linking whole datasets and releasing values are possible only in declared `LINKS` and `DISCLOSES` functions, the single places where those operations are legal.

DEP is precise about its limits: permitted matching can still reveal that a record exists and associate it with the data returned. What it removes is the single query that exports a register or links two sensitive datasets wholesale.

The first release is a reference server built on Apache DataFusion. It speaks the PostgreSQL protocol, so you can connect with `psql`, run the same query on both paths, and watch one succeed and the other refuse. It proves the model and the trust boundary; it is not a production database. We welcome technical discussion, criticism, and attempts to break it.

---

## 9. After the release: what shipped

This document describes plans. What actually ships is recorded separately, in a living document maintained with each release, so that plans are never mistaken for results. It will contain:

- **Release notes** for each version of each implementation.
- **Conformance results** per engine and per contract version, including the canary tests.
- **The supported-forms list** for each release.
- **Known limitations** and open bypass reports, with their status.
- **Measurements**, once they exist, with the scripts to reproduce them.
- **Security advisories**, published through the process in `SECURITY.md`.

---

## Appendix A: Mapping the design to real engines

The architecture in [chapter 12 of the design proposal](01-dep-concept.md#12-architecture) is engine-independent. This appendix considers the two engines on which it could be realized ([chapter 1](#1-strategy-one-engine-end-to-end-then-a-second)): how the design maps onto each, and what each lacks.

### A.1 The reference server on Apache DataFusion

Apache DataFusion is an extensible query engine written in Rust: it parses SQL, plans, optimizes, and executes queries, and lets the embedding application control the logical plan. It is a library, not a database. A DEP reference server would wrap it in a server process that speaks the PostgreSQL wire protocol, using the Rust `pgwire` library, so that clients connect with `psql` or any standard PostgreSQL driver and the trust boundary of the design ([12.1](01-dep-concept.md#121-the-trust-boundary)) is real.

**What maps naturally**

| DEP component | DataFusion mechanism |
| --- | --- |
| Endpoints and paths | One `pgwire` listener per endpoint; the session's path is fixed when the connection is accepted |
| DEP analyzer and plan re-check | Custom analyzer rules over the logical plan, and a final check after optimization |
| Matching | Predicates on protected columns rewritten to comparisons on internal index columns; joins become equi-joins on index entries |
| Output boundary | A final projection and a wrapper on the result stream, before encoding |
| Parameters | DataFusion's prepared statements with `$1`-style placeholders |
| Errors with `DETAIL`, `HINT`, and SQLSTATE | `pgwire`'s PostgreSQL-format error responses |

**What the reference server must build itself**

| Gap | Approach |
| --- | --- |
| DEP's syntax | A DEP dialect of the SQL parser DataFusion uses (`sqlparser-rs`), extending the PostgreSQL dialect with a small, fixed set of constructs: the `PROTECTED MATCH DOMAIN` column option, the `DISCLOSES` and `LINKS` function attributes, `GRANT DISCLOSE` and `GRANT LINK`, and `CREATE ENDPOINT`. `REVEAL(x)` and all queries parse unchanged. If the dialect mechanism cannot express a construct, a small parser patch or a pre-parsing step handles it. |
| Users, roles, and privileges | DataFusion has none. The server implements authentication, the mapping from identity to path and roles, column privileges (which decide who may match on which protected column), `GRANT EXECUTE` on declared functions without public execution, and `GRANT DISCLOSE` and `GRANT LINK`. |
| Table-returning SQL functions | Declared functions are stored and expanded into the calling plan, like parameterized views, with `REVEAL` markers and linking regions preserved ([14.2](01-dep-concept.md#142-tracking-disclosed-values-and-linking-regions)). |
| Hidden internal columns | A custom table provider that exposes only the logical protected column. |
| `UPDATE` and `DELETE` | Implemented by the server's table provider where DataFusion's support is insufficient. |
| Unique and foreign-key constraints | DataFusion does not enforce them. The server's table provider enforces both on index entries, with each protected foreign-key check counted as a protected lookup. |
| `COMMENT ON` and `dep.declared_functions` | A virtual catalog table served by the DEP catalog. |

**What does not exist, by design**

Transactions, durable storage, and crash recovery. The reference server loads data at startup or accepts it through `INSERT`, and loses it on restart. It also lacks several leakage channels of a real database (server logs, persistent statistics, bulk export) which is precisely why it proves the **model** and the **trust boundary**, but not resistance to every channel of a production engine.

**Engine-specific risks**

- `EXPLAIN` exists and can display constants and statistics, so it is rejected for statements involving protected values.
- DataFusion's own error messages may quote literals or values, so every non-DEP error passes through the output boundary and is scrubbed or withheld.
- Statistics such as column minimum and maximum must never be collected on internal protected columns.

### A.2 PostgreSQL

PostgreSQL would be the natural foundation for a production DEP Server. The design maps onto its extension mechanisms as follows; whether they are sufficient is exactly what a gap analysis would have to establish ([chapter 3](#3-milestones)).

| DEP component | Candidate PostgreSQL mechanism |
| --- | --- |
| Endpoints and paths | `ClientAuthentication_hook`, storing an immutable per-backend path |
| DEP analyzer | `post_parse_analyze_hook` on the analyzed query tree |
| Plan re-check | `planner_hook` |
| Output boundary | `ExecutorRun_hook`, wrapping the destination receiver so values are replaced before text or binary encoding |
| Declaration checker, `COPY`, `SET`, and other utility commands | `ProcessUtility_hook` |
| Log scrubbing | `emit_log_hook` |
| Protected representation and matching | A user-defined type with input and output functions that never produce plaintext, plus an operator class for equality |
| Protected foreign keys | PostgreSQL's referential-integrity checks, running through the protected type's equality operator, with violation messages reduced to the constraint name |

Two constraints shape the PostgreSQL track:

- **An extension that uses these hooks must be loaded at server start**, through `shared_preload_libraries`, not only with `CREATE EXTENSION`. Installing it requires control of the server's configuration.
- **An extension cannot add grammar.** PostgreSQL's SQL grammar is compiled into the server. Until engine patches add native syntax, the PostgreSQL extension uses an interim form (a protected type such as `dep.protected_text` and administrative functions), while the reference server already accepts the full syntax of the design proposal.

---

## Appendix B: How DEP relates to existing approaches

DEP is not built from new techniques. Using a column in a predicate without returning it, masking output, restricting joins, approving queries in advance, keyed equality indexes, and declaring privacy policy as code all exist today, in products and in research. This appendix describes what exists, what DEP borrows, and what it adds. It is written through one lens: **Privacy as Code**: where the privacy policy is written, where it is enforced, and what it actually controls.

The descriptions of products below are based on their public documentation as of October 2026.

### B.1 Privacy as Code around the database

The tools that use the name *Privacy as Code* work mostly **around** the database.

- **Fides** (Ethyca) defines privacy policies in YAML over a shared taxonomy of data categories, data uses, and data subjects. Systems and datasets are annotated with the same taxonomy, and policies are evaluated in CI, so that a change that, for example, uses contact information for advertising is rejected before it ships.
- **Privado** scans application code to discover which personal data is collected and where it flows (to third parties, APIs, and logs) and lets teams write policy guardrails over the results.
- **General policy-as-code engines**, such as Open Policy Agent and Cedar, evaluate authorization rules written as code, and are called by applications or gateways.

These tools make privacy policy reviewable and testable, and they are the direct inspiration for DEP's development method ([chapter 10 of the design proposal](01-dep-concept.md#10-designing-with-dep)). What they do not do is enforce anything at the moment a query runs. A query the scanner never saw, written by a new AI agent at three in the morning, is not covered. DEP takes the same idea (policy as reviewed, tested code) and moves its enforcement into the engine that executes the query.

### B.2 Output controls inside the database

Several databases already separate **using** a column from **returning** it, at least for output:

- **Snowflake projection policies** are schema-level objects that decide whether a column may appear in a query's final output. A constrained column can still be used in `WHERE` clauses and inner queries; projecting it either fails or, with `ENFORCEMENT => 'NULLIFY'`, returns `NULL`. The constrained column also cannot be inserted into another table or passed to external functions. This is the closest existing mechanism to DEP's *return* decision.
- **Dynamic data masking** in SQL Server, **Oracle Data Redaction**, and **PostgreSQL Anonymizer** replace values in the output while predicates run against the real values. Oracle's documentation states that the `WHERE` clause is never redacted. PostgreSQL Anonymizer declares its masking rules inside the database schema, with security labels, explicitly as an implementation of privacy by design.
- **Databricks Unity Catalog** applies column masks and row filters, including policies driven by governed tags, so that classification drives enforcement.

DEP adopts the core idea, and the suppression to `NULL` matches Snowflake's `NULLIFY` behavior. But these mechanisms are output controls, and their own documentation is candid about what that leaves open. Snowflake's documentation notes that a constrained column can still be used to target a specific individual with `WHERE`, that joins can expose values when the matching column in another table is unconstrained, and that error messages can contain a value. Masking leaves the real value available to predicates, which permits inference. None of them distinguishes looking up one record from linking two datasets, and none makes the places where values *may* leave into a reviewable inventory of code.

### B.3 Collaboration controls in data clean rooms

Data clean rooms solve a neighboring problem: letting two organizations analyze their combined data without either seeing the other's rows.

- **Snowflake** adds **join policies**, which can require that a table is always joined and restrict joins to allowed keys, and **aggregation policies**, which require every query to aggregate and drop groups below a minimum size.
- **BigQuery analysis rules**, applied to views, enforce aggregation thresholds, differential-privacy budgets, join restrictions that limit which columns can be used in a join, and list-overlap queries.
- **AWS Clean Rooms** attaches one analysis rule (aggregation, list, or custom) to each configured table, specifying join columns, columns that may not appear in output, minimum aggregation thresholds, and, for custom rules, **analysis templates**: pre-approved queries that are the only ones allowed to run.

These systems are the strongest evidence that the problem DEP addresses is real and commercially important. They also contain ideas DEP shares: restricting how a sensitive column may be joined, preferring aggregates, and, in AWS's analysis templates, the principle that sensitive processing happens only in **pre-approved queries**, which is what DEP's declared functions are.

The differences are in scope and in mechanism. Clean rooms are built for multi-party analytics in a data warehouse, not for the operational database that an application reads and writes every second. Their rules constrain *which columns* may be joined or *how results* must be aggregated; they do not distinguish an application finding one patient from the same application linking every patient. And their approved queries are governed as collaboration configuration, not type-checked against the provenance of each value.

### B.4 Cryptographic approaches

- **Client-side encryption** (SQL Server Always Encrypted, MongoDB Queryable Encryption) protects data from an untrusted server or administrator. A client holding the keys receives plaintext. This is the opposite trust model to DEP, which protects data from clients and trusts the server; the two are complementary.
- **Encrypted query processing research**, such as CryptDB, also protects data from the server.
- **Application-side blind indexes**, as in the CipherSweet library, store a keyed hash next to each encrypted value so that applications can search by equality. DEP uses the same technique internally. The difference is where the key lives: with an application-side index, the application holds the key, can hash every possible value, and can enumerate them; in DEP, the key and the enforcement stay in the server.

### B.5 Research on policy enforcement in databases

- **Hippocratic databases** (IBM Research, VLDB 2002, followed by *Limiting Disclosure in Hippocratic Databases*, VLDB 2004) proposed that databases take responsibility for privacy principles (purpose specification, consent, limited disclosure, limited retention) and enforce purpose-based disclosure inside the database. DEP's `DISCLOSES` functions, documented with a purpose, are a much narrower descendant of that vision.
- **Qapla** (USENIX Security 2017) stores policies written in SQL in the database itself, and enforces them in a database adapter, so that policy compliance does not depend on application correctness.
- **Blockaid** (OSDI 2022) sits between a web application and its database, verifies with an SMT solver that each query complies with the data-access policy, and blocks queries it cannot verify.
- **IFDB** (EuroSys 2013) added decentralized information-flow labels to PostgreSQL, tracking how information moves between principals.

This line of work established that policy can and should be enforced by the database rather than the application, and it is essential prior work to study. DEP applies a narrower, type-based idea to a specific practical problem, rather than general policy languages or information-flow labels.

### B.6 Side by side

| | Use without returning | One value vs. whole datasets | Controls linkage | Exits declared as code | Enforced at query time, in the engine |
| --- | --- | --- | --- | --- | --- |
| Fides, Privado, OPA, Cedar | None | No | No | Policies as code, outside the database | No |
| Snowflake projection policies | Yes | No | No; joins can expose values | No; role conditions | Yes |
| Dynamic masking, Oracle Data Redaction, PostgreSQL Anonymizer | Partly; predicates see real values | No | No | Rules declared in schema (Anonymizer) | Yes |
| Snowflake join and aggregation policies | None | No | Partly: required joins, allowed keys, minimum groups | No | Yes |
| BigQuery analysis rules | None | No | Partly: join restrictions, thresholds, differential privacy | No | Yes, on views |
| AWS Clean Rooms | Partly: columns excluded from output | No | Yes, for collaborations | Partly: analysis templates | Yes |
| Client-side encryption | No; clients with keys see plaintext | No | No | No | Different trust model |
| Hippocratic databases, Qapla, Blockaid, IFDB | Varies | No | Varies | Policies in the database or a proxy | Yes |
| **DEP** | **Yes** | **Yes: anchoring** | **Yes: `LINKS`** | **Yes: `DISCLOSES`, `LINKS`, cataloged** | **Yes** |

The table simplifies. Each system does things DEP does not (differential privacy, multi-party collaboration, general policy languages) and its strengths should not be read as gaps.

### B.7 What DEP adds

DEP's contribution is a combination, applied to the operational database that applications and AI agents query directly:

1. **Protection as a property of the value, not of the output.** A protected value cannot be cast, sliced, or passed to arbitrary functions anywhere in a query, not only in the final projection; protected literals in the query text are recognized and redacted; and protection is checked on referencing columns.
2. **The distinction between one value and whole datasets.** Anchored matching lets an application find, update, and connect the records of the value it was given, and nothing more. No existing mechanism we have found draws this line; projection policies explicitly permit targeted filtering and unrestricted joins.
3. **Linkage and disclosure as declared, type-checked code.** `LINKS` and `DISCLOSES` are function attributes whose bodies are analyzed when they are created, whose revealed values are tracked as terminal, and which cannot be executed by `PUBLIC`. This is closest to AWS's analysis templates, but expressed in the SQL of the database itself and checked by its type rules.
4. **A complete inventory by construction.** Because nothing else can reveal protected values or link datasets through them, `dep.declared_functions` lists every such place: an answer to "where can our ID numbers leave?" that no output-masking policy can give.
5. **A development method in which the policy, the schema, and the tests are the same artifact.** The privacy matrix becomes grants and declared functions, and its cells become conformance tests. This is the Privacy as Code idea of B.1, with the enforcement moved inside the engine.

### B.8 What DEP can borrow

The existing systems also suggest improvements that are not yet part of DEP:

- **Aggregation thresholds** from clean rooms and Snowflake aggregation policies could become a property of `LINKS` functions called from the masked path, so that statistics never report groups smaller than a minimum size.
- **Differential privacy**, as offered in BigQuery and AWS Clean Rooms, could protect aggregate outputs of linking functions against repeated-query attacks.
- **Tag-driven policies**, as in Databricks and Snowflake, suggest letting classification metadata drive which columns are declared `PROTECTED`, connecting Step 2 of the method to the schema automatically.

The novelty of DEP's combination should still be confirmed by independent expert review. This appendix reflects a search of public documentation and well-known research, not an exhaustive survey.

---

## References

1. Apache DataFusion. https://datafusion.apache.org/
2. sqlparser-rs. https://github.com/apache/datafusion-sqlparser-rs
3. pgwire. https://github.com/sunng87/pgwire
4. sql.js: SQLite compiled to WebAssembly. https://github.com/sql-js/sql.js
5. pgrx: PostgreSQL extension development in Rust. https://github.com/pgcentralfoundation/pgrx
6. PGXN: the PostgreSQL Extension Network. https://pgxn.org/
7. PostgreSQL License. https://www.postgresql.org/about/licence/
8. Apache License 2.0. https://www.apache.org/licenses/LICENSE-2.0
9. mdBook. https://rust-lang.github.io/mdBook/
10. PostgreSQL documentation. *CREATE TYPE*: type input, output, and binary-transfer functions. https://www.postgresql.org/docs/current/sql-createtype.html
11. PostgreSQL documentation. *Extending SQL*. https://www.postgresql.org/docs/current/extend.html
12. PostgreSQL documentation. *Shared library preloading* (`shared_preload_libraries`). https://www.postgresql.org/docs/current/runtime-config-client.html
13. Snowflake. *Projection policies*. https://docs.snowflake.com/en/user-guide/projection-policies
14. Snowflake. *Join policies*. https://docs.snowflake.com/en/user-guide/join-policies
15. Snowflake. *Aggregation policies*. https://docs.snowflake.com/en/user-guide/aggregation-policies
16. Google Cloud. *Restrict data access using analysis rules* (BigQuery). https://docs.cloud.google.com/bigquery/docs/analysis-rules
17. AWS. *Adding an analysis rule to a configured table* (AWS Clean Rooms). https://docs.aws.amazon.com/clean-rooms/latest/userguide/add-analysis-rule.html
18. Microsoft. *Dynamic Data Masking* (SQL Server). https://learn.microsoft.com/en-us/sql/relational-databases/security/dynamic-data-masking
19. Oracle. *Oracle Data Redaction and nested functions, inline views, and the WHERE clause*. https://docs.oracle.com/en/database/oracle/oracle-database/26/dbred/oracle-data-redaction-and-nested-functions-inline-views-and-where-clause.html
20. PostgreSQL Anonymizer. *Dynamic Masking*. https://postgresql-anonymizer.readthedocs.io/en/stable/dynamic_masking/
21. PostgreSQL Anonymizer. *Declare masking rules*. https://postgresql-anonymizer.readthedocs.io/en/latest/declare_masking_rules/
22. Databricks. *Row filter and column mask policies* (Unity Catalog ABAC). https://docs.databricks.com/aws/data-governance/unity-catalog/abac/policies
23. Microsoft. *Always Encrypted* (SQL Server). https://learn.microsoft.com/en-us/sql/relational-databases/security/encryption/always-encrypted-database-engine
24. MongoDB. *Queryable Encryption*. https://www.mongodb.com/docs/manual/core/queryable-encryption/
25. Paragon Initiative. *CipherSweet: blind indexes*. https://ciphersweet.paragonie.com/internals/blind-index
26. Ethyca. *Creating privacy policies as code with Fides*. https://content.ethyca.com/?p=96024
27. Privado. *Open source*. https://privado.ai/open-source
28. R. Agrawal, J. Kiernan, R. Srikant, Y. Xu. *Hippocratic Databases*, VLDB 2002.
29. K. LeFevre, R. Agrawal, V. Ercegovac, R. Ramakrishnan, Y. Xu, D. DeWitt. *Limiting Disclosure in Hippocratic Databases*, VLDB 2004.
30. A. Mehta et al. *Qapla: Policy compliance for database-backed systems*, USENIX Security 2017. https://www.usenix.org/conference/usenixsecurity17/technical-sessions/presentation/mehta
31. W. Zhang et al. *Blockaid: Data Access Policy Enforcement for Web Applications*, OSDI 2022. https://www.usenix.org/conference/osdi22/presentation/zhang
32. D. Schultz, B. Liskov. *IFDB: Decentralized Information Flow Control for Databases*, EuroSys 2013.
33. R. A. Popa, C. M. S. Redfield, N. Zeldovich, H. Balakrishnan. *CryptDB: Protecting Confidentiality with Encrypted Query Processing*, SOSP 2011.
