# Transactions, constraints and indexes

This guide covers the parts of DEP Engine that behave like an ordinary SQL database: transactions and savepoints, transactional schema changes, constraints, indexes and `EXPLAIN`. DEP's privacy rules apply inside all of them. Examples run against the demo clinic, and the output is real.

## Transactions

Every statement runs in a transaction: its own, or one you open.

```sql
BEGIN;                 -- or START TRANSACTION
...
COMMIT;                -- or END
ROLLBACK;              -- or ABORT
```

A transaction sees a consistent snapshot of the data and its own writes. Nobody else sees them until `COMMIT`, which is one durable write. `SHOW TRANSACTION ISOLATION LEVEL` reports `repeatable read`.

DEP CLI marks the prompt: `=*>` inside a transaction, `=!>` after an error.

```text
admin@operational=> BEGIN;
BEGIN

admin@operational=*> INSERT INTO patients (patient_id, national_id, phone, full_name, health_fund)
VALUES (7, '789012348', '+972-50-0000007', 'Lior Katz', 'Clalit');
DEP:  PROTECTED column patients.national_id (MATCH DOMAIN person_id) is stored as a blind index and ciphertext.
DEP:  PROTECTED column patients.phone (MATCH DOMAIN phone_number) is stored as a blind index and ciphertext.
INSERT 0 1

admin@operational=*> SELECT full_name FROM patients WHERE national_id = '789012348';
DEP:  PROTECTED column patients.national_id is matched by equality through the blind index of MATCH DOMAIN person_id.
 full_name
-----------
 Lior Katz
(1 row)

admin@operational=*> ROLLBACK;
ROLLBACK

admin@operational=> SELECT count(*) FROM patients;
 count(*)
----------
 6
(1 row)
```

### After an error

An error inside a transaction aborts it. Every later statement fails with `25P02` until you run `ROLLBACK`, or `ROLLBACK TO` a savepoint.

```text
admin@operational=> BEGIN;
BEGIN

admin@operational=*> INSERT INTO patients (patient_id, national_id, phone, full_name, health_fund)
VALUES (7, '789012348', '+972-50-0000007', 'Lior Katz', 'Clalit');
DEP:  PROTECTED column patients.national_id (MATCH DOMAIN person_id) is stored as a blind index and ciphertext.
DEP:  PROTECTED column patients.phone (MATCH DOMAIN phone_number) is stored as a blind index and ciphertext.
INSERT 0 1

admin@operational=*> INSERT INTO patients (patient_id, national_id, phone, full_name, health_fund)
VALUES (8, '789012348', '+972-50-0000008', 'Second', 'Clalit');
ERROR:  duplicate key value violates unique constraint "patients_national_id_key": Key (national_id) already exists.
SQLSTATE:  23505

admin@operational=!> SELECT 1;
ERROR:  current transaction is aborted, commands ignored until end of transaction block
HINT:  ROLLBACK (or ROLLBACK TO a savepoint), then run the statements again.
SQLSTATE:  25P02

admin@operational=!> ROLLBACK;
ROLLBACK
```

The second insert reuses an ID number, and the unique constraint refuses it without showing the value. A DEP rule violation, such as an unanchored join, aborts a transaction in the same way.

### Savepoints

`SAVEPOINT name`, `ROLLBACK TO SAVEPOINT name` and `RELEASE SAVEPOINT name` work as in PostgreSQL. Rolling back to a savepoint also ends the aborted state.

```text
admin@operational=> BEGIN;
BEGIN

admin@operational=*> SAVEPOINT before_change;
SAVEPOINT

admin@operational=*> UPDATE patients SET health_fund = 'Leumit' WHERE patient_id = 1;
UPDATE 1

admin@operational=*> SELECT health_fund FROM patients WHERE patient_id = 1;
 health_fund
-------------
 Leumit
(1 row)

admin@operational=*> ROLLBACK TO SAVEPOINT before_change;
ROLLBACK

admin@operational=*> SELECT health_fund FROM patients WHERE patient_id = 1;
 health_fund
-------------
 Clalit
(1 row)

admin@operational=*> COMMIT;
COMMIT
```

### Schema changes are transactional

`CREATE` and `DROP` of tables, indexes, functions, roles, grants and endpoints happen on the transaction's own copy of the catalog. Other connections see them after `COMMIT`, and `ROLLBACK` undoes them.

```text
admin@operational=> BEGIN;
BEGIN

admin@operational=*> CREATE TABLE visitors (visitor_id BIGINT PRIMARY KEY, name TEXT);
CREATE TABLE

admin@operational=*> INSERT INTO visitors VALUES (1, 'Guest');
INSERT 0 1

admin@operational=*> SHOW TABLES;
      table      |  protected_columns  | rows | temporary
-----------------+---------------------+------+-----------
 appointments    |                     | 10   |
 medical_records | patient_national_id | 6    |
 patients        | national_id, phone  | 6    |
 visitors        |                     | 1    |
(4 rows)

admin@operational=*> ROLLBACK;
ROLLBACK

admin@operational=> SHOW TABLES;
      table      |  protected_columns  | rows | temporary
-----------------+---------------------+------+-----------
 appointments    |                     | 10   |
 medical_records | patient_national_id | 6    |
 patients        | national_id, phone  | 6    |
(3 rows)
```

