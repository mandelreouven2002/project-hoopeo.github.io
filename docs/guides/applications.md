# Connecting from applications

DEP Engine speaks the PostgreSQL wire protocol (the simple and the extended protocol), so an application connects with an ordinary PostgreSQL driver. It also has an HTTP API for clients that cannot speak the protocol. This guide shows the connection settings, how parameters, DEP notes and DEP errors appear to a driver, and complete examples in Python and JavaScript. The examples ran against DEP Engine 0.5.2 with the demo clinic, with `psycopg` 3.3 and `node-postgres` 8.23, and `psql` 16 was used for the checks in the [getting started guide](getting-started.md). Other PostgreSQL drivers should work the same way, but only these were tried.

## Connection settings

| Setting | Value |
|---|---|
| Host, port | The server's `--listen` address (default `127.0.0.1:5433`) |
| User | The **role**, for example `reception_app` |
| Database | The **endpoint**, for example `operational` or `analytics`. It fixes the access path. |
| Password | The role's password |
| SSL mode | `require` or `verify-full` for any server that is not on the same machine; see the [administration guide](administration.md#tls) |

The role is a database role, not a person, in most deployments: one role per application or job, each granted the endpoint, columns and functions it needs and nothing more. Connect an analytics tool through `analytics` and it cannot match, write, or disclose, whatever its role is granted.

A connection keeps its role and path for its whole life. The path is fixed when it logs in, and cannot be changed by `SET`. If an application needs two access paths, it opens two connections, ideally with two roles.

## Parameters

Use parameters, as with any database. DEP treats a protected parameter (`$1` compared with a protected column, or inserted into one) exactly as it treats a protected literal. In the extended protocol the engine infers the types of parameters for queries and for `INSERT`, `UPDATE` and `DELETE`. An error message never carries the value of a bound parameter.

## Notes and errors

- **Notes.** Each rule that applies produces a DEP note, sent as a PostgreSQL `NOTICE` whose text starts with `DEP: `. Drivers deliver notices through a handler (`add_notice_handler` in psycopg, the `notice` event in node-postgres). They need no handling: ignore them in production and read them while developing.
- **Errors.** A DEP error has a `ZDnnn` SQLSTATE and a message and a hint. Branch on the SQLSTATE. The `DEP_...` name is not sent over the PostgreSQL protocol; the [error reference](../reference/errors.md) maps one to the other. Constraint violations use PostgreSQL's SQLSTATEs (`23505`, `23502`, `23503`), and `40001` means the transaction should be retried.
- **Protected columns come back as NULL.** `SELECT national_id` returns a row with NULL, never an error. An application that needs the value must call a declared function, and the role must have `EXECUTE` on it.

## Python (psycopg)

```python
import psycopg

DSN = "host=127.0.0.1 port=5433 user=reception_app password=demo dbname=operational sslmode=disable"

with psycopg.connect(DSN) as conn:
    conn.add_notice_handler(lambda d: print("  note:", d.message_primary))

    # A lookup by a protected value, with a parameter
    cur = conn.execute("SELECT full_name, national_id FROM patients WHERE national_id = %s", ("123456782",))
    print(cur.fetchall())

    # A refused operation: branch on the SQLSTATE
    try:
        conn.execute("SELECT full_name FROM patients WHERE national_id LIKE %s", ("1%",))
    except psycopg.errors.Error as e:
        print(e.sqlstate, e.diag.message_primary)
        print("hint:", e.diag.message_hint)
        conn.rollback()                      # the transaction is aborted until then

    # A write, then the same lookup
    conn.execute("UPDATE patients SET full_name = %s WHERE national_id = %s", ("Dana Cohen-Levi", "123456782"))
    conn.commit()
    print(conn.execute("SELECT full_name FROM patients WHERE national_id = %s", ("123456782",)).fetchone())
```

```text
  note: DEP: PROTECTED column patients.national_id is matched by equality through the blind index of MATCH DOMAIN person_id.
  note: DEP: Output column national_id is PROTECTED column patients.national_id (MATCH DOMAIN person_id) and is returned as NULL.
[('Dana Cohen', None)]
ZD101 LIKE is not supported for PROTECTED values (patients.national_id).
hint: PROTECTED values support only full-value equality matching. To release a value, use a disclosing function.
  note: DEP: PROTECTED column patients.national_id is matched by equality through the blind index of MATCH DOMAIN person_id.
  note: DEP: PROTECTED column patients.national_id is matched by equality through the blind index of MATCH DOMAIN person_id.
('Dana Cohen-Levi',)
```

psycopg sends `%s` parameters as `$1` and so on over the extended protocol. After an error inside a transaction block, the transaction is aborted (`25P02`) until `rollback()`.

## JavaScript (node-postgres)

```javascript
const { Client } = require('pg');

(async () => {
  const client = new Client({
    host: '127.0.0.1', port: 5433, user: 'reception_app', password: 'demo', database: 'operational',
  });
  client.on('notice', (n) => console.log('  note:', n.message));
  await client.connect();

  const res = await client.query(
    'SELECT full_name, national_id FROM patients WHERE national_id = $1', ['123456782']);
  console.log(res.rows);

  try {
    await client.query('SELECT full_name FROM patients WHERE national_id LIKE $1', ['1%']);
  } catch (e) {
    console.log(e.code, e.message, '| hint:', e.hint);
  }
  await client.end();
})();
```

```text
  note: DEP: PROTECTED column patients.national_id is matched by equality through the blind index of MATCH DOMAIN person_id.
  note: DEP: Output column national_id is PROTECTED column patients.national_id (MATCH DOMAIN person_id) and is returned as NULL.
[ { full_name: 'Dana Cohen', national_id: null } ]
ZD101 LIKE is not supported for PROTECTED values (patients.national_id). | hint: PROTECTED values support only full-value equality matching. To release a value, use a disclosing function.
```

Calling a declared function is a plain `SELECT * FROM fn($1)`. Billing's job gets the monthly report like this:

```python
import psycopg

DSN = "host=127.0.0.1 port=5433 user=billing_job password=demo dbname=operational sslmode=disable"
with psycopg.connect(DSN) as conn:
    conn.add_notice_handler(lambda d: print("  note:", d.message_primary))
    for row in conn.execute("SELECT national_id, visit_type FROM monthly_fund_report(%s) ORDER BY national_id LIMIT 3", ("2026-10-01",)):
        print(row)
```

```text
  note: DEP: monthly_fund_report() is declared DISCLOSES person_id: the columns it declares are returned in plaintext.
  note: DEP: Output column national_id is disclosed: Column national_id of monthly_fund_report() (DISCLOSES person_id).
('123456782', 'Lab results')
('123456782', 'Follow-up')
('234567893', 'Lab results')
```

## Pools, roles and sessions

- **One pool per role and endpoint.** A connection's identity is its role and path, so do not share a pool between applications. The audit log attributes every protected match to the role that made it.
- **Do not rely on `SET ROLE`.** Only a superuser may use it, and it changes the privileges that apply without changing the path. Use separate connections.
- **Keep transactions short.** One transaction writes at a time, and a transaction left idle for `--idle-in-transaction-timeout` is rolled back. See [Transactions](transactions-and-constraints.md#concurrent-transactions).
- **Retry on `40001`.** It means another transaction changed a table this one wanted to write.
- **Mind the match limit.** An application that looks up many protected values in a short time can reach its role's limit (`ZD109`). The limit counts every value of an `IN` list.

## The HTTP API

When a client cannot use the PostgreSQL protocol, `POST /v1/query` runs one or more statements. It authenticates with HTTP Basic authentication as a role, and the endpoint comes from `?endpoint=` or the `DEP-Endpoint` header (default `operational`).

```sh
curl -s -u reception_app:demo "http://127.0.0.1:8433/v1/query?endpoint=operational" \
     -H 'Content-Type: application/json' \
     -d '{"sql": "SELECT full_name, national_id FROM patients WHERE national_id = $1", "params": ["123456782"]}' | jq .
```

```text
{
  "endpoint": "operational",
  "open_transaction_rolled_back": false,
  "path": "OPERATIONAL",
  "results": [
    {
      "columns": [
        "full_name",
        "national_id"
      ],
      "elapsed_ms": 2.83,
      "notes": [
        "PROTECTED column patients.national_id is matched by equality through the blind index of MATCH DOMAIN person_id.",
        "Output column national_id is PROTECTED column patients.national_id (MATCH DOMAIN person_id) and is returned as NULL."
      ],
      "row_count": 1,
      "rows": [
        [
          "Dana Cohen",
          null
        ]
      ],
      "statement": "SELECT full_name, national_id FROM patients WHERE national_id = $1"
    }
  ],
  "role": "reception_app"
}
```

The statements of one request run in order on one connection, so `BEGIN; ...; COMMIT;` works in a single request. They stop at the first error, and a transaction left open is rolled back. See the [HTTP API reference](../reference/http-api.md).
