# Licensing

Project Hoopeo is distributed in layers. The ideas, the specification and the browser tooling are open: anyone may read, cite, implement and build on them. The source code of DEP Engine, DEP CLI and Hoopeo Studio is public too, under the Business Source License 1.1: free for individuals and for non-profit, educational and research organizations, and free for non-production use by anyone, including companies. A company that wants to use it in production, or commercially, needs a license from the copyright holder.

| Layer | License | SPDX |
|---|---|---|
| Specification and documentation (`docs/`) | Creative Commons Attribution 4.0 | `CC-BY-4.0` |
| DEP.js and the playground (`playground/`) | Apache License 2.0 | `Apache-2.0` |
| Examples (`examples/`) | Apache License 2.0 | `Apache-2.0` |
| DEP Engine (`engine/`) | Business Source License 1.1 | `BUSL-1.1` |
| DEP CLI (`packages/dep-cli/`) | Business Source License 1.1 | `BUSL-1.1` |
| Hoopeo Studio (`studio/`) | Business Source License 1.1 | `BUSL-1.1` |
| Packaging and tooling (`packaging/`, `tools/`) | Business Source License 1.1 | `BUSL-1.1` |
| Names and logos (`branding/`) | Not licensed; see [`BRAND.md`](BRAND.md) | |
| Third-party libraries | Their own licenses | listed in `THIRD-PARTY-NOTICES` |

The license texts are in [`LICENSES/`](LICENSES/). Each directory with its own license also has a `LICENSE` file that names it.

## What this means in practice

- **Citing or implementing DEP.** The specification and the documentation are CC BY 4.0. Anyone may implement Database-Enforced Privacy, in any product, with attribution.
- **Using DEP.js or the examples.** Apache License 2.0. Use, modify and ship them in open or closed products, including in companies. This has not changed.
- **Using the DEP Engine, DEP CLI and Hoopeo Studio as an individual, or in a non-profit, educational or research organization.** Free, including in production, as long as the use is not part of a commercial product or service or of the operations of a for-profit organization. You may read, modify and redistribute the source under the Business Source License.
- **Evaluating, developing, testing, teaching or researching, as a company.** Free. The Business Source License allows non-production use by anyone.
- **Using them in production, or commercially, as a company.** This needs a commercial license from the copyright holder. See below.
- **After four years.** Each version of the source code becomes available under the Apache License 2.0, four years after that version is first published in this repository.
- **The license is a draft.** The Business Source License 1.1 itself is a standard text. The parameters that define who may use the software in production (the Additional Use Grant, in [`LICENSES/BUSL-1.1.txt`](LICENSES/BUSL-1.1.txt)) are written by the copyright holder and will be reviewed by a lawyer. The text in that file governs.

Binaries published in releases up to 0.5.2 were distributed under the Project Hoopeo Evaluation License ([`LICENSES/LicenseRef-Hoopeo-Evaluation.txt`](LICENSES/LicenseRef-Hoopeo-Evaluation.txt)). Releases after 0.5.2 are distributed under the Business Source License 1.1.

## Commercial license

A commercial license covers production use by or for a company, commercial use, embedding in a product, and offering the software as a service. Terms are agreed in writing with the copyright holder, Reuven Mandelovich: contact him through <https://lp.reuven.click>. [`LICENSES/LicenseRef-Hoopeo-Commercial.txt`](LICENSES/LicenseRef-Hoopeo-Commercial.txt) describes its availability and grants no rights by itself.

## Contributions

Pull requests are accepted for the specification and the documentation, DEP.js and the playground, the examples and the website. They are contributed under the license of the layer they go to (Apache License 2.0 or CC BY 4.0). There is no contributor license agreement for these. Pull requests to the source of DEP Engine, DEP CLI and Hoopeo Studio are not accepted yet, because the copyright holder needs a contributor agreement first; issues and proposals are welcome. See [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Third-party software

DEP Engine is built on Apache DataFusion and Apache Arrow (Apache License 2.0). Hoopeo Studio is built with Tauri (Apache License 2.0 or MIT) and includes CodeMirror 5 (MIT). DEP.js is built on sql.js (MIT). These components remain under their own licenses, and nothing in the licenses above limits the rights those licenses give. [`THIRD-PARTY.md`](THIRD-PARTY.md) describes each library and what it is used for; the full texts are in `THIRD-PARTY-NOTICES`, generated with `cargo about` and shipped with every installer.

This page summarizes the licenses; it is not legal advice. The license texts govern.
