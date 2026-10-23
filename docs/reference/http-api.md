# HTTP API

DEP Engine can serve a small HTTP API next to the PostgreSQL protocol, for clients that cannot speak the protocol. It applies the same DEP rules, privileges, match limits and audit log as a wire connection. Start it with `--http-listen`; it uses HTTPS when the server has a TLS certificate.

| Method and path | Authentication | Purpose |
|---|---|---|
| `POST /v1/query` | Basic, as a role | Run one or more statements |
| `GET /v1/health` (also `GET /health`) | none | Status and version |
| `GET /metrics` | none | Counters in the Prometheus text format; no data |

The request body is limited to 4 MB.

## POST /v1/query

**Authentication.** HTTP Basic, with a role and its password. The role name is lower-cased. Without credentials, or with a wrong password, the answer is `401` with a `WWW-Authenticate` header.

**Endpoint.** The endpoint, which fixes the access path, is the `endpoint` query parameter or the `DEP-Endpoint` header (the header wins). The default is `operational`. A role that may not use the endpoint gets `403`.

**Body.** Either JSON, with `Content-Type: application/json`:

```json
{"sql": "SELECT full_name FROM patients WHERE national_id = $1", "params": ["123456782"]}
```

or plain text SQL with any other content type. `params` is optional; `$1`, `$2` in the SQL take its values: JSON strings, numbers, booleans and `null`. The SQL may hold several statements separated by semicolons.

**Execution.** The statements run in order on one connection, so `BEGIN; ...; COMMIT;` works in one request. They stop at the first error. A transaction left open at the end is rolled back, and the response says so.

**Response.** A JSON object:

| Field | Meaning |
|---|---|
| `role`, `endpoint`, `path` | Who ran the statements, through which endpoint, on which access path |
| `results` | One object per statement that ran |
| `open_transaction_rolled_back` | `true` if the request left a transaction open and it was rolled back |

Each result has:

| Field | Meaning |
|---|---|
| `statement` | The statement as it was sent, with `$1` and never the values |
| `elapsed_ms` | Time taken |
| `notes` | The DEP notes of the statement |
| `columns`, `rows`, `row_count` | For a statement that returns rows. Protected columns are `null` |
| `command` | For other statements, the command tag, such as `UPDATE 1` |
| `error` | If the statement failed: `code` (the DEP code, or `null`), `sqlstate`, `message`, `hint` |

**Status codes.**

| Status | When |
|---|---|
| `200` | Every statement succeeded |
| `422` | A statement failed; `results` ends with it and carries its `error` |
| `400` | The body is not valid JSON, or the SQL cannot be split |
| `401` | Missing, malformed or wrong credentials |
| `403` | The role may not connect through the endpoint, or does not exist |

Values of bound parameters are removed from every error message.

### Examples

A lookup. The protected column is `null`, and the notes say why:

```sh
curl -s -u reception_app:demo "http://127.0.0.1:8433/v1/query?endpoint=operational" \
     -H 'Content-Type: application/json' \
     -d '{"sql": "SELECT full_name, national_id FROM patients WHERE national_id = $1", "params": ["123456782"]}' | jq -c '.results[] | {statement, rows, notes}'
```

```text
{"statement":"SELECT full_name, national_id FROM patients WHERE national_id = $1","rows":[["Dana Cohen",null]],"notes":["PROTECTED column patients.national_id is matched by equality through the blind index of MATCH DOMAIN person_id.","Output column national_id is PROTECTED column patients.national_id (MATCH DOMAIN person_id) and is returned as NULL."]}
```

A refused statement, with the `422` status and the error:

```sh
curl -s -w '\nHTTP %{http_code}\n' -u reception_app:demo "http://127.0.0.1:8433/v1/query" \
     -H 'Content-Type: application/json' \
     -d '{"sql": "SELECT full_name FROM patients WHERE national_id LIKE $1", "params": ["1%"]}'
```

```text
{"endpoint":"operational","open_transaction_rolled_back":false,"path":"OPERATIONAL","results":[{"elapsed_ms":0.68,"error":{"code":"DEP_PROTECTED_OPERATION","hint":"PROTECTED values support only full-value equality matching. To release a value, use a disclosing function.","message":"LIKE is not supported for PROTECTED values (patients.national_id).","sqlstate":"ZD101"},"notes":[],"statement":"SELECT full_name FROM patients WHERE national_id LIKE $1"}],"role":"reception_app"}
HTTP 422
```

A transaction in one request, as plain text, on the masked read path:

```sh
curl -s -u bi_dashboard:demo "http://127.0.0.1:8433/v1/query" -H 'DEP-Endpoint: analytics' \
     --data 'BEGIN; SELECT health_fund, count(*) AS patients FROM patients GROUP BY health_fund ORDER BY health_fund; COMMIT;' | jq -c '.path, (.results[] | {command, rows})'
```

```text
"MASKED_READ"
{"command":"BEGIN","rows":null}
{"command":null,"rows":[["Clalit",2],["Leumit",1],["Maccabi",2],["Meuhedet",1]]}
{"command":"COMMIT","rows":null}
```

Wrong password, then a role that may not use the endpoint:

```sh
curl -s -w ' HTTP %{http_code}\n' -u reception_app:wrong "http://127.0.0.1:8433/v1/query" --data 'SELECT 1'
curl -s -w ' HTTP %{http_code}\n' -u reception_app:demo "http://127.0.0.1:8433/v1/query?endpoint=analytics" --data 'SELECT 1'
```

```text
{"error":{"code":null,"hint":null,"message":"password authentication failed for role \"reception_app\"","sqlstate":"28P01"}} HTTP 401
{"error":{"code":null,"hint":"A superuser can allow it: GRANT CONNECT ON ENDPOINT analytics TO reception_app.","message":"role \"reception_app\" is not allowed to connect through endpoint \"analytics\"","sqlstate":"28000"}} HTTP 403
```

## GET /v1/health

```sh
curl -s http://127.0.0.1:8433/v1/health
```

```text
{"persistent":false,"product":"DEP Engine","status":"ok","tables":3,"version":"0.5.2"}
```

`persistent` is `true` when the server keeps its data in a data directory. The health check is meant for a monitor on a trusted network.

## GET /metrics

The counters are listed in the [administration guide](../guides/administration.md#metrics). They contain no data.

## Over HTTPS

With TLS on, use `https://`. A self-signed certificate needs `curl --cacert DATA_DIR/tls/server.crt`, or `-k` for a local test only.
