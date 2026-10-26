# DEP CLI

`dep-cli`, the command-line tool of Project Hoopeo, with DEP Engine built in. It is a shell for DEP SQL in the
style of psql: it opens a local database directly, or connects to a DEP Engine server over the PostgreSQL wire
protocol.

```sh
dep-cli                                        # the local database in the default data directory
dep-cli --memory --init examples/clinic.sql    # the demo clinic, in memory
dep-cli --data-dir ./data                      # a data directory
dep-cli -h db.example.com -p 5433 -U reception_app -d operational --sslmode verify-full --sslrootcert ca.pem
```

| Option | Meaning |
|---|---|
| `-h`, `--host` and `-p`, `--port` | Connect to a DEP Engine server (default port 5433). Without `-h` the database is local. |
| `-U`, `--user` | The role (default `admin`) |
| `-d`, `--endpoint` (or `-e`) | The endpoint, which fixes the access path (default `operational`) |
| `--data-dir DIR` | A local database in DIR (default: the platform's data directory for Project Hoopeo) |
| `--memory` | A local database in memory, lost on exit |
| `--key-file FILE` | The master key of a local database |
| `--init FILE` | A script run as `admin` when a local database is new; may be repeated |
| `-c SQL`, `-f FILE` | Run SQL (or a script) and exit; `-f` prints each statement before its result |
| `--sslmode`, `--sslrootcert` | `disable`, `require` or `verify-full`, and the CA file (also `PGSSLMODE`, `PGSSLROOTCERT`) |

A local shell logs in without a password, as a local trusted connection does in PostgreSQL. The password of a
remote connection comes from `DEP_PASSWORD`, `PGPASSWORD`, or a prompt. SQL piped to standard input is run as a
script. After each statement, DEP CLI prints its DEP notes (`\notes off` hides them).

Meta-commands: `\dt`, `\d TABLE`, `\di [TABLE]`, `\df`, `\du`, `\dp`, `\matrix`, `\audit [N]`, `\storage TABLE`,
`\c ENDPOINT [ROLE]` (also `\connect`), `\conninfo`, `\i FILE`, `\notes [on|off]`, `\echo TEXT`, `\q`, and `\?`
(also `\h`). Statements end with a semicolon.

`BEGIN ... COMMIT` runs statements as one transaction; the prompt shows `=*>` inside a transaction and `=!>`
when it failed and needs ROLLBACK.

License: the binaries are distributed under the Project Hoopeo Evaluation License
(`../../LICENSES/LicenseRef-Hoopeo-Evaluation.txt`); a commercial license is available. See `../../LICENSING.md`.
