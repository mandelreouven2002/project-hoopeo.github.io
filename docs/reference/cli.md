# DEP CLI

`dep-cli` is the command-line shell of Project Hoopeo, in the style of `psql`. It has DEP Engine built in, so it can open a local database directly, and it can also connect to a DEP Engine server over the PostgreSQL wire protocol. It prints the DEP notes of each statement.

```sh
dep-cli --help 2>&1
```

```text
usage: dep-cli [-h HOST [-p PORT]] [-U ROLE] [-d ENDPOINT] [--data-dir DIR | --memory] [--key-file FILE]
               [--init FILE]... [-c SQL] [-f FILE]
               [--sslmode disable|require|verify-full] [--sslrootcert CA.pem]

  without -h:  a local database in --data-dir (default: ~/Library/Application Support/Project Hoopeo/data), or in memory with --memory
  with -h:     a DEP Engine server, over the PostgreSQL wire protocol (password from DEP_PASSWORD, PGPASSWORD, or a prompt)
```

## Options

| Option | Meaning |
|---|---|
| `-h`, `--host HOST` | Connect to a DEP Engine server on `HOST`. Without it the database is local |
| `-p`, `--port PORT` | The server's port (default `5433`) |
| `-U`, `--user ROLE` | The role (default `admin`) |
| `-d`, `--endpoint NAME` (also `-e`) | The endpoint, which fixes the access path (default `operational`) |
| `--data-dir DIR` | A local database in `DIR` (default: `~/Library/Application Support/Project Hoopeo/data` on macOS, `~/.local/share/project-hoopeo/data` on Linux) |
| `--memory` | A local database in memory, lost when the shell exits |
| `--key-file FILE` | The master key of a local database |
| `--init FILE` | A DEP SQL script, run as `admin` when a local database is new. Repeat the option for several. If the database already exists the shell says that `--init` was not run |
| `-c SQL` | Run the SQL (or meta-commands) and exit |
| `-f FILE` | Run a script and exit, printing each statement before its result |
| `--sslmode MODE` | `disable` (default), `require` or `verify-full`; also `PGSSLMODE` |
| `--sslrootcert FILE` | The CA certificate for `verify-full`; also `PGSSLROOTCERT` |

A local shell logs in without a password, like a local trusted connection in PostgreSQL: whoever can read the data directory and the key already holds the data. A remote connection always asks for one: the password comes from `DEP_PASSWORD`, `PGPASSWORD`, or a prompt.

When standard input is not a terminal, the shell runs it as a script. History is kept in `dep-cli-history.txt`, next to the default data directory.

## The prompt

The prompt names the role and the endpoint: `reception_app@operational=>`. Inside a transaction it is `=*>`, and after an error that needs `ROLLBACK` it is `=!>`. A statement that continues on another line shows `->`. Statements end with a semicolon.

## Meta-commands

A meta-command starts with a backslash, on a line of its own.

| Command | Meaning |
|---|---|
| `\c ENDPOINT [ROLE]` (also `\connect`) | Open a new connection through `ENDPOINT`, as `ROLE` (default: the current one). A remote shell asks for the password |
| `\conninfo` | Where you are connected, as whom, and on which access path |
| `\dt` | List tables (`SHOW TABLES`) |
| `\d TABLE` | Describe a table (`SHOW COLUMNS FROM TABLE`) |
| `\di [TABLE]` | List indexes (`SHOW INDEXES`) |
| `\df` | List declared functions |
| `\du` | List roles and the endpoints they may use |
| `\dp` | List privileges (`SHOW GRANTS`) |
| `\storage TABLE` | Show a table as the server stores it (superusers; `SHOW STORAGE`) |
| `\matrix` | Show the privacy matrix (`SHOW PRIVACY MATRIX`) |
| `\audit [N]` | Show the latest `N` audit records, 50 by default (superusers) |
| `\notes [on\|off]` | Print the DEP notes after each statement. On by default; `\notes` alone turns them on |
| `\i FILE` | Run a script, printing each statement before its result |
| `\echo TEXT` | Print `TEXT` |
| `\q` | Quit |
| `\?`, `\h` | Help |

```text
reception_app@operational=> \conninfo
Role "reception_app", endpoint "operational", access path OPERATIONAL, server local (in memory).

reception_app@operational=> \d appointments
     column     |   type    | match_domain | primary_key |      references       | not_null | unique
----------------+-----------+--------------+-------------+-----------------------+----------+--------
 appointment_id | BIGINT    | NULL         | yes         | NULL                  | yes      | yes
 patient_id     | BIGINT    | NULL         |             | patients (patient_id) |          |
 starts_at      | TIMESTAMP | NULL         |             | NULL                  |          |
 visit_type     | TEXT      | NULL         |             | NULL                  |          |
(4 rows)

reception_app@operational=> \notes off
DEP notes are off.

reception_app@operational=> SELECT full_name FROM patients WHERE national_id = '123456782';
 full_name
------------
 Dana Cohen
(1 row)

reception_app@operational=> \notes on
DEP notes are on.

Done.
reception_app@operational=> \c analytics bi_dashboard
You are now connected as "bi_dashboard" through endpoint "analytics" (MASKED_READ).

bi_dashboard@analytics=> \conninfo
Role "bi_dashboard", endpoint "analytics", access path MASKED_READ, server local (in memory).
```

## Scripts

`-f FILE` and `\i FILE` run a file that mixes SQL and meta-commands, which is how the guided tour works: `\c operational reception_app` switches role in the middle of a script. The tour is in the public repository, run from its folder:

```sh
dep-cli --memory --init examples/clinic.sql -f examples/tour/all.sql
```

`-c` takes a statement too:

```sh
dep-cli --memory --init examples/clinic.sql -U doctors_app -c "SELECT p.full_name, m.diagnosis FROM patients AS p JOIN medical_records AS m ON m.patient_national_id = p.national_id WHERE p.national_id = '123456782';"
```

```text
DEP:  PROTECTED column patients.national_id is matched by equality through the blind index of MATCH DOMAIN person_id.
DEP:  Join of PROTECTED columns medical_records.patient_national_id and patients.national_id (MATCH DOMAIN person_id) is anchored to a single protected value.
 full_name  |    diagnosis
------------+------------------
 Dana Cohen | Asthma
 Dana Cohen | Seasonal allergy
(2 rows)
```

## Connecting to a server

```sh
export DEP_PASSWORD=demo
dep-cli -h 127.0.0.1 -p 5433 -U reception_app -d operational -c "\conninfo"
echo "SELECT count(*) AS patients FROM patients;" | dep-cli -h 127.0.0.1 -p 5433 -U doctors_app -d operational
```

```text
Role "reception_app", endpoint "operational", access path OPERATIONAL, server 127.0.0.1:5433.

 patients
----------
 6
(1 row)
```
