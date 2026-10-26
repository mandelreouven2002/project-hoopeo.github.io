# Licensing

Project Hoopeo is distributed in layers. The ideas, the specification and the browser tooling are open: anyone may read, cite, implement and build on them. DEP Engine, DEP CLI and Hoopeo Studio are distributed as binaries only, under an evaluation license; their source code is not public. A commercial license is available for production use.

| Layer | Source | Binaries | License | SPDX |
|---|---|---|---|---|
| Specification and documentation (`docs/`) | Public | n/a | Creative Commons Attribution 4.0 | `CC-BY-4.0` |
| DEP.js and the playground (`playground/`) | Public | Public | Apache License 2.0 | `Apache-2.0` |
| Examples (`examples/`) | Public | n/a | Apache License 2.0 | `Apache-2.0` |
| DEP Engine | Private | Public | Project Hoopeo Evaluation License | `LicenseRef-Hoopeo-Evaluation` |
| DEP CLI | Private | Public | Project Hoopeo Evaluation License | `LicenseRef-Hoopeo-Evaluation` |
| Hoopeo Studio | Private | Public | Project Hoopeo Evaluation License | `LicenseRef-Hoopeo-Evaluation` |
| Names and logos (`branding/`) | Public assets | n/a | Not licensed; see [`BRAND.md`](BRAND.md) | |
| Third-party libraries | n/a | Bundled | Their own licenses | listed in `THIRD-PARTY-NOTICES` |

The license texts are in [`LICENSES/`](LICENSES/). Each directory with its own license also has a `LICENSE` file that names it.

## What this means in practice

- **Citing or implementing DEP.** The specification and the documentation are CC BY 4.0. Anyone may implement Database-Enforced Privacy, in any product, with attribution.
- **Using DEP.js or the examples.** Apache License 2.0. Use, modify and ship them in open or closed products.
- **Using the DEP Engine, DEP CLI and Hoopeo Studio binaries.** The Evaluation License lets you install and use the unmodified binaries for evaluation, research, teaching and non-production testing, free of charge. It does not allow production use, modification, decompiling or reverse engineering (except where the law allows it regardless), sublicensing, or offering the software as a service to others. You may pass on the unmodified installers, complete, with the license, `NOTICE` and `THIRD-PARTY-NOTICES`.
- **Production or commercial use.** This needs a commercial license. See below.
- **The Evaluation License is a draft.** It will be reviewed by a lawyer before the first public release. The text in [`LICENSES/LicenseRef-Hoopeo-Evaluation.txt`](LICENSES/LicenseRef-Hoopeo-Evaluation.txt) governs.

## Commercial license

A commercial license covers production use, commercial use, embedding in a product, and offering the software as a service. Terms are agreed in writing with the copyright holder, Reuven Mandelovich: contact him through <https://lp.reuven.click>. [`LICENSES/LicenseRef-Hoopeo-Commercial.txt`](LICENSES/LicenseRef-Hoopeo-Commercial.txt) describes its availability and grants no rights by itself.

## Contributions

Contributions are accepted only to the public layers: DEP.js and the playground, the examples, the documentation and the specification. They are contributed under the license of the layer they go to (Apache License 2.0 or CC BY 4.0). There is no contributor license agreement. DEP Engine, DEP CLI and Hoopeo Studio do not accept contributions. See [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Third-party software

DEP Engine is built on Apache DataFusion and Apache Arrow (Apache License 2.0). Hoopeo Studio is built with Tauri (Apache License 2.0 or MIT) and includes CodeMirror 5 (MIT). DEP.js is built on sql.js (MIT). These components remain under their own licenses, and nothing in the licenses above limits the rights those licenses give. [`THIRD-PARTY.md`](THIRD-PARTY.md) describes each library and what it is used for; the full texts are in `THIRD-PARTY-NOTICES`, generated with `cargo about` and shipped with every installer.

This page summarizes the licenses; it is not legal advice. The license texts govern.
