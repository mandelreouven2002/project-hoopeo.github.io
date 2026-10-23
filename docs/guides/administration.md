# Administration

This guide is for the person who runs DEP Engine as a server for other people or programs: configuration, TLS, the data directory and its key, resources, the audit log, key rotation, upgrades and metrics. DEP Engine is experimental software. Everything here is guidance for evaluation and testing, not a recipe for protecting real personal data. Use invented data only.

## Running the server

```sh
dep-engine --listen 127.0.0.1:5433 --data-dir /var/lib/dep-engine/data --http-listen 127.0.0.1:8433
```

Every option has an environment variable too, which is how the packages configure the server. The [server options reference](../reference/server-options.md) lists them all.

| Installed as | Configuration | Data | Notes |
|---|---|---|---|
| Linux `.deb` | `/etc/dep-engine/dep-engine.env`, then `sudo systemctl restart dep-engine` | `/var/lib/dep-engine/data` | A systemd service as the `dep-engine` user. The admin password of a new database is in `/etc/dep-engine/admin-password` |
| Linux tarball | `sudo ./install.sh` installs the same service | the same | `sudo ./install.sh --uninstall` removes it and keeps the data |
| macOS service | A launch agent, started at login | `~/Library/Application Support/Project Hoopeo/data` | Listens on 127.0.0.1:5433 |
| Docker | Environment variables (`DEP_ADMIN_PASSWORD`, ...) | the volume at `/var/lib/dep-engine` | Self-signed TLS by default |

Logs: `journalctl -u dep-engine` on Linux, `/tmp/dep-engine.log` for the macOS service, and `docker logs` in a container.

### The admin password

`admin` is the superuser of a new database. Set its password with `--admin-password` (at every start) or `--init-admin-password-file` (for a new database only, read from a file). With neither, a random password is printed once, when the database is created:

```sh
dep-engine --listen 127.0.0.1:5433 --data-dir /var/lib/dep-engine/data
```

```text
DEP Engine 0.5.2 listening on 127.0.0.1:5433
  data directory: /var/lib/dep-engine/data (new)
  TLS: off (use --tls-cert and --tls-key, or --tls-self-signed)
  query memory: 6144 MB (beyond it, encrypted spill files); data file cache: 3072 MB; idle transactions end after 300 s; at most 200 connections
  password for admin: d251f045a4d6029dddcc681f
  dep-cli -h 127.0.0.1 -p 5433 -U admin -d operational   (OPERATIONAL)
  dep-cli -h 127.0.0.1 -p 5433 -U admin -d analytics     (MASKED_READ)
```

