# Server options

`dep-engine` is the DEP Engine server. Every option can be given on the command line or in an environment variable, which is how the Linux service, the macOS launch agent and the Docker image configure it. A command-line option overrides the environment. `dep-engine --help` prints the same list.

```sh
dep-engine --help 2>&1
```

```text
usage: dep-engine [--listen 127.0.0.1:5433] [--data-dir DIR] [--key-file FILE] [--init FILE]... [--admin-password PASSWORD]
                  [--tls-cert FILE --tls-key FILE | --tls-self-signed [--tls-host NAME]...] [--tls-required]
                  [--http-listen 127.0.0.1:8433] [--match-limit 1000/minute] [--lock-timeout 30]
                  [--memory-limit 4GB] [--cache-size 1GB] [--idle-in-transaction-timeout 300] [--max-connections 200]
  --data-dir   keep the state in DIR (created if new); without it the server keeps everything in memory
  --key-file   where the master key is kept (default DIR/master.key); keep it apart from the data
  --init       a DEP SQL script run as admin, only when the database is new
  --admin-password  sets the admin password; for a new database without one, a random password is printed
  --init-admin-password-file  for a new database only: the admin password is read from FILE
  --tls-cert, --tls-key  a PEM certificate chain and private key: clients may then connect with TLS
  --tls-self-signed      create a self-signed certificate (DIR/tls/server.crt) for localhost and each --tls-host
  --tls-required         refuse connections without TLS
  --http-listen          also serve the HTTP API (HTTPS with TLS): /v1/query, /v1/health, /metrics
  --match-limit          default limit on protected matches for roles that are not superusers, e.g. 1000/minute
  --lock-timeout         seconds a writer waits for the write lock (default 30)
  --memory-limit         memory for running queries; beyond it they spill to encrypted files (default: 1/4 of RAM)
  --cache-size           page cache of the data file (default: 1/8 of RAM, at most 4GB)
  --idle-in-transaction-timeout  seconds before a transaction left idle is rolled back (default 300; 0: never)
  --max-connections      client connections served at once (default 200; 0: no limit)
```

## Options

| Option | Environment variable | Default | Meaning |
|---|---|---|---|
| `--listen ADDR` | `DEP_LISTEN` | `127.0.0.1:5433` | Address of the PostgreSQL wire protocol. Use `0.0.0.0:5433` to accept connections from the network, with TLS |
| `--data-dir DIR` | `DEP_DATA_DIR` | none | Keep the database in `DIR` (created if new). Without it everything is in memory and is lost when the server stops |
| `--key-file FILE` | `DEP_KEY_FILE` | `DIR/master.key` | Where the master key is kept. Keep it apart from the data |
| `--init FILE` | `DEP_INIT` | none | A DEP SQL script run as `admin`, only when the database is new. Repeat the option for several scripts; in `DEP_INIT`, separate them with colons |
| `--admin-password PW` | `DEP_ADMIN_PASSWORD` | random, for a new database | Sets the `admin` password at every start. For a new database without one, a random password is printed |
| `--init-admin-password-file FILE` | `DEP_INIT_ADMIN_PASSWORD_FILE` | none | The `admin` password of a new database, read from `FILE` |
| `--tls-cert FILE`, `--tls-key FILE` | `DEP_TLS_CERT`, `DEP_TLS_KEY` | none | A PEM certificate chain and private key. Clients may then connect with TLS |
| `--tls-self-signed` | `DEP_TLS_SELF_SIGNED` (`1`, `0`, `false`) | off | Create a self-signed certificate in `DIR/tls` (in `./tls` without `--data-dir`), valid for `localhost` and each `--tls-host` |
| `--tls-host NAME` | `DEP_TLS_HOSTS` (comma-separated) | none | A host name for the self-signed certificate. Repeat the option for several |
| `--tls-required` | `DEP_TLS_REQUIRED` (`1`, `0`, `false`) | off | Refuse connections without TLS |
| `--http-listen ADDR` | `DEP_HTTP_LISTEN` | off | Also serve the HTTP API, over HTTPS when TLS is on |
| `--match-limit N/UNIT` | `DEP_MATCH_LIMIT` | none | Default limit on protected matches for roles that are not superusers. Units: `second`, `minute`, `hour`, `day`. For example `1000/minute` |
| `--lock-timeout SECONDS` | `DEP_LOCK_TIMEOUT` | `30` | How long a writer waits for the write lock before `55P03` |
| `--memory-limit SIZE` | `DEP_MEMORY_LIMIT` | a quarter of RAM | Memory for running queries. Beyond it they spill to encrypted files |
| `--cache-size SIZE` | `DEP_CACHE_SIZE` | an eighth of RAM, at most 4 GB | Page cache of the data file |
| `--idle-in-transaction-timeout SECONDS` | `DEP_IDLE_IN_TRANSACTION_TIMEOUT` | `300` | Seconds before a transaction left idle is rolled back. `0`: never |
| `--max-connections N` | `DEP_MAX_CONNECTIONS` | `200` | Client connections served at once. `0`: no limit |

A size is a number with an optional unit: `1048576`, `512MB`, `4GB`, `2GiB`. A malformed value stops the server with a message.

In a container, the default memory and cache sizes follow the container's memory limit, not the host's.

## Behaviour to know

- `--init` runs only when the database is new. A server with a data directory runs the scripts the first time and skips them when it starts again on the same directory. A server with no `--data-dir` is new at every start, so it always runs them.
- `--admin-password` is applied at every start, so it can also reset a forgotten password.
- A data directory is locked: a second server on the same directory stops with `55006`.
- The server prints its settings and how to connect when it starts; see the [getting started guide](../guides/getting-started.md).

## Ports

| Port | Used for |
|---|---|
| 5433 | The PostgreSQL wire protocol (`--listen`) |
| 8433 | The HTTP API (`--http-listen`; the examples use this number, there is no default) |

## DEP CLI

`dep-cli` takes its own options: see the [DEP CLI reference](cli.md).
