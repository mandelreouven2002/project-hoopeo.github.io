# Contributing

Project Hoopeo implements Database-Enforced Privacy (DEP): a specification, DEP Engine, DEP CLI, DEP.js and Hoopeo Studio. This repository holds the open layers: the specification, the documentation, DEP.js and the playground, the examples, and the website. Critical reading and hard questions are as welcome as code.

## What accepts contributions

Contributions are accepted to the public layers only:

- the specification and the design proposal (`docs/design/`);
- the guides and the reference (`docs/guides/`, `docs/reference/`);
- DEP.js and the DEP SQL Playground (`playground/`);
- the examples (`examples/`);
- the website (`site/`).

DEP Engine, DEP CLI and Hoopeo Studio are developed in a private repository and do not accept contributions. Bug reports and questions about them are welcome as issues here.

## Ways to help

- **Challenge the design.** Open an issue explaining where a rule is unclear, inconsistent, or too strict to be practical.
- **Point to prior art.** If a system or paper already does part of what DEP proposes, open an issue with a link. The comparison in the roadmap will be updated, and you will be credited.
- **Propose rules for the open questions** listed in the roadmap.
- **Report a difference between DEP.js and DEP Engine.** The playground should give the same result as the engine for every statement; a case that does not is a bug in DEP.js.
- **Find bypasses.** Please report those privately, as described in [`SECURITY.md`](SECURITY.md).

## Discussion norms

Be specific: a schema, a query, and what you expect is worth more than a general objection. Disagreement is welcome; personal remarks are not.

## Credit

Contributors whose feedback changes the design are acknowledged in the document's acknowledgements and in the release notes.

## Licensing of contributions

Each directory keeps its licence (see [`LICENSING.md`](LICENSING.md)). A contribution is made under the licence of the directory it goes to, and there is no contributor licence agreement:

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

`npm run compat` compares DEP.js with a running DEP Engine; it needs an engine binary, which is distributed under the Evaluation License (see [`playground/README.md`](playground/README.md)).