`ALTER SYSTEM ROTATE MASTER KEY` is the exception: it cannot run inside a transaction block (`25001`).

### Concurrent transactions

Readers never wait for writers, and never see uncommitted data. **One transaction writes at a time.** A second writer waits up to `--lock-timeout` (30 seconds by default) and then fails with `55P03`. A transaction that writes a table which another transaction changed after it began fails with `40001`; roll back and run it again. This script starts the server with `--lock-timeout 2` and shows both:

```python
import psycopg
dsn = "host=127.0.0.1 port=5433 user=admin password=admin dbname=operational sslmode=disable"
a = psycopg.connect(dsn, autocommit=True)
b = psycopg.connect(dsn, autocommit=True)

a.execute("BEGIN")
a.execute("UPDATE patients SET health_fund = 'Maccabi' WHERE patient_id = 1")
print("a sees:", a.execute("SELECT health_fund FROM patients WHERE patient_id = 1").fetchall())
print("b sees:", b.execute("SELECT health_fund FROM patients WHERE patient_id = 1").fetchall())
try:
    b.execute("UPDATE patients SET health_fund = 'Leumit' WHERE patient_id = 2")
except psycopg.Error as e:
    print("b write:", e.sqlstate, str(e).strip())
a.execute("COMMIT")
print("b now sees:", b.execute("SELECT health_fund FROM patients WHERE patient_id = 1").fetchall())

# b takes a snapshot, then a commits a change to patients; b may no longer write that table
b.execute("BEGIN")
b.execute("SELECT count(*) FROM patients")
a.execute("UPDATE patients SET health_fund = 'Clalit' WHERE patient_id = 1")
try:
    b.execute("UPDATE patients SET health_fund = 'Leumit' WHERE patient_id = 2")
except psycopg.Error as e:
    print("b write after a committed:", e.sqlstate, str(e).strip())
b.execute("ROLLBACK")
```

```text
a sees: [('Maccabi',)]
b sees: [('Clalit',)]
b write: 55P03 could not obtain the write lock within 2 s: another transaction is writing
HINT:  Commit or roll back the other transaction, or retry.
b now sees: [('Maccabi',)]
b write after a committed: 40001 could not serialize access due to a concurrent update of patients
HINT:  Another transaction committed a change to this table after this transaction began. Roll back and run it again.
```

A transaction left idle holds the write lock, so the server ends it. After `--idle-in-transaction-timeout` (300 seconds by default; `0` never) the transaction is rolled back and the next statement on the connection fails with `25P03`.

| SQLSTATE | Meaning | What to do |
|---|---|---|
| `25P02` | The transaction is aborted | `ROLLBACK`, or `ROLLBACK TO` a savepoint |
| `40001` | Another transaction changed a table you want to write | Roll back and retry |
| `55P03` | The write lock was not free within `--lock-timeout` | Retry, or find the long transaction |
| `25P03` | The transaction was idle too long and was rolled back | Start again, and keep transactions short |
| `25001` | `ALTER SYSTEM ROTATE MASTER KEY` inside a transaction | Run it on its own |

## Constraints

`PRIMARY KEY` (single or composite), `UNIQUE`, `NOT NULL` and `REFERENCES` (`NO ACTION`) are enforced, with PostgreSQL's SQLSTATEs. `DEFAULT`, `CHECK` and `ON DELETE CASCADE` are not supported.

```text
admin@operational=> INSERT INTO patients (patient_id, national_id) VALUES (1, '999999990');
ERROR:  duplicate key value violates unique constraint "patients_pkey": Key (patient_id)=(1) already exists.
SQLSTATE:  23505

admin@operational=> INSERT INTO patients (patient_id, national_id) VALUES (NULL, '999999990');
ERROR:  null value in column "patient_id" of relation "patients" violates not-null constraint
SQLSTATE:  23502

admin@operational=> INSERT INTO appointments VALUES (999, 77, '2026-10-30 10:00:00', 'Follow-up');
ERROR:  insert or update on table "appointments" violates foreign key constraint "appointments_patient_id_fkey": Key (patient_id)=(77) is not present in table "patients".
SQLSTATE:  23503

admin@operational=> DELETE FROM patients WHERE patient_id = 1;
ERROR:  update or delete on table "patients" violates foreign key constraint "appointments_patient_id_fkey" on table "appointments": Key (patient_id)=(1) is still referenced from table "appointments".
SQLSTATE:  23503
```

| SQLSTATE | Constraint |
|---|---|
| `23505` | A primary or unique key already exists |
| `23502` | A `NOT NULL` column received NULL |
| `23503` | A reference points at nothing, or a referenced row is changed or deleted |

A constraint on an ordinary column shows its key in the message. A constraint on a **protected** column never does: the unique key on `national_id` is checked through blind index entries, and the error names the column only.

