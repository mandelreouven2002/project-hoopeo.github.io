# privacy-check

A small checker that keeps a DEP database equal to the privacy decisions approved for it. It implements steps 6 and 7 of the design method in [chapter 4 of the DEP guide](../../docs/book/04-design.md#107-step-7-keep-the-matrix-and-the-database-in-sync): tests compiled from the privacy matrix, and a drift report that compares the matrix with the database's own catalog.

## What it does

The privacy matrix is a YAML file kept with the schema: one row per protected column, one column per workflow, and in each cell `MATCH`, `LINK`, `DISCLOSE` or nothing (`NONE`), with a reason. `clinic/matrix.yaml` is the matrix of the clinic in `examples/clinic.sql`.

The checker runs three checks against a running DEP Engine:

| Check | What it verifies |
| --- | --- |
| **lint** | Every decision other than NONE has a reason; every DISCLOSE and LINK names the declared function that implements it. |
| **tests** | The matrix compiled into statements, each run as the workflow's role through its path, with the expected outcome: a lookup for every MATCH cell, a refused lookup for every NONE cell on the operational path, an anchored and a refused unanchored join for every reference between protected columns, a call for every DISCLOSE and LINK cell and a refused call for every other role, and a refused login through the other path. |
| **drift** | What the database allows equals what the matrix decides. It reads `SHOW PRIVACY MATRIX`, `SHOW FUNCTIONS` and `SHOW GRANTS`, and reports every MATCH, DISCLOSE or LINK the database allows and the matrix does not (and the reverse), every protected column without a row, every declared function that no cell names or that records no purpose, every login through a path a role's workflows do not use, and every role that may declare functions for a domain without being listed for it. |

Tests and drift answer different questions. Tests are generated from the matrix, so they can only exercise what the matrix already knows. A new disclosing function, a grant that has no effect on a role's current path, or a declaration privilege changes no test result, and only the drift check sees it.

## Usage

Requires Python 3 and PyYAML (`pip install pyyaml`).

```sh
dep-engine --init examples/clinic.sql --http-listen 127.0.0.1:8433 --admin-password admin &
python3 tools/privacy-check/privacy_check.py tools/privacy-check/clinic/matrix.yaml --url http://127.0.0.1:8433
```

```text
10 decisions in 9 cells; lint 0, drift 0, tests 58/58 passed
```

The exit status is 0 when the database equals the matrix and every test passes, and 1 otherwise, so the checker can gate a build. Options: `--admin user:password` for the account that reads the catalog (default `admin:admin`), `--password` for the test roles (default `demo`), `--json FILE` for a machine-readable report, and `--quiet`.

## An unreviewed change

A migration adds a function that discloses phone numbers for marketing, without a cell in the matrix:

```sql
SET ROLE privacy_admin;
CREATE FUNCTION marketing_contacts() RETURNS TABLE (full_name TEXT, phone TEXT) LANGUAGE sql DISCLOSES phone_number
AS $$ SELECT full_name, REVEAL(phone) FROM patients $$;
COMMENT ON FUNCTION marketing_contacts() IS 'Purpose: promotional SMS. Owner: marketing.';
GRANT EXECUTE ON FUNCTION marketing_contacts() TO app_developer;
RESET ROLE;
```

```text
drift:  DISCLOSE phone_number for app_developer through marketing_contacts(): in the database, not in the matrix
drift:  marketing_contacts() is declared DISCLOSES phone_number but no matrix cell uses it
10 decisions in 9 cells; lint 0, drift 2, tests 58/58 passed
```

Every test still passes, because no workflow in the matrix changed. Adding the cell `patients.phone × marketing = DISCLOSE` (with its workflow and reason) to the matrix makes the check pass again: the decision is then one reviewed line of the matrix.

## Scope

- The checker compares disclosure and linkage per match domain, as the engine reports them, not per column.
- Matching through an undeclared function: such a function runs with its owner's privileges, so `EXECUTE` on one that reads a protected column lets a role match it. `SHOW PRIVACY MATRIX` in DEP Engine 0.5.2 has no `matches_via` column for this (one is planned). The checker reads it when it exists and reports it as an undeclared MATCH. Until then, review `EXECUTE` grants on undeclared functions by hand.
- It does not check paths of ordinary keys between identities and sensitive data, what a declared function's body returns beyond its declarations, or anything about administrators.
- It checks a test database built from your migrations, or a live one; it does not run an approval workflow.

Licensed under the Business Source License 1.1, like the rest of `tools/` (see `LICENSING.md`).
