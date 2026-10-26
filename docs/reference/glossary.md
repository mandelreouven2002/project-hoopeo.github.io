# Glossary

Every term of DEP, with what it means, why it exists, an example from the demo clinic, and where you will meet it. Each term has its own address (`glossary/#protected-column`), so any page can link to it. For a first reading, [DEP SQL in five minutes](../guides/overview.md) introduces the main terms in order.

**The terms:** [Access path](#access-path) · [Anchor](#anchor) · [Audit log](#audit-log) · [Blind index](#blind-index) · [Ciphertext](#ciphertext) · [Declared function](#declared-function) · [DEP error](#dep-error) · [DEP note](#dep-note) · [Disclosure](#disclosure) · [Endpoint](#endpoint) · [Linking](#linking) · [Masked read path](#masked-read-path) · [Master key](#master-key) · [Match domain](#match-domain) · [Match limit](#match-limit) · [Matching](#matching) · [Operational path](#operational-path) · [Privacy matrix](#privacy-matrix) · [Protected column](#protected-column) · [REVEAL](#reveal) · [Terminal rule](#terminal-rule)

## The data

### Protected column

**A column whose values the database uses, but never returns.**

A column is protected when the schema says so: `PROTECTED MATCH DOMAIN` after its type. Its type does not change; `national_id` is still `TEXT`. The database can compare it with a whole value for equality, so the column can still find a row, carry a primary or unique key, and connect two tables. What it cannot do is come back: in any query, on any path, for any role, including the administrator, a protected column in the output is returned as NULL. Anything that would reveal part of a value, or its order, is refused: `LIKE`, substrings, casts, sorting, ranges, `min` and `max`, or passing the value to a function. `IS NULL` is allowed.

```sql
national_id TEXT PROTECTED MATCH DOMAIN person_id UNIQUE,
patient_national_id TEXT PROTECTED REFERENCES patients (national_id),   -- takes the domain of the column it references
```

- **Why:** the application that finds a patient by ID number never needs to read ID numbers, and now it cannot.
- **You will meet it:** in the DEP note "Output column national_id is PROTECTED ... and is returned as NULL", and in error ZD101 when an operation is refused.
- **More:** [Concepts: Protected columns](../guides/concepts.md#protected-columns), [DEP SQL: Protected columns](../guides/dep-sql.md#protected-columns).

### Match domain

**A family of values that may be compared with each other, with its own keys.**

Every protected column belongs to one match domain, named in its declaration. Values can be compared only within a domain: every column that holds ID numbers is in `person_id`, every column of phone numbers in `phone_number`, and an ID number can never be compared with a phone number, even by mistake. Each domain has its own keys, so the stored form of a value in one domain says nothing about the same characters in another. Permissions to disclose and to link are also given per domain.

```sql
GRANT DISCLOSE ON MATCH DOMAIN phone_number TO privacy_admin;   -- may write functions that return phone numbers
```

- **Why:** a domain is the unit of meaning ("a person's ID number") that the privacy rules talk about.
- **You will meet it:** in every DEP note about a protected column, and in error ZD103 when two domains are mixed.
- **More:** [Concepts: Match domains](../guides/concepts.md#match-domains).

### Blind index

**What the database stores instead of the value: a keyed fingerprint, the same for equal values.**

When a protected value is written, DEP Engine computes its index entry: a keyed hash (HMAC-SHA256) of the value with the key of its match domain, stored as `ie1:` and 32 hexadecimal digits. Equal values give equal entries, so the engine can find, join and check uniqueness by comparing entries, without the plain value. Without the key, an entry cannot be turned back into the value, and it says nothing about order or prefixes, which is why `LIKE` and ranges cannot work. One thing it does show, to someone who can read the stored data: which rows share a value.

```text
'123456782'  ->  ie1:fed1451ad4eb22a68f74e52c2945fbd5
```

- **Why:** equality is all that finding and connecting records needs.
- **You will meet it:** in the DEP note "matched by equality through the blind index of MATCH DOMAIN person_id", in `EXPLAIN` (`Read: index patients_national_id_key`), and in `SHOW STORAGE`.
- **More:** [Concepts: Blind indexes and ciphertexts](../guides/concepts.md#blind-indexes-and-ciphertexts).

### Ciphertext

**The encrypted copy of a protected value, stored next to its index entry.**

A protected value is also encrypted (XChaCha20-Poly1305, with the key of its domain) and stored as `ct1:...`. The database never decrypts it to answer a query; only [REVEAL](#reveal), inside a [disclosing function](#disclosure), does. It exists so that a value can be disclosed when a declared purpose requires it, and so that keys can be rotated.

- **You will meet it:** in `SHOW STORAGE patients`, which shows a table as it is kept: index entries and ciphertexts.

### Master key

**The one secret from which every other key is derived.**

Each database has a 256-bit master key, in `master.key` in the data directory, or elsewhere with `--key-file`. The keys of each match domain (one for index entries, one for encryption) are derived from it. Kept on another volume, it makes a copy of the data directory worthless on its own: only index entries and ciphertexts. `ALTER SYSTEM ROTATE MASTER KEY` replaces it, and re-encrypts every value and recomputes every index entry.

- **More:** [Administration: The data directory and the master key](../guides/administration.md#the-data-directory-and-the-master-key), [Key rotation](../guides/administration.md#key-rotation).

## The connections

### Endpoint

**The door a connection comes in by. It fixes the connection's access path.**

A client connects to an endpoint by name, where PostgreSQL clients name a database: `-d operational`. Each endpoint has an [access path](#access-path), and a role may use an endpoint only if it was granted to it. The demo clinic has two: `operational` and `analytics`. More can be made with `CREATE ENDPOINT`.

```sql
GRANT CONNECT ON ENDPOINT analytics TO bi_dashboard;
```

- **Why:** what a connection may do depends on what it is for, not only on who it is. A dashboard and a reception desk need different powers even with the same data.
- **More:** [Concepts: Endpoints and access paths](../guides/concepts.md#endpoints-and-access-paths).

### Access path

**What a connection may do with protected values: `OPERATIONAL` or `MASKED_READ`.**

The path is set by the endpoint at login and stays for the whole session: `SET ROLE` keeps it, and `SET dep.path` is refused (error ZD106). To work on another path, connect through another endpoint. It is the second question the engine asks about every statement, after the role's privileges.

### Operational path

**For applications that serve one person at a time.**

On `OPERATIONAL` a connection may [match](#matching) protected values, write, change the schema, and call declared functions. Protected outputs are still NULL. This is the path of reception, the doctors' application and the jobs that act on one person or send a report.

### Masked read path

**For dashboards, reports and analysis: read what is ordinary, match nothing.**

On `MASKED_READ` a connection reads ordinary columns, and protected columns come back as NULL. It may not match protected values, write, change the schema, or call a disclosing function (error ZD102). It may call a function declared `LINKS`, which is how a dashboard gets statistics that need protected values.

```text
bi_dashboard@analytics=> SELECT patient_id FROM patients WHERE national_id = '123456782';
ERROR:  Matching PROTECTED values is unavailable on a MASKED_READ connection.
```

## The queries

### Matching

**Finding rows by a protected value, by equality only.**

A match compares a protected column with a whole value (`national_id = '123456782'`, or a parameter `$1`), with a list (`IN`, `NOT IN`, `OR`), or with another protected column of the same domain (a join). The engine computes the index entry of the given value and compares entries. Matching needs the `OPERATIONAL` path and a column privilege: in DEP, `SELECT` on a protected column is the privilege to match on it. Every match is counted for the [match limit](#match-limit) and written to the [audit log](#audit-log).

- **You will meet it:** in the DEP note "is matched by equality through the blind index".
- **More:** [DEP SQL: Matching](../guides/dep-sql.md#matching).

### Anchor

**The one value that a join on protected columns is tied to.**

A join between protected columns is allowed when the query also pins one side to a single value: then it connects the records of one person. Without that value, the same join would connect every person in one table to every person in the other, which is linking, and it is refused before a row is read (error ZD107).

```sql
-- anchored: the records of one patient
SELECT m.diagnosis FROM patients AS p
JOIN medical_records AS m ON m.patient_national_id = p.national_id
WHERE p.national_id = '123456782';
```

- **You will meet it:** in the DEP note "is anchored to a single protected value", and in error ZD107 with a hint that shows how to anchor.
- **More:** [DEP SQL: Joins and anchors](../guides/dep-sql.md#joins-and-anchors).

### Linking

**Connecting protected values across whole tables, without an anchor. Only in a `LINKS` function.**

Statistics, deduplication and reports often need every record joined on a protected value. That is the most powerful thing a database can do with personal data, so DEP allows it only inside a [declared function](#declared-function) with `LINKS` for the domain. `GROUP BY` and `DISTINCT` on a protected column are also linking.

```sql
CREATE FUNCTION diagnoses_per_fund() RETURNS TABLE (health_fund TEXT, diagnosis TEXT, patients BIGINT)
LANGUAGE sql LINKS person_id AS $$
    SELECT p.health_fund, m.diagnosis, COUNT(*)
    FROM patients AS p JOIN medical_records AS m ON m.patient_national_id = p.national_id
    GROUP BY p.health_fund, m.diagnosis
$$;
```

## The exceptions

### Declared function

**A named function that may do what ordinary queries may not: disclose, or link.**

A declared function is written in SQL (`LANGUAGE sql`) and declares its purpose and domain: `DISCLOSES person_id`, `LINKS person_id`, or both. Only a role granted `DISCLOSE` or `LINK` on that match domain may create one; other roles get `EXECUTE` on it, never `PUBLIC`. It runs with its owner's privileges, so the caller needs no access to the tables. Every call is in the audit log. `COMMENT ON FUNCTION` records its purpose and owner, and the [privacy matrix](#privacy-matrix) lists it.

- **Why:** every exception has a name, an owner, a reviewer and a record, instead of being a privilege hidden in an application.
- **You will meet it:** marked D and L in Hoopeo Studio's object browser, and in `SHOW FUNCTIONS`.
- **More:** [Concepts: Declared functions](../guides/concepts.md#declared-functions), [DEP SQL: Declared functions](../guides/dep-sql.md#declared-functions).

### Disclosure

**Returning a protected value in plain text. Only in a `DISCLOSES` function, with `REVEAL`.**

When a value must leave the database for a declared purpose (the ID numbers on a report to the health funds, the phone numbers for appointment reminders), a disclosing function returns it. The columns it discloses are marked in its result, and each call is audited with the number of values disclosed.

```text
billing_job@operational=> SELECT * FROM monthly_fund_report(DATE '2026-10-01');
DEP:  Output column national_id is disclosed: Column national_id of monthly_fund_report() (DISCLOSES person_id).
```

### REVEAL

**The one operation that decrypts a protected value.**

`REVEAL(p.national_id)` turns a protected value into plain text. It is accepted only inside a function declared `DISCLOSES` for that column's domain (error ZD001 anywhere else), and its result is a [disclosed value](#terminal-rule).

### Terminal rule

**A disclosed value can be returned, and used for nothing else.**

Once a value has been disclosed, it is plain text, and the database could no longer protect it in a further match. So it may only leave: it cannot be filtered on, joined, grouped, or matched again (error ZD104). A query that needs to match should do so on the protected column inside the function, before `REVEAL`.

```text
ERROR:  Column national_id of monthly_fund_report() is a disclosed value and cannot be used in a join condition.
SQLSTATE:  ZD104 (DEP_DISCLOSED_VALUE_USE)
```

## What the database tells you

### DEP note

**A line with a result that says how protected columns were handled.**

Every statement that touches a protected column comes back with DEP notes: which column was matched and how, which output was returned as NULL, which join was anchored, which value was disclosed. They are notices on the PostgreSQL protocol (`NOTICE:  DEP: ...` in psql), a `notes` field in the HTTP API, `DEP:` lines in DEP CLI, and the Output pane of Hoopeo Studio. They explain; they never change the result.

- **More:** [The notes reference](notes.md).

### DEP error

**A refusal, with a `ZD` code and a hint that says what would be allowed.**

A statement that breaks a rule is refused before it reads any data. The error has a message, a hint, a SQLSTATE of class `ZD` (ZD001 to ZD109) and a name such as `DEP_UNANCHORED_MATCH`. An error never contains a protected value or the value of a parameter.

- **More:** [The error reference](errors.md).

### Audit log

**The record of every protected match, disclosure, linking and refusal, without values.**

DEP Engine writes an audit record for each protected match (with the number of values), each call of a declared function, each refused statement, each match limit reached and each key rotation, with the role, the endpoint, the path, the object and the domain. It never records a protected value or the text of a statement. `SHOW AUDIT` shows the latest records; Hoopeo Studio has Tools > Audit Log.

- **More:** [Administration: The audit log](../guides/administration.md#the-audit-log).

### Match limit

**How many protected matches a role may make per minute.**

A reception desk looks up a few patients an hour; a script that tries a million ID numbers is an attack. A match limit stops it: `ALTER ROLE reception_app MATCH LIMIT 100 PER MINUTE`, or `--match-limit` for every role that is not a superuser. A role over its limit gets error ZD109, and the audit log records it.

- **More:** [DEP SQL: Match limits](../guides/dep-sql.md#match-limits).

### Privacy matrix

**Who may match, and who may disclose, each protected column, through which path.**

`SHOW PRIVACY MATRIX` (Tools > Privacy Matrix in Hoopeo Studio) lists every protected column against every role: the paths it can use, whether it may match, and the declared functions through which it may disclose. It is computed from the schema and the grants, so it is always the rules as the engine applies them, and it is the table a privacy officer reviews.

- **More:** [The SHOW reference](show.md).
