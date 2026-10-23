# Install

DEP Engine, DEP CLI and Hoopeo Studio are distributed as binaries for Linux and macOS, and DEP Engine as a Docker image. DEP.js, the playground, is a web page. Every download is a file of the [latest release](https://github.com/mandelreouven2002/project-hoopeo.github.io/releases/latest) on the public repository, and every release has a `SHA256SUMS` file. Replace `<version>` in the file names below with the version of the release, for example `0.5.2`.

<p class="muted"><span class="badge">Experimental</span> The binaries are under the Project Hoopeo Evaluation License: evaluation, research, teaching and non-production testing, with invented data only. See <a href="../../LICENSING.md">Licensing</a>. Nothing here is a statement that the software is ready for production.</p>

## Check a download

Download `SHA256SUMS` next to the files you want, and check them:

```sh
sha256sum -c --ignore-missing SHA256SUMS         # Linux
shasum -a 256 -c --ignore-missing SHA256SUMS     # macOS
```

Each line should say `OK`. A file that fails the check must not be installed.

<div class="tabs" markdown="1">
<div class="tablist" role="tablist" aria-label="Platform" markdown="1">
<a role="tab" href="#linux" aria-selected="true">Linux</a>
<a role="tab" href="#macos" aria-selected="false">macOS</a>
<a role="tab" href="#docker" aria-selected="false">Docker</a>
<a role="tab" href="#windows" aria-selected="false">Windows</a>
<a role="tab" href="#depjs" aria-selected="false">DEP.js</a>
</div>

<section class="tabpanel" id="linux" markdown="1">

### Linux

Builds for x86-64 (`amd64`, `x86_64`) and ARM (`arm64`, `aarch64`), for systemd distributions with glibc 2.34 or later (Ubuntu 22.04, Debian 12 and newer).

**Debian packages.**

| File | Contains |
|---|---|
| `dep-engine_<version>_amd64.deb` | The server, as a systemd service, with the demo clinic in `/usr/share/dep-engine/examples` |
| `dep-cli_<version>_amd64.deb` | `dep-cli`, with the examples in `/usr/share/dep-cli/examples` |
| `hoopeo-studio_<version>_amd64.deb` | Hoopeo Studio. It needs WebKitGTK 4.1 (`libwebkit2gtk-4.1-0`) and GTK 3 |

```sh
sudo apt install ./dep-engine_<version>_amd64.deb ./dep-cli_<version>_amd64.deb
```

The engine package creates the `dep-engine` user, generates the admin password, and starts the service on `127.0.0.1:5433`.

- **Configuration:** `/etc/dep-engine/dep-engine.env`. After a change: `sudo systemctl restart dep-engine`. The [server options reference](../../docs/reference/server-options.md) lists the variables.
- **Data and key:** `/var/lib/dep-engine/data`, owned by the `dep-engine` user and not readable by others.
- **Admin password:** `/etc/dep-engine/admin-password`, readable by root and the `dep-engine` group.
- **Logs:** `journalctl -u dep-engine`.
- **Start and stop:** `sudo systemctl stop dep-engine`, `sudo systemctl start dep-engine`, `systemctl status dep-engine`.
- **Remove:** `sudo apt remove dep-engine`. The database is kept; `sudo apt purge dep-engine` also removes the configuration, and the data in `/var/lib/dep-engine` stays until you delete it.

Connect, and load the demo clinic into a new database by setting `DEP_INIT=/usr/share/dep-engine/examples/clinic.sql` in the environment file before the first start:

```sh
DEP_PASSWORD=$(sudo cat /etc/dep-engine/admin-password) dep-cli -h 127.0.0.1 -p 5433 -U admin -d operational
```

**Tarballs, for any systemd distribution.**

| File | Contains |
|---|---|
| `dep-engine-<version>-linux-x86_64.tar.gz` | `dep-engine` and `dep-cli`, the examples, and `install.sh` |
| `dep-cli-<version>-linux-x86_64.tar.gz` | `dep-cli` and the examples |

```sh
tar xzf dep-engine-<version>-linux-x86_64.tar.gz
cd dep-engine-<version>-linux-x86_64
sudo ./install.sh              # installs the service and starts it
sudo ./install.sh --uninstall  # removes it, and keeps /etc/dep-engine and /var/lib/dep-engine
```

Without `install.sh`, `./bin/dep-engine --help` lists the options, and the engine runs from the folder.

</section>

<section class="tabpanel" id="macos" markdown="1">

### macOS

A universal package, for Apple silicon and Intel, macOS 11 or later.

| File | Contains |
|---|---|
| `Project-Hoopeo-<version>-macos-universal.pkg` | All the products, with choices under Customize |
| `Hoopeo-Studio-<version>-macos-universal.pkg` | Hoopeo Studio only |
| `DEP-CLI-<version>-macos-universal.pkg` | DEP CLI, the DEP CLI app that opens it in Terminal, and the examples |
| `DEP-Engine-<version>-macos-universal.pkg` | The server, and optionally a background service |

**Gatekeeper.** A release that is not signed and notarised, which the release notes say, opens with a warning. Control-click (or right-click) the package and choose Open, or open System Settings, Privacy & Security, and choose Open Anyway after the first attempt.

**The background service.** If you choose "Run DEP Engine in the background", a launch agent starts the server when you log in, on `127.0.0.1:5433`, with its data in `~/Library/Application Support/Project Hoopeo/data`. The admin password is in `~/Library/Application Support/Project Hoopeo/admin-password`, readable only by you. DEP CLI and Hoopeo Studio then connect to it.

```sh
DEP_PASSWORD=$(cat ~/Library/Application\ Support/Project\ Hoopeo/admin-password) dep-cli -h 127.0.0.1 -p 5433 -U admin -d operational
launchctl bootout gui/$(id -u)/click.reuven.hoopeo.dep-engine    # stop the service for this session
```

The service logs to `/tmp/dep-engine.log`.

**Where things go.** `/usr/local/bin/dep-engine` and `dep-cli`, `/Applications/Hoopeo Studio.app` and `DEP CLI.app`, and the examples in `/usr/local/share/project-hoopeo`.

**Remove.** `/usr/local/share/project-hoopeo/uninstall.sh` removes everything and keeps your databases in `~/Library/Application Support/Project Hoopeo`.

</section>

<section class="tabpanel" id="docker" markdown="1">

### Docker

The image of DEP Engine is public, for `linux/amd64` and `linux/arm64`: `ghcr.io/mandelreouven2002/dep-engine`, tagged with the version and `latest`.

```sh
docker run -d --name dep-engine -p 5433:5433 -v dep-data:/var/lib/dep-engine \
  -e DEP_ADMIN_PASSWORD=change-me \
  -e DEP_INIT=/usr/share/project-hoopeo/examples/clinic.sql \
  ghcr.io/mandelreouven2002/dep-engine:latest
```

- **The volume** at `/var/lib/dep-engine` holds the database and its master key. To keep the key apart from the data, mount it elsewhere and set `DEP_KEY_FILE`.
- **The admin password** comes from `DEP_ADMIN_PASSWORD`. Without it, a random one is printed in the log (`docker logs dep-engine`) when the database is created.
- **TLS** is on by default, with a self-signed certificate (`DEP_TLS_SELF_SIGNED=1`). Connect with `--sslmode require`. For your own certificate, mount it and set `DEP_TLS_CERT` and `DEP_TLS_KEY`, and `DEP_TLS_REQUIRED=1` to refuse plain connections.
- **Ports.** 5433 is the PostgreSQL protocol. For the HTTP API set `DEP_HTTP_LISTEN=0.0.0.0:8433` and publish 8433. Publish only to `127.0.0.1` (`-p 127.0.0.1:5433:5433`) unless the network needs it.
- **Resources** follow the container's memory limit: `DEP_MEMORY_LIMIT`, `DEP_CACHE_SIZE`, `DEP_MAX_CONNECTIONS`, `DEP_MATCH_LIMIT`.

```sh
dep-cli -h 127.0.0.1 -p 5433 -U admin -d operational --sslmode require
```

The release also has `dep-engine-<version>-docker-<arch>.tar.gz` where a registry is not an option: a build context with the binaries and a `compose.yaml`. Unpack it, then `echo 'DEP_ADMIN_PASSWORD=change-me' > .env && docker compose up -d`. The image is for evaluation, under the Evaluation License; its notices are in `/usr/share/doc/project-hoopeo`.

</section>

<section class="tabpanel" id="windows" markdown="1">

### Windows

There is no Windows build yet. DEP Engine and DEP CLI run on Windows through Docker Desktop (the image above) or inside WSL 2 (install the Linux packages in an Ubuntu distribution). Hoopeo Studio has no Windows build. The playground runs in any browser, on any system.

</section>

<section class="tabpanel" id="depjs" markdown="1">

### DEP.js and the playground

DEP.js needs no installation. Open the [playground](../../playground/) in a browser.

- **One file:** `dep-sql-playground-<version>.html` in the release (the same page is at [`playground/dep-sql-playground.html`](../../playground/dep-sql-playground.html) on this site). It works from disk and offline.
- **A folder:** `npm run build` in the [`playground/` folder](https://github.com/mandelreouven2002/project-hoopeo.github.io/tree/main/playground) of the repository writes `site/`, which any static server can serve. Browsers do not load WebAssembly from `file://`, so serve it (`npm start`).
- **As a library:** `depsql.js` runs in Node or in a page, on top of sql.js. See the [playground guide](playground-guide.md#using-depsqljs-as-a-library). DEP.js is open source under the Apache License 2.0.

</section>
</div>

## Running a server

A DEP Engine that other people or programs connect to needs more than the defaults. This is guidance for an experimental system that holds invented data, and not a recipe for protecting real data. The [administration guide](../../docs/guides/administration.md) has the details.

- **TLS.** Give the server a certificate and key (`--tls-cert`, `--tls-key`), and refuse plain connections (`--tls-required`). A self-signed certificate (`--tls-self-signed`) is acceptable for a test, with clients using `verify-full` and the certificate as the CA.
- **Keep the data and the key apart.** `--key-file` puts `master.key` on another volume, with its own permissions. A copy of the data alone is then only ciphertext and index entries.
- **Back up** the data directory and the key file, with the server stopped or from a filesystem snapshot, and try the restore.
- **Limits.** `--memory-limit`, `--cache-size` and `--max-connections` sized for the machine. `--match-limit 1000/minute` as the default for every role that is not a superuser, and a limit per role with `ALTER ROLE`.
- **One role per application**, each granted only its endpoint, columns and functions. Keep `admin` for administration.
- **Upgrade** by replacing the binary or the package. Older data directories are converted when first opened. Back up first.
- **Bind carefully.** `127.0.0.1` accepts connections from the machine only. Open the port to a network only with TLS and a firewall.

## Local development

You need no server to try DEP:

```sh
dep-cli --memory --init examples/clinic.sql          # the demo clinic, in memory
dep-cli --data-dir ./data --init examples/clinic.sql # kept in a folder
```

Hoopeo Studio holds its own databases: **Session > New Local Database** creates one, in a folder or in memory, and can serve it on a port for other tools. To load the clinic, open `examples/clinic.sql` (File > Open) and run it with Execute. The `examples/` folder is in the installers, and in the [repository](https://github.com/mandelreouven2002/project-hoopeo.github.io/tree/main/examples).
