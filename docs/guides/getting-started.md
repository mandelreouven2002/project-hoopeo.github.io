# Getting started

This guide installs DEP Engine, loads the demo clinic, connects to it in three ways, and runs a first lookup. It takes about ten minutes. Everything here is experimental software: use invented data only.

Database-Enforced Privacy (DEP) is a way of building a database so that privacy rules are enforced by the engine on every query. DEP Engine is the reference server that implements it, DEP CLI is its command-line shell, and Hoopeo Studio is its developer application. The [concepts guide](concepts.md) explains the model; this one shows it working.

## 1. Install

Download the installer for your system from the [latest release](https://github.com/mandelreouven2002/project-hoopeo.github.io/releases/latest), and check it against `SHA256SUMS`. The [Install page](../../site/content/install.md) describes every package. In short:

- **Linux:** `sudo apt install ./dep-engine_<version>_amd64.deb ./dep-cli_<version>_amd64.deb`. The engine becomes a systemd service.
- **macOS:** open `Project-Hoopeo-<version>-macos-universal.pkg` and choose the parts you want.
- **Docker:** `docker run -p 5433:5433 -e DEP_ADMIN_PASSWORD=change-me ghcr.io/mandelreouven2002/dep-engine:latest`.
- **Without installing:** the playground runs the same rules in your browser, with no server. See the [playground guide](../../playground/README.md).

The binaries are distributed under the Project Hoopeo Evaluation License: evaluation, research, teaching and non-production testing, with synthetic data only. See [`LICENSING.md`](../../LICENSING.md).

The demo clinic is in `examples/clinic.sql`, in the [repository](https://github.com/mandelreouven2002/project-hoopeo.github.io/blob/main/examples/clinic.sql). The installers also carry it: `/usr/share/dep-engine/examples` and `/usr/share/dep-cli/examples` on Linux, `/usr/local/share/project-hoopeo/examples` on macOS.

## 2. Start a server with the clinic

`--init` runs a script as `admin` when the database is new. Without `--data-dir` the data stays in memory and is lost when the server stops, which is what you want for a first try.

```sh
dep-engine --init examples/clinic.sql --admin-password admin --http-listen 127.0.0.1:8433
```

The server prints how to connect. The memory figures depend on the machine:

```sh
(dep-engine --listen 127.0.0.1:5433 --init examples/clinic.sql --admin-password admin --http-listen 127.0.0.1:8433 & P=$!; sleep 1; kill $P) 2>&1
```

```text
DEP Engine 0.5.2 listening on 127.0.0.1:5433
  data kept in memory: it is lost when the server stops (use --data-dir to keep it)
  TLS: off (use --tls-cert and --tls-key, or --tls-self-signed)
  query memory: 6144 MB (beyond it, encrypted spill files); data file cache: 3072 MB; idle transactions end after 300 s; at most 200 connections
  HTTP API: http://127.0.0.1:8433/v1/query  (health: /v1/health, metrics: /metrics)
  dep-cli -h 127.0.0.1 -p 5433 -U admin -d operational   (OPERATIONAL)
  dep-cli -h 127.0.0.1 -p 5433 -U admin -d analytics     (MASKED_READ)
```

Leave it running. The clinic has three tables (`patients`, `appointments`, `medical_records`), eight roles whose password is `demo`, and two endpoints. An endpoint is the "database name" a client connects to, and it fixes the access path: `operational` (OPERATIONAL, for applications that find and update records) and `analytics` (MASKED_READ, for dashboards and agents that only read).

## 3. Connect

The user name is the **role**, and the database name is the **endpoint**. There are three ways in.

### DEP CLI

```sh
dep-cli -h 127.0.0.1 -p 5433 -U reception_app -d operational
```

The password comes from `DEP_PASSWORD`, `PGPASSWORD` or a prompt. For a first try without a server, DEP CLI can hold the clinic itself, in memory:

```sh
dep-cli --memory --init examples/clinic.sql -U reception_app
```

### psql, or any PostgreSQL driver

DEP Engine speaks the PostgreSQL wire protocol, so `psql` works as it is. The DEP notes of each statement arrive as ordinary `NOTICE` messages.

```sh
export PGPASSWORD=demo
psql "host=127.0.0.1 port=5433 user=reception_app dbname=operational sslmode=disable" \
     -c "SELECT full_name, national_id FROM patients WHERE national_id = '123456782'"
```

```text
 full_name  | national_id 
------------+-------------
 Dana Cohen | 
(1 row)

NOTICE:  DEP: PROTECTED column patients.national_id is matched by equality through the blind index of MATCH DOMAIN person_id.
NOTICE:  DEP: Output column national_id is PROTECTED column patients.national_id (MATCH DOMAIN person_id) and is returned as NULL.
```

`sslmode=disable` is for this local try. See the [administration guide](administration.md) for TLS.

### The HTTP API

```sh
curl -s -u reception_app:demo "http://127.0.0.1:8433/v1/query?endpoint=operational" \
     -H 'Content-Type: application/json' \
     -d '{"sql": "SELECT full_name, national_id FROM patients WHERE national_id = $1", "params": ["123456782"]}'
```

```text
{"endpoint":"operational","open_transaction_rolled_back":false,"path":"OPERATIONAL","results":[{"columns":["full_name","national_id"],"elapsed_ms":3.13,"notes":["PROTECTED column patients.national_id is matched by equality through the blind index of MATCH DOMAIN person_id.","Output column national_id is PROTECTED column patients.national_id (MATCH DOMAIN person_id) and is returned as NULL."],"row_count":1,"rows":[["Dana Cohen",null]],"statement":"SELECT full_name, national_id FROM patients WHERE national_id = $1"}],"role":"reception_app"}
```

The [HTTP API reference](../reference/http-api.md) describes the request and the response.

## 4. A first lookup

Reception finds a patient by the ID number she presents. The query below filters on a protected column, and also selects it.

```text
reception_app@operational=> SELECT full_name, national_id FROM patients WHERE national_id = '123456782';
DEP:  PROTECTED column patients.national_id is matched by equality through the blind index of MATCH DOMAIN person_id.
DEP:  Output column national_id is PROTECTED column patients.national_id (MATCH DOMAIN person_id) and is returned as NULL.
 full_name  | national_id
------------+-------------
 Dana Cohen | NULL
(1 row)
```

Three things happened, and the DEP notes name them:

1. The ID number was **matched by equality** through a blind index. The server never compared plaintext values.
2. `national_id` was **returned as NULL**. A protected value is never returned by an ordinary query, however it is selected.
3. Nothing about the statement was special. It is plain SQL, and no client had to cooperate.

Now ask for something a protected value does not allow:

```text
reception_app@operational=> SELECT full_name FROM patients WHERE national_id LIKE '1234%';
ERROR:  LIKE is not supported for PROTECTED values (patients.national_id).
HINT:  PROTECTED values support only full-value equality matching. To release a value, use a disclosing function.
SQLSTATE:  ZD101 (DEP_PROTECTED_OPERATION)
```

The error carries a `ZD101` SQLSTATE and a hint. A DEP rule never fails silently and never returns a degraded answer.

## 5. The tour

The repository includes a guided tour with six scripts: reception, doctors, a dashboard, billing, the privacy administrator and the administrator. Each statement is shown with its result and its DEP notes, so you can read the rules in action. From the repository folder:

```sh
dep-cli --memory --init examples/clinic.sql -f examples/tour/all.sql
```

## Where next

- [Concepts](concepts.md): protected columns, match domains, anchoring, declared functions.
- [DEP SQL](dep-sql.md): every addition DEP makes to SQL, with the errors each rule raises.
- [Applications](applications.md): connecting from Python and JavaScript.
- [Administration](administration.md): running a server for others to use.
