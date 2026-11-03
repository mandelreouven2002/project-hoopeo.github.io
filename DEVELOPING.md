# Developing Project Hoopeo

How to build, test and release DEP Engine, DEP CLI and Hoopeo Studio from the source in this repository. For what the project is and how to use it, see [README.md](README.md).

| Product | Where | Notes |
|---|---|---|
| DEP Engine | `engine/` | The library (`dep_engine`) and the server (`dep-engine`). Rust, Apache DataFusion 55, redb, pgwire, Axum |
| DEP CLI | `packages/dep-cli/` | `dep-cli`, with DEP Engine built in |
| Hoopeo Studio | `studio/` | Tauri 2: `src/` in Rust, `ui/` the window |

The source is under the Business Source License 1.1 (`LICENSES/BUSL-1.1.txt`): free for individuals, non-profit, educational and research use, and for non-production use by anyone; a company needs a commercial license for production or commercial use. See `LICENSING.md`.

## Layout

```
engine/            DEP Engine, with its tests in engine/tests/
packages/dep-cli/  DEP CLI
studio/            Hoopeo Studio
packaging/         installers: macos/, linux/, docker/, source/, icons/, about.* (cargo-about)
tools/             scripts that make the documentation images
playground/        DEP.js, its tests and the compatibility test (Apache 2.0)
examples/          the demo clinic and the guided tour
docs/, site/       the documentation and the website
try-it.sh          run the CLI, the tour, the server or Studio from source
.github/workflows/ ci.yml, release.yml, pages.yml and playground.yml
```

The engine tests read `examples/clinic.sql`, and the installers copy the examples and the icons from `examples/` and `branding/`.

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
target/release/dep-engine --http-listen 127.0.0.1:8433 --init examples/clinic.sql --admin-password admin &
cd playground && npm ci && npm test
DEP_ENGINE_URL=http://127.0.0.1:8433 DEP_ADMIN_PASSWORD=admin npm run compat
```

Start the server afresh for each run: the cases change its data.

## Releasing

Releases are built by `.github/workflows/release.yml` when a tag `v*` is pushed, and published as a GitHub Release of this repository. The workflow's own token is enough: no secret is needed.

1. Bump the version in `Cargo.toml` (`[workspace.package]`), `studio/tauri.conf.json`, `playground/package.json`, `playground/depsql.js` (`VERSION`). Add the entry to `CHANGELOG.md`.
2. Push to `main`, and wait for `ci.yml` to pass.
3. Tag and push the tag:
   ```sh
   git tag v0.5.3 && git push origin v0.5.3
   ```
4. The workflow then runs: **prepare** (all of `ci.yml`, and `THIRD-PARTY-NOTICES` with cargo-about), **linux** (`.deb` and `tar.gz` on `ubuntu-22.04` and `ubuntu-22.04-arm`), **macos** (universal `.pkg`), **docker** (the image from the Linux tarballs, to GHCR), **playground** (the single-file page and `depsql.js`), **checksums** (`SHA256SUMS`) and **publish** (the GitHub Release `v<version>`, with the CHANGELOG section as its notes).
5. Check the release: every asset is there, `sha256sum -c SHA256SUMS` passes, a `.deb` installs on a clean Ubuntu container, and the Docker image starts with `examples/clinic.sql`.

### Secrets

| Secret | Needed for |
|---|---|
| `MACOS_SIGN_APP`, `MACOS_SIGN_PKG` | Optional. The names of the Developer ID Application and Installer certificates, which must be in the runner's keychain. Without them the macOS packages are built unsigned, and the release notes say so |
| `MACOS_NOTARY_PROFILE` | Optional. A `notarytool` keychain profile, to notarize the signed packages |

The `GITHUB_TOKEN` of the workflow is enough to push the Docker image to GHCR. After the first push, open the package `dep-engine` on GitHub and make it public, and link it to this repository (the image carries the `org.opencontainers.image.source` label that does this).

### Third-party notices

`THIRD-PARTY-NOTICES` is generated with `cargo install cargo-about --locked --features cli` and `cargo about generate packaging/about.hbs -c packaging/about.toml > THIRD-PARTY-NOTICES`, and committed, because Hoopeo Studio embeds it. Regenerate it when dependencies change.

## Licensing

Business Source License 1.1; a commercial license is available on request through <https://lp.reuven.click>. See `LICENSING.md` and `NOTICE`. Pull requests to this source are not accepted yet (`CONTRIBUTING.md`).
