# Project Hoopeo (private repository)

The source of **DEP Engine**, **DEP CLI** and **Hoopeo Studio**, and the packaging and release workflows of Project Hoopeo. This repository is private. The public repository, <https://github.com/mandelreouven2002/project-hoopeo.github.io>, holds the specification, the documentation, DEP.js and the playground, the examples, the website, and the published installers.

| Product | Where | Notes |
|---|---|---|
| DEP Engine | `engine/` | The library (`dep_engine`) and the server (`dep-engine`). Rust, Apache DataFusion 55, redb, pgwire, Axum |
| DEP CLI | `packages/dep-cli/` | `dep-cli`, with DEP Engine built in |
| Hoopeo Studio | `studio/` | Tauri 2: `src/` in Rust, `ui/` the window |

The products are distributed as binaries under the Project Hoopeo Evaluation License (`LICENSES/LicenseRef-Hoopeo-Evaluation.txt`). The source in this repository is not licensed to the public. See `LICENSING.md`.

## Repository layout

```
engine/            DEP Engine, with its tests in engine/tests/
packages/dep-cli/  DEP CLI
studio/            Hoopeo Studio
packaging/         installers: macos/, linux/, docker/, source/ (private use only), icons/, about.* (cargo-about)
public/            git submodule: the public repository (examples, docs, playground, branding, CHANGELOG, site)
try-it.sh          run the CLI, the tour, the server or Studio from source
.github/workflows/ ci.yml and release.yml
LICENSING.md, LICENSE, NOTICE, LICENSES/, THIRD-PARTY-NOTICES   the same as in the public repository
```

The private repository keeps no copy of anything that lives in the public one. The examples, the docs, the branding, the CHANGELOG and the playground are read from `public/`.

## Cloning

```sh
git clone --recurse-submodules git@github.com:mandelreouven2002/project-hoopeo.git
# or, in an existing clone:
git submodule update --init
# to pick up new public changes:
git submodule update --remote public
git add public && git commit -m "Update the public submodule"
```

Building and testing need `public/`: the engine tests read `public/examples/clinic.sql`, and the installers copy the examples and the icons from it.

## Developing

Rust 1.94 or later. Hoopeo Studio on Linux needs `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev`; macOS needs nothing extra.

```sh
cargo test --locked -p dep-engine              # the rules, privileges, persistence, wire, TLS, HTTP and migration tests
cargo build --locked -p dep-cli -p hoopeo-studio
./try-it.sh                                    # DEP CLI on the demo clinic, in memory
./try-it.sh tour                               # the guided tour
./try-it.sh server                             # DEP Engine on 127.0.0.1:5433, HTTPS API on 8433, admin password: secret
./try-it.sh studio                             # Hoopeo Studio
```

The compatibility test compares DEP.js with a running DEP Engine. It is part of the CI, and can be run by hand:

```sh
cargo build --release -p dep-engine
target/release/dep-engine --http-listen 127.0.0.1:8433 --init public/examples/clinic.sql --admin-password admin &
cd public/playground && npm ci && npm test
DEP_ENGINE_URL=http://127.0.0.1:8433 DEP_ADMIN_PASSWORD=admin npm run compat
```

Start the server afresh for each run: the cases change its data.

## Releasing

Releases are built by `.github/workflows/release.yml` when a tag `v*` is pushed. The workflow publishes to the public repository; nothing is ever uploaded to this repository's releases, and source archives are never published.

1. Update `public/` to the commit that has the changes of the release (docs, CHANGELOG, playground), and commit the new submodule pointer.
2. Bump the version in `Cargo.toml` (`[workspace.package]`) and `studio/tauri.conf.json`, and in the public repository in `playground/package.json`, `playground/depsql.js` (`VERSION`) and `CITATION.cff`. Add the entry to `public/CHANGELOG.md`.
3. Push to `main`, and wait for `ci.yml` to pass.
4. Tag and push the tag:
   ```sh
   git tag v0.5.2 && git push origin v0.5.2
   ```
5. The workflow then runs: **prepare** (all of `ci.yml`, and `THIRD-PARTY-NOTICES` with cargo-about), **linux** (`.deb` and `tar.gz` on `ubuntu-22.04` and `ubuntu-22.04-arm`), **macos** (universal `.pkg`), **docker** (the image from the Linux tarballs, to GHCR), **playground** (the single-file page), **checksums** (`SHA256SUMS`) and **publish** (the GitHub Release `v<version>` on the public repository, with the CHANGELOG section as its notes).
6. Check the release: every asset is there, `sha256sum -c SHA256SUMS` passes, a `.deb` installs on a clean Ubuntu container, and the Docker image starts with `examples/clinic.sql`.

### Secrets

| Secret | Where | Needed for |
|---|---|---|
| `PUBLIC_REPO_TOKEN` | This repository, Settings > Secrets and variables > Actions | Publishing the release to the public repository. A fine-grained personal access token, scoped to `mandelreouven2002/project-hoopeo.github.io` only, with Contents: read and write |
| `MACOS_SIGN_APP`, `MACOS_SIGN_PKG` | the same | Optional. The names of the Developer ID Application and Installer certificates, which must be in the runner's keychain. Without them the macOS packages are built unsigned, and the release notes say so |
| `MACOS_NOTARY_PROFILE` | the same | Optional. A `notarytool` keychain profile, to notarise the signed packages |

The `GITHUB_TOKEN` of the workflow is enough to push the Docker image to GHCR. After the first push, open the package `dep-engine` on GitHub and make it public, and link it to the public repository (the image carries the `org.opencontainers.image.source` label that does this).

### Third-party notices

`THIRD-PARTY-NOTICES` is generated with `cargo install cargo-about --locked --features cli` and `cargo about generate packaging/about.hbs -c packaging/about.toml > THIRD-PARTY-NOTICES`, and committed, because Hoopeo Studio embeds it. Regenerate it when dependencies change, and copy it to the public repository.

## Licensing

The binaries are distributed under the Project Hoopeo Evaluation License; the source is private. A commercial license is available on request through <https://lp.reuven.click>. See `LICENSING.md` and `NOTICE`. This repository does not take contributions (`CONTRIBUTING.md`).
