# Contributing

Project Hoopeo implements Database-Enforced Privacy (DEP): a specification, DEP Engine, DEP CLI, DEP.js and Hoopeo Studio. This repository holds all of it: the specification, the documentation, DEP.js and the playground, the examples, the website, and the source of DEP Engine, DEP CLI and Hoopeo Studio. Critical reading and hard questions are as welcome as code.

## What accepts contributions

Pull requests are accepted for:

- the specification and the design proposal (`docs/design/`);
- the guides and the reference (`docs/guides/`, `docs/reference/`, `docs/book/`);
- DEP.js and the DEP SQL Playground (`playground/`);
- the examples (`examples/`);
- the website (`site/`).

The source of DEP Engine, DEP CLI and Hoopeo Studio (`engine/`, `packages/`, `studio/`, `packaging/`) is public, under the Business Source License 1.1, but pull requests to it are not accepted yet: the maintainer needs a contributor agreement before taking code that he may later license under the Change License or a commercial license. Bug reports, questions, and proposals for these products are welcome as issues, and a proposed change described in an issue is welcome too.

## Ways to help

- **Challenge the design.** Open an issue explaining where a rule is unclear, inconsistent, or too strict to be practical.
- **Point to prior art.** If a system or paper already does part of what DEP proposes, open an issue with a link. The comparison in the roadmap will be updated, and you will be credited.
- **Propose rules for the open questions** listed in the roadmap.
- **Report a difference between DEP.js and DEP Engine.** The playground should give the same result as the engine for every statement; a case that does not is a bug in DEP.js.
- **Find bypasses.** Please report those privately, as described in [`SECURITY.md`](SECURITY.md).

## Discussion norms

Be specific: a schema, a query, and what you expect is worth more than a general objection. Disagreement is welcome; personal remarks are not.

## Credit

Contributors whose feedback changes the design are acknowledged in the document's acknowledgments and in the release notes.

## Licensing of contributions

Each directory keeps its license (see [`LICENSING.md`](LICENSING.md)). A contribution is made under the license of the directory it goes to, and there is no contributor license agreement for these directories:

- text in `docs/` is contributed under CC BY 4.0;
- code in `playground/`, `examples/` and `site/` is contributed under the Apache License 2.0.

You keep the copyright to your contribution.

## Working on DEP.js

```sh
cd playground
npm ci
npm test            # the playground's own tests
npm run build       # writes site/ and dep-sql-playground.html
```

`npm run compat` compares DEP.js with a running DEP Engine; build the engine from this repository (see [`DEVELOPING.md`](DEVELOPING.md)) or use a released binary (see [`playground/README.md`](playground/README.md)).
