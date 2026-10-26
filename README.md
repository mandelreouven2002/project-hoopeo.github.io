<p align="center"><img src="branding/project-hoopeo-wordmark.png" alt="Project Hoopeo, Database-Enforced Privacy" width="520"></p>

# Project Hoopeo

Project Hoopeo builds **DEP SQL**: SQL in which the privacy rules are part of the database. Mark a column `PROTECTED`, and applications can still use it to find, check and connect records, but no query gets the value back; the few exceptions are named, granted and audited functions. **DEP Engine** is the database server that enforces it, **DEP CLI** and **Hoopeo Studio** are its shell and developer application, and **DEP.js** runs it in the browser. New here? Read [DEP SQL in five minutes](docs/guides/overview.md).

This repository holds the open layers of the project: the specification, the documentation, DEP.js and the playground, the examples, the website, and the installers, which are published as [GitHub Releases](https://github.com/mandelreouven2002/project-hoopeo.github.io/releases).

<p align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/images/diagrams/dep-sql-adds-dark.png"><img src="docs/images/diagrams/dep-sql-adds.png" alt="What DEP SQL adds to SQL: protected columns in the schema, endpoints at login, and declared functions for the exceptions" width="760"></picture></p>

<p align="center"><a href="https://mandelreouven2002.github.io/project-hoopeo.github.io/playground/"><img src="site/assets/screenshots/playground.png" alt="The DEP SQL Playground: the same statement in a regular database, on the masked read path and on the operational path" width="760"></a></p>

<p align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/images/studio/studio-lookup-dark.png"><img src="docs/images/studio/studio-lookup.png" alt="Hoopeo Studio after a lookup by a protected value, with the DEP notes in the Output pane" width="760"></picture><br><sub>Hoopeo Studio: a lookup by a protected value, with its DEP notes.</sub></p>

- **Website:** <https://mandelreouven2002.github.io/project-hoopeo.github.io/>
- **Playground:** [try DEP in your browser](https://mandelreouven2002.github.io/project-hoopeo.github.io/playground/), with nothing to install
- **Install:** see [Install](#install) below
- **Docs:** the [guides and reference](https://mandelreouven2002.github.io/project-hoopeo.github.io/docs/), also in [`docs/`](docs/)
- **Privacy as Code:** [DEP and privacy as code](https://mandelreouven2002.github.io/project-hoopeo.github.io/privacy-as-code/)

## Products

| Product | What it is | License |
|---|---|---|
| <img src="site/assets/brand/dep-engine-64.png" alt="" width="28"> **DEP Engine** | The database server: Rust, Apache DataFusion, disk-based storage with indexes (redb), constraints, transactions with savepoints and transactional schema changes, roles and privileges, an audit log and match limits, the PostgreSQL wire protocol (simple and extended), an HTTP API, TLS | Binaries under the Evaluation License; source private |
| <img src="site/assets/brand/dep-cli-64.png" alt="" width="28"> **DEP CLI** | `dep-cli`, the command-line tool, with DEP Engine built in | Binaries under the Evaluation License; source private |
| <img src="site/assets/brand/hoopeo-studio-64.png" alt="" width="28"> **Hoopeo Studio** | The developer application, in the spirit of PL/SQL Developer, with DEP Engine built in | Binaries under the Evaluation License; source private |
| <img src="site/assets/brand/dep-js-64.png" alt="" width="28"> **DEP.js** | The DEP SQL Playground: a browser simulator of DEP Engine 0.5, on sql.js | Apache 2.0 |
| **Specification** | The design proposal and the roadmap | CC BY 4.0 |

<p align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/images/diagrams/products-dark.png"><img src="docs/images/diagrams/products.png" alt="How the products connect: clients reach DEP Engine through its two endpoints; DEP CLI and Hoopeo Studio include the engine; DEP.js runs in the browser" width="760"></picture></p>

The binaries are free for evaluation, research, teaching and non-production testing. Production and commercial use needs a commercial license. See [Licensing](#licensing).

## Install

Every download is one click, and the Install page has numbered steps for each product.

**[macOS](https://mandelreouven2002.github.io/project-hoopeo.github.io/install/#hoopeo-studio)** &nbsp;|&nbsp; **[Linux](https://mandelreouven2002.github.io/project-hoopeo.github.io/install/#hoopeo-studio)** &nbsp;|&nbsp; **[Docker](https://mandelreouven2002.github.io/project-hoopeo.github.io/install/#engine-docker)** &nbsp;|&nbsp; **[In the browser](https://mandelreouven2002.github.io/project-hoopeo.github.io/install/#depjs)**

## A short example

Reception finds a patient by the ID number she presents. The ID number is matched through the blind index, and it comes back as NULL:

```
reception_app@operational=> SELECT full_name, national_id FROM patients WHERE national_id = '123456782';
DEP:  PROTECTED column patients.national_id is matched by equality through the blind index of MATCH DOMAIN person_id.
DEP:  Output column national_id is PROTECTED column patients.national_id (MATCH DOMAIN person_id) and is returned as NULL.
 full_name  | national_id
------------+-------------
 Dana Cohen | NULL
(1 row)
```

Plain SQL gets no notes. A DEP rule gets a note, and a violation gets an error with a `ZDnnn` SQLSTATE and a hint. Over the PostgreSQL protocol the notes are `NOTICE` messages, so `psql` shows them as `NOTICE:  DEP: ...`. The [getting started guide](docs/guides/getting-started.md) runs this against a server.

## Contents of this repository

```
docs/
  design/       the design proposal, the roadmap and the diagrams
  guides/       getting started, concepts, DEP SQL, transactions, administration, applications
  reference/    errors, notes, SHOW commands, server options, HTTP API, DEP CLI
examples/       the demo clinic (clinic.sql) and the guided tour
playground/     DEP.js: the DEP SQL Playground, its tests and the compatibility test
site/           the source of the website
branding/       logos and icons (see BRAND.md)
LICENSES/       the license texts
```

The source of DEP Engine, DEP CLI and Hoopeo Studio is in a private repository. Their installers are in the [Releases](https://github.com/mandelreouven2002/project-hoopeo.github.io/releases).

## Licensing

Each layer has its own license; see [LICENSING.md](LICENSING.md) and [NOTICE](NOTICE).

| Layer | License |
|---|---|
| Specification, documentation | CC BY 4.0 |
| DEP.js and the playground, the examples | Apache License 2.0 |
| DEP Engine, DEP CLI, Hoopeo Studio (binaries) | Project Hoopeo Evaluation License: evaluation, research, teaching and non-production testing |
| Production and commercial use | A commercial license, on request |

The names and logos are covered by [BRAND.md](BRAND.md). Third-party libraries keep their own licenses: see [THIRD-PARTY.md](THIRD-PARTY.md).

---

**Experimental.** Project Hoopeo is an experimental, conceptual project. See the [terms of use](https://mandelreouven2002.github.io/project-hoopeo.github.io/legal/terms/). By Reuven Mandelovich, [lp.reuven.click](https://lp.reuven.click).