Create a role for each application, grant it only the endpoint and the columns it needs, and keep `admin` for administration. See the [DEP SQL guide](dep-sql.md#roles-grants-and-endpoints).

## TLS

Without TLS, passwords and results cross the network in clear text. A server that accepts connections from other machines should use it.

- `--tls-cert FILE --tls-key FILE` use your own PEM certificate chain and private key.
- `--tls-self-signed` creates a self-signed certificate in `DATA_DIR/tls`, valid for `localhost` and each `--tls-host NAME`.
- `--tls-required` refuses connections without TLS.

The same certificate serves the HTTP API, which then runs over HTTPS. Clients choose the mode: `--sslmode require` encrypts, and `verify-full` also checks the certificate against a CA file (`--sslrootcert`; for a self-signed certificate, the certificate itself).

```sh
dep-engine --listen 127.0.0.1:5433 --data-dir /var/lib/dep-engine/data --tls-self-signed --tls-required --admin-password admin
export DEP_PASSWORD=admin
# Without TLS
dep-cli -h 127.0.0.1 -p 5433 -U admin -d operational --sslmode disable -c "SELECT 1;" 2>&1
# Verified TLS, with the server's own certificate as the CA
dep-cli -h localhost -p 5433 -U admin -d operational --sslmode verify-full \
        --sslrootcert /var/lib/dep-engine/data/tls/server.crt -c '\conninfo'
```

```text
ERROR:  this DEP Engine server accepts only TLS connections
HINT:  Connect with sslmode=require (dep-cli --sslmode require).
SQLSTATE:  28000
Role "admin", endpoint "operational", access path OPERATIONAL, server localhost:5433.
```

Keep the private key readable only by the server's user. The packages do this for the files they create.

## The data directory and the master key

The data directory holds the database (`data.redb`), the audit log (`audit.redb`), a lock file and, with `--tls-self-signed`, the certificate. The **master key** (`master.key`) is the secret from which every protected value's index key and encryption key derive. Without it the protected values cannot be read or matched, and with it anyone with the data can read them.

- By default the key is in the data directory. **Keep it apart:** `--key-file /etc/dep-engine/master.key` on another volume, with its own permissions and its own backup. A stolen backup of the data alone then holds only ciphertext and index entries.
- Neither file is useful without the other, so back up both, and test the restore.
- Back up while the server is stopped, or from a filesystem snapshot. Copying `data.redb` while the server writes can produce a file that does not open.
- The key file and the data directory should be readable only by the server's user.

```sh
dep-engine --data-dir /var/lib/dep-engine/data --key-file /etc/dep-engine/master.key --admin-password admin
find /var/lib/dep-engine /etc/dep-engine -type f | sort
```

```text
/etc/dep-engine/master.key
/var/lib/dep-engine/data/.lock
/var/lib/dep-engine/data/audit.redb
/var/lib/dep-engine/data/data.redb
```

Data persists across restarts when `--data-dir` is set. Without it the database lives in memory and is lost when the server stops.

## Resources and limits

| Option | Default | Effect |
|---|---|---|
| `--memory-limit` | a quarter of RAM | Memory for running queries. Beyond it, sorts and aggregations spill to **encrypted** temporary files, since they can hold values a disclosing function released. A hash join whose smaller side exceeds it fails with `53200`. |
| `--cache-size` | an eighth of RAM, at most 4 GB | Page cache of the data file |
| `--max-connections` | 200 | Client connections served at once (`0`: no limit) |
| `--idle-in-transaction-timeout` | 300 s | A transaction left idle is rolled back (`0`: never) |
| `--lock-timeout` | 30 s | How long a writer waits for the write lock before `55P03` |
| `--match-limit` | none | Default limit on protected matches for roles that are not superusers, for example `1000/minute` |

In a container the defaults follow the container's memory limit, not the host's. One transaction writes at a time; readers never wait.

### Match limits

A lookup on a protected column answers the question "is this value here?", and a client that can ask without limit can guess every value. Set a default for all roles that are not superusers with `--match-limit`, and a specific limit per role with `ALTER ROLE r MATCH LIMIT 600 PER HOUR`. A statement over the limit is refused with `ZD109` and recorded. See the [DEP SQL guide](dep-sql.md#match-limits).

## The audit log

The audit log is `audit.redb`, written with group commit: a statement returns only after its records are on disk. It records:

| `kind` | When |
|---|---|
| `match` | A protected match, with the number of values matched |
| `disclose`, `link` | A call of a function declared `DISCLOSES` or `LINKS` |
| `denied` | A refused statement, with the error code |
| `rate_limited` | A statement over a match limit |
| `key_rotated` | A master key rotation |

Each record has a sequence number, a time, the session user and the current role, the endpoint and path, the object and match domain, and the count. It never holds a protected value, a disclosed value, or the text of a statement. Superusers read it with `SHOW AUDIT [n]` (the latest `n` records, default 100; DEP CLI's `\audit` shows 50). The log is not rotated or truncated by the server.

```text
reception_app@operational=> SELECT full_name FROM patients WHERE national_id IN ('123456782', '234567893');
DEP:  PROTECTED column patients.national_id is matched against a list of values through the blind index of MATCH DOMAIN person_id.
 full_name
------------
 Dana Cohen
 Yossi Levi
(2 rows)

reception_app@operational=> SELECT full_name FROM patients WHERE national_id LIKE '1%';
ERROR:  LIKE is not supported for PROTECTED values (patients.national_id).
HINT:  PROTECTED values support only full-value equality matching. To release a value, use a disclosing function.
SQLSTATE:  ZD101 (DEP_PROTECTED_OPERATION)

reception_app@operational=> \c operational billing_job
You are now connected as "billing_job" through endpoint "operational" (OPERATIONAL).

billing_job@operational=> SELECT count(*) FROM monthly_fund_report(DATE '2026-10-01');
DEP:  monthly_fund_report() is declared DISCLOSES person_id: the columns it declares are returned in plaintext.
 count(*)
----------
 10
(1 row)

billing_job@operational=> \c operational admin
You are now connected as "admin" through endpoint "operational" (OPERATIONAL).

admin@operational=> SHOW AUDIT 10;
 seq |            at            | session_user  |     role      |  endpoint   |    path     |   kind   |        object         |  domain   | count |         detail
-----+--------------------------+---------------+---------------+-------------+-------------+----------+-----------------------+-----------+-------+-------------------------
 3   | 2026-10-23T12:27:12.903Z | billing_job   | billing_job   | operational | OPERATIONAL | disclose | monthly_fund_report() | person_id | NULL  | NULL
 2   | 2026-10-23T12:27:12.900Z | reception_app | reception_app | operational | OPERATIONAL | denied   | SELECT                | NULL      | NULL  | DEP_PROTECTED_OPERATION
 1   | 2026-10-23T12:27:12.899Z | reception_app | reception_app | operational | OPERATIONAL | match    | patients.national_id  | person_id | 2     | NULL
(3 rows)
```

## Key rotation

`ALTER SYSTEM ROTATE MASTER KEY` replaces the master key. Every protected value is decrypted with the old key and re-indexed and re-encrypted with a new one, and every index is rebuilt, in a single commit. A crash at any point leaves either the old key and data or the new ones. It needs a superuser, and it cannot run in a transaction block. Take a backup of the key file and the data first; the old key stops working.

```text
admin@operational=> ALTER SYSTEM ROTATE MASTER KEY;
DEP:  The master key was replaced: 18 protected values were indexed and encrypted again with the new key, and the indexes were rebuilt. The old key no longer opens this data.
ALTER SYSTEM

admin@operational=> SELECT full_name FROM patients WHERE national_id = '123456782';
DEP:  PROTECTED column patients.national_id is matched by equality through the blind index of MATCH DOMAIN person_id.
 full_name
------------
 Dana Cohen
(1 row)
```

With `--key-file`, the new key is written to the key file. The rotation is recorded in the audit log.

## Upgrading and migrating

Replace the binary or the package and restart. Older data directories are converted the first time they are opened:

- a **0.4** directory is converted in place (rows are re-encoded, indexes are built, the audit log moves to `audit.redb`);
- a **0.3** directory is converted into a new `data.redb`, and its old files are kept in `legacy-0.3/`;
- a primary or unique key whose existing rows have duplicates (0.4 did not enforce keys) is kept, but not enforced. The server says so when it starts, and `SHOW INDEXES` shows `enforced` empty. Remove the duplicates, then run `REINDEX TABLE t`.

Back up the data directory and the key before upgrading. A directory converted by a newer version is not guaranteed to open in an older one, so do not move back.

## Metrics

`GET /metrics` on the HTTP port (no authentication, no data) returns counters in the Prometheus text format, and `GET /v1/health` returns `{"status":"ok", ...}`. Both are meant for a local monitor or a firewall-protected port.

| Metric | Meaning |
|---|---|
| `dep_statements_total`, `dep_commits_total`, `dep_rollbacks_total` | Statements and transactions |
| `dep_protected_matches_total`, `dep_disclosures_total`, `dep_links_total` | Protected matches, and calls of `DISCLOSES` and `LINKS` functions |
| `dep_rate_limited_total` | Statements refused by a match limit |
| `dep_index_lookups_total`, `dep_full_scans_total`, `dep_rows_read_total` | How tables were read |
| `dep_idle_transactions_ended_total` | Transactions ended by the idle timeout |
| `dep_wire_connections_total`, `dep_wire_connections_active`, `dep_http_requests_total` | Connections and requests |
| `dep_tables`, `dep_rows`, `dep_persistent` | Size of the database, and whether it is kept on disk |
| `dep_query_memory_limit_bytes`, `dep_spilled_bytes_total` | Query memory and the bytes spilled to encrypted files |
| `dep_errors_total{code="..."}` | Failed statements, by SQLSTATE or DEP code |

```sh
curl -s http://127.0.0.1:8433/metrics | grep -v '^#' | head -12
```

```text
dep_statements_total 40
dep_protected_matches_total 0
dep_disclosures_total 0
dep_links_total 0
dep_rate_limited_total 0
dep_commits_total 38
dep_rollbacks_total 0
dep_wire_connections_total 1
dep_wire_connections_active 0
dep_http_requests_total 0
dep_index_lookups_total 0
dep_full_scans_total 0
```

## A checklist for an evaluation server

Experimental software calls for caution, so treat this as the minimum for a shared evaluation server, and not as a statement that anything is production-ready.

1. Invented data only. The Evaluation License forbids real personal data.
2. TLS on, and `--tls-required` if clients are remote.
3. A strong admin password, and one role per application.
4. The master key on a separate volume, with its own permissions.
5. A match limit as the default.
6. Memory and connection limits sized for the machine.
7. The HTTP port bound to `127.0.0.1` or behind a firewall.
8. Backups of the data and the key, tested by a restore.
9. Someone reads `SHOW AUDIT` and `/metrics`.
