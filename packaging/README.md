# Installers

The installers are published as GitHub Releases of this repository,
<https://github.com/mandelreouven2002/project-hoopeo.github.io/releases>, by the `release` workflow. The source of
DEP Engine, DEP CLI and Hoopeo Studio is public in this repository, under the Business Source License 1.1.
Every installer carries `LICENSES/BUSL-1.1.txt`, `NOTICE`, `LICENSING.md`, `BRAND.md` and `THIRD-PARTY-NOTICES`.

| Installer | Script | Where it can be built |
|---|---|---|
| macOS `.pkg`: all products with choices, and one per product | `macos/build-pkg.sh` | a Mac (Xcode command line tools), or the `macos` job of `.github/workflows/release.yml` |
| Linux `.deb`: `dep-engine` (systemd service), `dep-cli`, `hoopeo-studio` | `linux/build.sh` | Linux with `dpkg-deb`; Hoopeo Studio also needs `libwebkit2gtk-4.1-dev` |
| Linux `tar.gz`: DEP Engine with `install.sh`, DEP CLI alone | `linux/build.sh` | as above |
| Docker image of DEP Engine | `docker/Dockerfile.binary` (from the release binary; this is the published image), `docker/Dockerfile` (from source, for local use) | any machine with Docker |
| Source archive | `source/make-source.sh` | anywhere with Rust. The source is public in the repository, so a release does not need one |
| Third-party notices | `about.toml`, `about.hbs` (cargo-about) | anywhere with Rust |
| Icons | `icons/make_icons.py` (from `branding/`) | anywhere with Python and Pillow |

All scripts read the version from the workspace `Cargo.toml` and write to `dist/`.

## What each product installs

**DEP Engine** (`dep-engine`): the server. On macOS, `/usr/local/bin/dep-engine`, and with the background
service a launch agent that starts it at login on 127.0.0.1:5433, its data in
`~/Library/Application Support/Project Hoopeo/data` and its generated admin password next to it. On Linux, the
`dep-engine` systemd service, running as the `dep-engine` user, configured in `/etc/dep-engine/dep-engine.env`,
data in `/var/lib/dep-engine`, admin password in `/etc/dep-engine/admin-password`.

**DEP CLI** (`dep-cli`): the command-line tool, with DEP Engine built in, so it opens local databases without a
server. On macOS also the DEP CLI app, which opens it in Terminal (connected to the background service when it
runs). The examples go to `/usr/local/share/project-hoopeo` (macOS) or `/usr/share/dep-cli` and `/usr/share/dep-engine` (Linux).

**Hoopeo Studio**: the developer application, with DEP Engine built in. On macOS `/Applications/Hoopeo Studio.app`;
on Linux `/usr/bin/hoopeo-studio` with a desktop entry.

## Signing

Unsigned macOS packages open with a warning (right-click, Open). To sign and notarize, set `SIGN_APP`,
`SIGN_PKG` and `NOTARY_PROFILE` (see the top of `macos/build-pkg.sh`); in GitHub Actions, the secrets
`MACOS_SIGN_APP` and `MACOS_SIGN_PKG` with the certificates in the runner keychain.
