# Docs

Guides, a reference, and the design documents of Project Hoopeo. The guides are written for someone using DEP Engine, DEP CLI or the playground. Every SQL example in them was run against DEP Engine, and the output shown is the real output, DEP notes included.

**New to DEP?** Start with [DEP SQL in five minutes](../../docs/guides/overview.md), then try the [playground](../../playground/). Terms you do not know are explained in the [glossary](../../docs/reference/glossary.md).

## Start here

| Guide | What it covers |
|---|---|
| [DEP SQL in five minutes](../../docs/guides/overview.md) | What DEP SQL is and why: the problem, the idea, the three things it adds to SQL, and what four users of a clinic see, with the real output of DEP Engine |
| [Getting started](../../docs/guides/getting-started.md) | Install, start a server with the demo clinic, connect with DEP CLI, `psql` and the HTTP API, and run a first lookup |

## Guides

| Guide | What it covers |
|---|---|
| [Concepts](../../docs/guides/concepts.md) | Protected columns, match domains, blind indexes, endpoints and access paths, matching, anchoring and linking, declared functions, the terminal rule, DEP notes, the audit log |
| [DEP SQL](../../docs/guides/dep-sql.md) | Every addition DEP makes to SQL: syntax, rules, examples and the error each rule raises |
| [Transactions, constraints and indexes](../../docs/guides/transactions-and-constraints.md) | Transactions and savepoints, transactional schema changes, isolation and the write lock, constraints, indexes, `EXPLAIN` |
| [Administration](../../docs/guides/administration.md) | Running a server: TLS, the data directory and key, resources, the audit log, key rotation, upgrades, metrics |
| [Applications](../../docs/guides/applications.md) | Connecting from applications over the PostgreSQL protocol and the HTTP API, with Python and JavaScript examples |

## Reference

| Page | What it lists |
|---|---|
| [Glossary](../../docs/reference/glossary.md) | Every term of DEP: a one-line definition, what it means, an example and where you meet it |
| [Errors](../../docs/reference/errors.md) | Every DEP code with its SQLSTATE, and the PostgreSQL SQLSTATEs DEP Engine uses |
| [DEP notes](../../docs/reference/notes.md) | Every DEP note text, and when it appears |
| [SHOW commands](../../docs/reference/show.md) | Every `SHOW` command and its columns |
| [Server options](../../docs/reference/server-options.md) | Every `dep-engine` option and environment variable |
| [HTTP API](../../docs/reference/http-api.md) | The endpoints, request and response formats, authentication and errors |
| [DEP CLI](../../docs/reference/cli.md) | The options and meta-commands of `dep-cli` |

## Design

| Document | What it is |
|---|---|
| [Design proposal](../../docs/design/01-dep-concept.md) | The full definition of DEP: the problem, the idea, the language, declared functions, the architecture, and the guarantees and limits. Version 1.0, CC BY 4.0 |
| [Roadmap and release](../../docs/design/02-roadmap-and-release.md) | A possible way to build and release DEP, and a comparison with existing products and approaches |
| [How to cite](about.md#how-to-cite) | The citation of the work |

## The playground

[Using the playground](playground-guide.md) explains the three panes, the examples and the schema tabs, and how DEP.js differs from DEP Engine. [Open the playground](../../playground/) in your browser.
