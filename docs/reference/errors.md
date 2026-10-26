# Error reference

A DEP error is an ordinary PostgreSQL error with a SQLSTATE in class `ZD`, a message, and a hint. DEP CLI and Hoopeo Studio print the DEP code beside the SQLSTATE, as in `SQLSTATE: ZD107 (DEP_UNANCHORED_MATCH)`. Over the PostgreSQL protocol only the SQLSTATE travels, so a program branches on `ZD107`; the HTTP API also returns the code. Errors that DataFusion or PostgreSQL would also raise keep their usual SQLSTATEs, listed in the second table.

![A DEP error in Hoopeo Studio: the message, the code and the hint](../images/studio/studio-refused.png)

A **policy rejection** depends only on the statement, the access path, the role and the schema, and happens before any row is read. An **execution failure** happens while an allowed statement runs and may depend on the data (a constraint, for example). An error message never contains a protected value or the value of a bound parameter.

## DEP errors

| SQLSTATE | Code | Raised when |
|---|---|---|
| `ZD001` | `DEP_REVEAL_OUTSIDE_DISCLOSURE` | [`REVEAL`](glossary.md#reveal) is used outside a function that declares `DISCLOSES` for the column's [match domain](glossary.md#match-domain) |
| `ZD002` | `DEP_DECLARATION_PRIVILEGE` | A role declares a [`DISCLOSES` or `LINKS` function](glossary.md#declared-function) for a domain without `GRANT DISCLOSE` or `GRANT LINK`, or `DISCLOSE` or `LINK` is granted to `PUBLIC` |
| `ZD003` | `DEP_DECLARED_LANGUAGE` | A [declared function](glossary.md#declared-function) is not `LANGUAGE sql` |
| `ZD004` | `DEP_DECLARED_CALL` | A declared function calls another declared function that is not declared for the same purpose and domain |
| `ZD005` | `DEP_PUBLIC_EXECUTE` | `EXECUTE` on a function declared `DISCLOSES` or `LINKS` is granted to `PUBLIC` |
| `ZD006` | `DEP_REFERENCE_MISMATCH` | A reference joins a [protected column](glossary.md#protected-column) and an ordinary one |
| `ZD101` | `DEP_PROTECTED_OPERATION` | An operation other than full-value equality is applied to a [protected value](glossary.md#protected-column) |
| `ZD102` | `DEP_ENDPOINT_OPERATION` | The operation is not allowed on the connection's [access path](glossary.md#access-path): matching, writing, schema changes, or disclosing on the [`MASKED_READ`](glossary.md#masked-read-path) path |
| `ZD103` | `DEP_MATCH_DOMAIN_MISMATCH` | Two protected columns of different [match domains](glossary.md#match-domain) are compared or referenced |
| `ZD104` | `DEP_DISCLOSED_VALUE_USE` | A disclosed value is used for anything but being returned: the [terminal rule](glossary.md#terminal-rule) |
| `ZD105` | `DEP_UNQUALIFIED_ACCESS` | Reserved. Defined by the design, and not raised by DEP Engine 0.5 |
| `ZD106` | `DEP_PATH_IMMUTABLE` | A connection tries to change its [access path](glossary.md#access-path) |
| `ZD107` | `DEP_UNANCHORED_MATCH` | A match of protected values is not [anchored](glossary.md#anchor) to a single value, outside a function declared for [linking](glossary.md#linking) (`LINKS`) |
| `ZD108` | `DEP_UNSUPPORTED_FORM` | A form DEP cannot analyze, rejected rather than run with uncertain protection |
| `ZD109` | `DEP_MATCH_RATE_LIMITED` | A role went over its [match limit](glossary.md#match-limit) |
| `ZD901` | | An internal code with no SQLSTATE of its own. DEP Engine does not raise it |

### Examples

Each was run against the demo clinic. The role is in the prompt.

```text
admin@operational=> SET ROLE privacy_admin;
SET

privacy_admin@operational=> SELECT full_name, REVEAL(national_id) FROM patients;
ERROR:  REVEAL() of patients.national_id (match domain person_id) appears outside a disclosing function.
HINT:  Move the disclosure into a function declared DISCLOSES person_id.
SQLSTATE:  ZD001 (DEP_REVEAL_OUTSIDE_DISCLOSURE)

privacy_admin@operational=> CREATE FUNCTION no_sql(id TEXT) RETURNS TABLE (x TEXT) LANGUAGE plpgsql DISCLOSES person_id AS $$ SELECT 1 $$;
ERROR:  Function no_sql is written in LANGUAGE plpgsql.
HINT:  Declared functions must be LANGUAGE sql, so the engine can analyze their bodies.
SQLSTATE:  ZD003 (DEP_DECLARED_LANGUAGE)

privacy_admin@operational=> CREATE FUNCTION wrap(d DATE) RETURNS TABLE (national_id TEXT) LANGUAGE sql LINKS person_id AS $$ SELECT r.national_id FROM monthly_fund_report(d) AS r $$;
ERROR:  Function wrap calls monthly_fund_report(), which is not declared for the same purpose and match domain.
HINT:  Inline the logic, or declare both functions for the same purpose.
SQLSTATE:  ZD004 (DEP_DECLARED_CALL)

privacy_admin@operational=> GRANT EXECUTE ON FUNCTION monthly_fund_report(DATE) TO PUBLIC;
ERROR:  Function monthly_fund_report is declared DISCLOSES or LINKS, and cannot be executable by PUBLIC.
HINT:  Grant EXECUTE to the specific roles that need it.
SQLSTATE:  ZD005 (DEP_PUBLIC_EXECUTE)

privacy_admin@operational=> RESET ROLE;
SET

admin@operational=> GRANT DISCLOSE ON MATCH DOMAIN person_id TO PUBLIC;
ERROR:  DISCLOSE and LINK cannot be granted to PUBLIC.
HINT:  Grant them to the roles that declare functions.
SQLSTATE:  ZD002 (DEP_DECLARATION_PRIVILEGE)

admin@operational=> \c operational app_developer
You are now connected as "app_developer" through endpoint "operational" (OPERATIONAL).

app_developer@operational=> CREATE FUNCTION all_ids() RETURNS TABLE (national_id TEXT) LANGUAGE sql DISCLOSES person_id AS $$ SELECT REVEAL(national_id) FROM patients $$;
ERROR:  Role app_developer may not declare functions that DISCLOSES match domain person_id.
HINT:  A superuser can allow it: GRANT DISCLOSE ON MATCH DOMAIN person_id TO app_developer.
SQLSTATE:  ZD002 (DEP_DECLARATION_PRIVILEGE)

app_developer@operational=> \c operational admin
You are now connected as "admin" through endpoint "operational" (OPERATIONAL).

admin@operational=> CREATE TABLE bad (id BIGINT PRIMARY KEY, nid TEXT REFERENCES patients (national_id));
ERROR:  Column bad.nid references PROTECTED column patients.national_id (match domain person_id) but is not PROTECTED.
HINT:  Declare it TEXT PROTECTED REFERENCES patients (national_id), so the reference is kept as an index entry.
SQLSTATE:  ZD006 (DEP_REFERENCE_MISMATCH)

admin@operational=> SELECT full_name FROM patients WHERE national_id LIKE '1%';
ERROR:  LIKE is not supported for PROTECTED values (patients.national_id).
HINT:  PROTECTED values support only full-value equality matching. To release a value, use a disclosing function.
SQLSTATE:  ZD101 (DEP_PROTECTED_OPERATION)
```

```text
bi_dashboard@analytics=> SELECT health_fund FROM patients WHERE national_id = '123456782';
ERROR:  Matching PROTECTED values is unavailable on a MASKED_READ connection.
HINT:  This operation requires an OPERATIONAL connection.
SQLSTATE:  ZD102 (DEP_ENDPOINT_OPERATION)

bi_dashboard@analytics=> SET dep.path = 'OPERATIONAL';
ERROR:  The access path of this connection is MASKED_READ and cannot be changed.
HINT:  The path is fixed at authentication. Connect through another endpoint instead.
SQLSTATE:  ZD106 (DEP_PATH_IMMUTABLE)
```

```text
admin@operational=> CREATE TABLE a (id BIGINT PRIMARY KEY, x TEXT PROTECTED MATCH DOMAIN d1);
CREATE TABLE

admin@operational=> CREATE TABLE b (id BIGINT PRIMARY KEY, y TEXT PROTECTED MATCH DOMAIN d2);
CREATE TABLE

admin@operational=> SELECT 1 FROM a, b WHERE a.x = b.y;
ERROR:  a.x (match domain d1) cannot be compared with b.y (match domain d2).
HINT:  Values from different match domains are different kinds of value.
SQLSTATE:  ZD103 (DEP_MATCH_DOMAIN_MISMATCH)
```

```text
doctors_app@operational=> SELECT p.full_name FROM patients AS p JOIN medical_records AS m ON m.patient_national_id = p.national_id;
ERROR:  Join on PROTECTED columns patients.national_id and medical_records.patient_national_id is not anchored to a single protected value.
HINT:  Anchor the join with a lookup such as WHERE patients.national_id = $1, or perform it inside a function declared LINKS person_id.
SQLSTATE:  ZD107 (DEP_UNANCHORED_MATCH)
```

```text
billing_job@operational=> SELECT * FROM monthly_fund_report(DATE '2026-10-01') AS r WHERE r.national_id = '123456782';
ERROR:  Column national_id of monthly_fund_report() is a disclosed value and cannot be used in a filter.
HINT:  Disclosed values can only be returned. Perform the match on PROTECTED fields inside a declared function, before REVEAL.
SQLSTATE:  ZD104 (DEP_DISCLOSED_VALUE_USE)
```

```text
admin@operational=> SELECT full_name FROM patients ORDER BY national_id;
ERROR:  ORDER BY is not supported for PROTECTED values (patients.national_id).
HINT:  PROTECTED values support only full-value equality matching. To release a value, use a disclosing function.
SQLSTATE:  ZD101 (DEP_PROTECTED_OPERATION)

admin@operational=> MERGE INTO patients USING patients p2 ON patients.patient_id = p2.patient_id WHEN MATCHED THEN DELETE;
ERROR:  Statement not supported: MERGE INTO patients
HINT:  DEP rejects forms it cannot analyze rather than run them with uncertain protection.
SQLSTATE:  ZD108 (DEP_UNSUPPORTED_FORM)

admin@operational=> ALTER ROLE reception_app MATCH LIMIT 1 PER MINUTE;
ALTER ROLE

admin@operational=> \c operational reception_app
You are now connected as "reception_app" through endpoint "operational" (OPERATIONAL).

reception_app@operational=> SELECT full_name FROM patients WHERE national_id = '123456782';
DEP:  PROTECTED column patients.national_id is matched by equality through the blind index of MATCH DOMAIN person_id.
 full_name
------------
 Dana Cohen
(1 row)

reception_app@operational=> SELECT full_name FROM patients WHERE national_id = '234567893';
ERROR:  Role reception_app has made 1 protected matches of the 1 PER MINUTE it is allowed; this statement would make 1 more.
HINT:  Wait for the period to pass, or ask a superuser to change the limit (ALTER ROLE ... MATCH LIMIT n PER MINUTE).
SQLSTATE:  ZD109 (DEP_MATCH_RATE_LIMITED)
```

## PostgreSQL SQLSTATEs

These are the SQLSTATEs DEP Engine itself raises, besides `ZD`. DataFusion may raise others for ordinary SQL, such as a type error.

| SQLSTATE | Meaning | In DEP Engine |
|---|---|---|
| `08000`, `08001`, `08006` | Connection exception | A TLS setup or handshake failure; an unknown endpoint at login; a client that could not connect |
| `0LP01` | Invalid grant operation | A privilege that does not apply to the object (for example `EXECUTE` on a table) |
| `22023` | Invalid parameter value | An unknown endpoint mode, role, match limit or period |
| `22P02` | Invalid text representation | A malformed JSON body in the HTTP API |
| `23502` | Not-null violation | NULL in a `NOT NULL` column |
| `23503` | Foreign-key violation | A reference to nothing, or a referenced row changed or deleted |
| `23505` | Unique violation | A primary or unique key already exists. For a protected column the message never shows the value |
| `25000`, `25001` | Invalid transaction state | The transaction has ended; `ALTER SYSTEM ROTATE MASTER KEY` inside a transaction block |
| `25P01` | No active transaction | `SAVEPOINT`, `ROLLBACK TO` or `RELEASE` outside a transaction block |
| `25P02` | In failed transaction | A statement after an error, until `ROLLBACK` or `ROLLBACK TO` |
| `25P03` | Idle-in-transaction timeout | The transaction was idle longer than `--idle-in-transaction-timeout` and was rolled back |
| `28000` | Invalid authorization | A role that does not exist, cannot log in, or may not use the endpoint; a plain connection to a server that requires TLS |
| `28P01` | Invalid password | Wrong password (also `401` over HTTP) |
| `2BP01` | Dependent objects exist | Dropping a table that others reference, or an index the engine keeps |
| `3B001` | Invalid savepoint | `ROLLBACK TO` or `RELEASE` of a savepoint that does not exist |
| `40001` | Serialization failure | A transaction wrote a table that another transaction changed after it began |
| `42501` | Insufficient privilege | A missing privilege, for a table, column, function, or the right to create objects. For a protected column the hint names the grant that would allow matching |
| `42601` | Syntax error | A malformed statement, and a `PROTECTED` column with no match domain |
| `42703`, `42P01` | Undefined column, undefined table | A name that does not exist |
| `42704` | Undefined object | A role, index, endpoint, match domain or type that does not exist, or an unknown `dep.` setting |
| `42710`, `42723`, `42P07` | Duplicate object | A role, endpoint, function, table or index that already exists |
| `42804` | Datatype mismatch | `REVEAL` of a column that is not protected; a protected column that is not `TEXT` |
| `42883` | Undefined function | A function that does not exist, or `REVEAL` with the wrong number of arguments |
| `42P02` | Undefined parameter | A `$n` with no value |
| `42P13` | Invalid function definition | A missing body, or a body whose columns do not match `RETURNS TABLE` |
| `42P16` | Invalid table definition | A permanent table that references a temporary one |
| `53200` | Out of memory | A query needs more than `--memory-limit` and cannot spill (a hash join) |
| `53300` | Too many connections | More than `--max-connections` |
| `55000` | Object not in prerequisite state | A function that uses a table dropped and created again |
| `55006` | Object in use | The data directory is used by another DEP Engine; a role that cannot be dropped; the `admin` role must stay a superuser |
| `55P03` | Lock not available | The write lock was not free within `--lock-timeout` |
| `58030` | I/O error | A data file, key file or commit failure; a missing or invalid key file |
| `XX000` | Internal error | An error from DataFusion that DEP does not classify, or an internal inconsistency |
| `XX001` | Data corrupted | A stored row cannot be decoded, or a protected value cannot be decrypted with the current key |