```text
admin@operational=> INSERT INTO patients (patient_id, national_id, phone, full_name, health_fund)
VALUES (9, '123456782', '+972-50-0000009', 'Copy', 'Clalit');
ERROR:  duplicate key value violates unique constraint "patients_national_id_key": Key (national_id) already exists.
SQLSTATE:  23505

admin@operational=> INSERT INTO medical_records VALUES (300, '000000000', 'Asthma', DATE '2026-10-01');
ERROR:  insert or update on table "medical_records" violates foreign key constraint "medical_records_patient_national_id_fkey": Key (patient_national_id) is not present in table "patients".
SQLSTATE:  23503
```

## Indexes

The engine keeps an index for every primary key, unique key, reference and protected column. On a protected column the key is the blind index entry, so a lookup never needs a plaintext value. `CREATE [UNIQUE] INDEX` adds more, and `DROP INDEX` removes only those you made.

```text
admin@operational=> CREATE INDEX appointments_by_type ON appointments (visit_type);
CREATE INDEX

admin@operational=> CREATE UNIQUE INDEX appointments_by_time ON appointments (starts_at);
CREATE INDEX

admin@operational=> CREATE UNIQUE INDEX appointments_by_type_u ON appointments (visit_type);
ERROR:  duplicate key value violates unique constraint "appointments_by_type_u": Key (visit_type)=(Follow-up) already exists.
HINT:  The table has rows with equal keys; remove the duplicates first.
SQLSTATE:  23505

admin@operational=> SHOW INDEXES FROM appointments;
    table     |            index            |    columns     |    kind     | enforced |   made_by    |  keys
--------------+-----------------------------+----------------+-------------+----------+--------------+--------
 appointments | appointments_pkey           | appointment_id | primary key | yes      | engine       | values
 appointments | appointments_patient_id_idx | patient_id     |             |          | engine       | values
 appointments | appointments_by_type        | visit_type     |             |          | CREATE INDEX | values
 appointments | appointments_by_time        | starts_at      | unique      | yes      | CREATE INDEX | values
(4 rows)

admin@operational=> DROP INDEX appointments_by_type;
DROP INDEX

admin@operational=> DROP INDEX appointments_pkey;
ERROR:  cannot drop index appointments_pkey: the engine keeps it for a key, a reference or a PROTECTED column of appointments
SQLSTATE:  2BP01
```

An index on a protected column answers equality only:

```text
admin@operational=> CREATE INDEX phones ON patients (phone);
DEP:  Index phones keys PROTECTED column(s) phone by their blind index entries: it answers equality only, and holds no plaintext.
CREATE INDEX
```

`REINDEX TABLE t` rebuilds the indexes of a table. After a [data directory migration](administration.md#upgrading-and-migrating), it also makes a key enforced again once its duplicates are removed.

## EXPLAIN

`EXPLAIN` shows the plan after the DEP rewrite, and below it a `Read:` section that says how each table is read: an index lookup, or every row. Reading by an index touches only the matching rows; reading every row is a scan.

```text
admin@operational=> EXPLAIN SELECT * FROM appointments WHERE appointment_id = 101;
                                                      dep_plan
---------------------------------------------------------------------------------------------------------------------
 Projection: appointments.appointment_id, appointments.patient_id, appointments.starts_at, appointments.visit_type
   Projection: appointments.appointment_id, appointments.patient_id, appointments.starts_at, appointments.visit_type
     Filter: appointments.appointment_id = Int64(101)
       TableScan: appointments
 Read:
   appointments: index appointments_pkey, 1 key(s)
(6 rows)

admin@operational=> EXPLAIN SELECT * FROM appointments WHERE starts_at > '2026-10-01';
                                                      dep_plan
---------------------------------------------------------------------------------------------------------------------
 Projection: appointments.appointment_id, appointments.patient_id, appointments.starts_at, appointments.visit_type
   Projection: appointments.appointment_id, appointments.patient_id, appointments.starts_at, appointments.visit_type
     Filter: appointments.starts_at > Utf8("2026-10-01")
       TableScan: appointments
 Read:
   appointments: every row
(6 rows)

admin@operational=> EXPLAIN SELECT full_name FROM patients WHERE national_id = '123456782';
DEP:  PROTECTED column patients.national_id is matched by equality through the blind index of MATCH DOMAIN person_id.
                                             dep_plan
--------------------------------------------------------------------------------------------------
 Projection: patients.full_name
   Projection: patients.full_name
     Filter: patients.national_id = dep_index(Utf8("person_id"), CAST(Utf8("123456782") AS Utf8))
       TableScan: patients
 Read:
   patients: index patients_national_id_key, 1 key(s)
(6 rows)
```

In the last plan the protected value has become a call of the internal function `dep_index`, which computes the blind index entry. The statement and its plan are shown to the role that sent them, and never recorded in the audit log.

`/metrics` counts index lookups, full scans and rows read; see the [administration guide](administration.md#metrics).
