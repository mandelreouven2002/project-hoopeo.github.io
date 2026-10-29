# Declared functions

<span id="8-declared-functions"></span>

**In one sentence:** a declared function is a SQL function whose signature states that it discloses protected values, links datasets through them, or both, and it is the only place where either can happen.

## Why declared functions are needed {#81-why-declared-functions-are-needed}

**What problem do they solve?** Most of the clinic's work is *pull* work: someone arrives with a value and asks for the matching record. Reception and the doctors work this way, with anchored matching, and never need the value back. Two kinds of legitimate work do not fit that pattern:

| Kind of work | In the clinic | Why ordinary queries cannot do it |
| --- | --- | --- |
| **Releasing the values themselves** | The monthly report to the health funds; appointment reminders by SMS; diagnosis claims | The database is the only party that knows the values at that moment, and ordinary queries never return them |
| **Linking whole datasets** | Diagnoses per health fund, for the dashboard | Ordinary queries may match only one value at a time |

**Why not a permission?** A general "read permission" or "link permission" would collapse back into the column privileges DEP was meant to improve on. DEP instead forbids both by default and makes each exception an explicit construct of the language, as Rust does with `unsafe`. There are two such constructs: `DISCLOSES` and `LINKS`.

<figure markdown="1">

<picture><source media="(prefers-color-scheme: dark)" srcset="../images/diagrams/declared-functions-dark.png"><img src="../images/diagrams/declared-functions.png" alt="Disclosure and linkage are separate declarations, each with domain privileges and explicit callers." width="1000"></picture>

<figcaption>Disclosure and linkage are separate declarations, each with domain privileges and explicit callers.</figcaption>
</figure>

## REVEAL {#82-reveal}

**What is it?** `REVEAL` is the only operation that turns a protected value into an ordinary one.

**Syntax**

```sql
REVEAL ( protected_column )
```

**Parameters**

| Parameter | Meaning |
| --- | --- |
| `protected_column` | A protected column of match domain `d`. |

**Return value.** The value, with the base type of the column: `REVEAL` of a `TEXT PROTECTED` column returns `TEXT`. The result is a **disclosed value** ([Disclosed values and the terminal rule](03-functions.md#89-disclosed-values-and-the-terminal-rule)).

**Example 1: `REVEAL` inside a disclosing function**

```sql
CREATE FUNCTION monthly_fund_report(report_month DATE)
RETURNS TABLE (national_id TEXT, visit_date DATE, visit_type TEXT, health_fund TEXT)
LANGUAGE sql
DISCLOSES person_id
AS $$
    SELECT REVEAL(p.national_id), a.starts_at::date, a.visit_type, p.health_fund
    FROM appointments AS a
    JOIN patients AS p USING (patient_id)
    WHERE date_trunc('month', a.starts_at) = date_trunc('month', CAST(report_month AS TIMESTAMP));
$$;
```

**Output**

```text
CREATE FUNCTION
```

**Explanation:** `REVEAL(p.national_id)` is allowed, because the function declares `DISCLOSES person_id` and `national_id` belongs to `person_id`.

**Example 2: `REVEAL` in an ordinary query**

```sql
SELECT REVEAL(national_id) FROM patients;
```

**Output**

```text
ERROR:  DEP_REVEAL_OUTSIDE_DISCLOSURE
DETAIL: REVEAL() of patients.national_id (match domain person_id) appears outside a disclosing function.
HINT:   Move the disclosure into a function declared DISCLOSES person_id.
```

**Explanation:** outside a disclosing function, `REVEAL` does not exist as far as the client is concerned.

**Example 3: `REVEAL` of the wrong domain**

```sql
CREATE FUNCTION phones_and_ids()
RETURNS TABLE (phone TEXT, national_id TEXT)
LANGUAGE sql
DISCLOSES phone_number
AS $$
    SELECT REVEAL(phone), REVEAL(national_id) FROM patients;
$$;
```

**Output**

```text
ERROR:  DEP_REVEAL_OUTSIDE_DISCLOSURE
DETAIL: REVEAL() of patients.national_id (match domain person_id) appears outside a disclosing function.
```

**Explanation:** the function may reveal phone numbers, but not ID numbers. Each domain must be declared separately. The check runs when the function is created, not only when it runs.

**Errors.** [`DEP_REVEAL_OUTSIDE_DISCLOSURE`](../reference/errors.md#dep-errors).

## CREATE FUNCTION ... DISCLOSES {#83-create-function--discloses}

**What is it?** A **disclosing function** is a function declared `DISCLOSES d`. Inside its body, values of match domain `d` may be revealed.

**Why is it needed?** For any workflow that needs protected values to leave the database: a report, a file for an external system, a message to a person.

**Syntax**

```sql
CREATE FUNCTION function_name ( [ parameter_name type [, ...] ] )
RETURNS TABLE ( column_name type [, ...] )
LANGUAGE sql
DISCLOSES domain_name [, ...]
[ LINKS domain_name [, ...] ]
AS $$
    query
$$;
```

**Parameters**

| Clause | Meaning |
| --- | --- |
| `function_name`, parameters | As in any PostgreSQL function. |
| `RETURNS TABLE (...)` | The columns the caller receives. A disclosed column has the base type of the protected column it comes from. |
| `LANGUAGE sql` | Required, so the engine can analyze the body when the function is created. |
| `DISCLOSES domain_name` | The match domains whose values the body may `REVEAL`. |
| `LINKS domain_name` | Optional; also allows unanchored matching in that domain ([Declaring both](03-functions.md#85-declaring-both)). |
| `AS $$ ... $$` | The function body, in ordinary PostgreSQL syntax. |

**Working with the loaded clinic.** Its functions already exist. Use SHOW FUNCTIONS to inspect them. The definitions below explain their construction; an administrator can use CREATE OR REPLACE FUNCTION to update one, or choose a new function name when experimenting.

**Example 1: Calling the monthly report**

The billing job calls the function from [REVEAL](03-functions.md#82-reveal) with ordinary SQL:

```sql
SELECT * FROM monthly_fund_report('2026-10-01');
```

**Output**

| national_id | visit_date | visit_type | health_fund |
| --- | --- | --- | --- |
| 123456782 | 2026-10-14 | Follow-up | Clalit |

**Explanation:** the billing job receives a real ID number, as ordinary `TEXT`, because that is what the signature promises. This is one of only two places in the clinic where ID numbers can leave the database.

**Example 2: Reminders, in the other domain**

```sql
CREATE FUNCTION appointment_reminders(day DATE)
RETURNS TABLE (phone TEXT, starts_at TIMESTAMPTZ)
LANGUAGE sql
DISCLOSES phone_number
AS $$
    SELECT REVEAL(p.phone), a.starts_at
    FROM appointments AS a
    JOIN patients AS p USING (patient_id)
    WHERE CAST(a.starts_at AS DATE) = day;
$$;

SELECT * FROM appointment_reminders(DATE '2026-10-21');     -- as reminder_job
```

**Output**

| phone | starts_at |
| --- | --- |
| +972-50-0000001 | 2026-10-21 10:30 |

**Explanation:** the function returns the phone number and the time, and nothing else. A reminder SMS does not need to say what the visit is for, so the visit type never leaves with the phone number. A declared function should return only what its purpose requires.

**Example 3: Calling a disclosing function from the masked path**

```sql
SELECT * FROM monthly_fund_report('2026-10-01');     -- as bi_dashboard
```

**Output**

```text
ERROR:  DEP_ENDPOINT_OPERATION
DETAIL: Disclosing functions are unavailable on a MASKED_READ connection.
```

**Explanation:** disclosure happens only on the operational path, even for a role that somehow holds `EXECUTE`.

**Rules in detail.** The creator needs `DISCLOSE ON MATCH DOMAIN d` ([GRANT DISCLOSE and GRANT LINK](03-functions.md#86-grant-disclose-and-grant-link)); the function is never executable by `PUBLIC` ([GRANT EXECUTE on declared functions](03-functions.md#87-grant-execute-on-declared-functions)); it runs only on the operational path; and its disclosed output columns are terminal ([Disclosed values and the terminal rule](03-functions.md#89-disclosed-values-and-the-terminal-rule)). The full list is in [The rules for declared functions](03-functions.md#88-the-rules-for-declared-functions).

**Errors.** [`DEP_DECLARATION_PRIVILEGE`](../reference/errors.md#dep-errors), [`DEP_DECLARED_LANGUAGE`](../reference/errors.md#dep-errors), [`DEP_DECLARED_CALL`](../reference/errors.md#dep-errors), [`DEP_REVEAL_OUTSIDE_DISCLOSURE`](../reference/errors.md#dep-errors), [`DEP_ENDPOINT_OPERATION`](../reference/errors.md#dep-errors).

## CREATE FUNCTION ... LINKS {#84-create-function--links}

**What is it?** A **linking function** is a function declared `LINKS d`. Inside its body, protected values of domain `d` may be matched without an anchor, linking whole datasets.

**Why is it needed?** For a workflow that must connect whole datasets through a protected value, typically to compute statistics, without releasing the values.

**Syntax**

```sql
CREATE FUNCTION function_name ( [ parameter_name type [, ...] ] )
RETURNS TABLE ( column_name type [, ...] )
LANGUAGE sql
LINKS domain_name [, ...]
AS $$
    query
$$;
```

**Example 1: Diagnoses per health fund**

```sql
CREATE FUNCTION diagnoses_per_fund()
RETURNS TABLE (health_fund TEXT, diagnosis TEXT, patients BIGINT)
LANGUAGE sql
LINKS person_id
AS $$
    SELECT p.health_fund, m.diagnosis, COUNT(*)
    FROM patients AS p
    JOIN medical_records AS m ON m.patient_national_id = p.national_id
    GROUP BY p.health_fund, m.diagnosis;
$$;

GRANT EXECUTE ON FUNCTION diagnoses_per_fund() TO bi_dashboard;
```

**Output**

```text
CREATE FUNCTION
GRANT
```

**Explanation:** the join inside the body has no anchor; it links every patient to every diagnosis. That is allowed only because the function declares `LINKS person_id`.

**Example 2: The dashboard calls it from the masked path**

```sql
SELECT * FROM diagnoses_per_fund();
```

**Output**

| health_fund | diagnosis | patients |
| --- | --- | --- |
| Clalit | Diabetes | 412 |
| Maccabi | Asthma | 187 |

**Explanation:** the function returns only counts, so it discloses nothing. The dashboard receives the result of a linkage the clinic approved, without gaining any ability to link anything itself. It sends no protected value, so it cannot use the function to probe for one, which is why a linking function without protected input may run on the masked path.

**Example 3: The dashboard tries the same join itself**

```sql
SELECT p.health_fund, m.diagnosis, COUNT(*)
FROM patients AS p
JOIN medical_records AS m ON m.patient_national_id = p.national_id
GROUP BY p.health_fund, m.diagnosis;
```

**Output**

```text
ERROR:  DEP_ENDPOINT_OPERATION
DETAIL: Matching PROTECTED values is unavailable on a MASKED_READ connection.
```

**Explanation:** the permission belongs to the reviewed function, not to the caller. The same SQL, written outside the function, is rejected.

**Rules in detail**

1. The creator needs `LINK ON MATCH DOMAIN d`.
2. `LINKS` alone does not permit `REVEAL`.
3. A linking function that takes **no protected input** may be called on both paths.
4. A linking function that takes a protected value as a parameter may be called only on the operational path, because calling it is itself a form of matching.

**Errors.** [`DEP_DECLARATION_PRIVILEGE`](../reference/errors.md#dep-errors), [`DEP_ENDPOINT_OPERATION`](../reference/errors.md#dep-errors).

## Declaring both {#85-declaring-both}

**What is it?** A function may declare both `DISCLOSES d` and `LINKS d` when it must link datasets *and* release values.

**Example: The diagnosis claims**

```sql
CREATE FUNCTION fund_claims_report(report_month DATE)
RETURNS TABLE (national_id TEXT, diagnosis TEXT, recorded_at DATE)
LANGUAGE sql
DISCLOSES person_id
LINKS person_id
AS $$
    SELECT REVEAL(p.national_id), m.diagnosis, m.recorded_at
    FROM medical_records AS m
    JOIN patients AS p ON p.national_id = m.patient_national_id
    WHERE date_trunc('month', CAST(m.recorded_at AS TIMESTAMP)) = date_trunc('month', CAST(report_month AS TIMESTAMP));
$$;

GRANT EXECUTE ON FUNCTION fund_claims_report(DATE) TO billing_job;
```

**Explanation:**

- The join links every diagnosis of the month to its patient, which requires `LINKS person_id`.
- The output includes the ID number, which requires `DISCLOSES person_id`.
- "Returns ID numbers" and "links patients to diagnoses" are two different privacy decisions, and a reviewer should see both in the signature. `DISCLOSES` alone does not permit linking, and `LINKS` alone does not permit `REVEAL`.

**The working idiom: match first, reveal last.** The join happens through internal matching on protected fields, and `REVEAL` appears only in the final projection, on the column the value is disclosed from.

## GRANT DISCLOSE and GRANT LINK {#86-grant-disclose-and-grant-link}

**What is it?** `GRANT DISCLOSE` and `GRANT LINK` give a role the privilege to *create* disclosing and linking functions for one match domain.

**Why is it needed?** Declaring a function is adding "unsafe code" to the database. Only designated roles should be able to do it, domain by domain.

**Syntax**

```sql
GRANT  { DISCLOSE | LINK } ON MATCH DOMAIN domain_name TO   role_name [, ...];
REVOKE { DISCLOSE | LINK } ON MATCH DOMAIN domain_name FROM role_name [, ...];
```

**Parameters**

| Parameter | Meaning |
| --- | --- |
| `DISCLOSE` | Allows creating or altering functions declared `DISCLOSES domain_name`. |
| `LINK` | Allows creating or altering functions declared `LINKS domain_name`. |
| `domain_name` | The match domain the privilege applies to. |
| `role_name` | The role receiving the privilege. |

**Example 1: The clinic's privacy administrator**

```sql
GRANT DISCLOSE ON MATCH DOMAIN person_id    TO privacy_admin;
GRANT LINK     ON MATCH DOMAIN person_id    TO privacy_admin;
GRANT DISCLOSE ON MATCH DOMAIN phone_number TO privacy_admin;
```

**Explanation:** every declared function in the clinic is created by `privacy_admin`. Note what is missing: nobody may declare `LINKS phone_number`, so phone numbers can never be used to link datasets.

**Example 2: A developer tries to declare a linking function**

```sql
-- as app_developer
CREATE FUNCTION all_diagnoses() RETURNS TABLE (diagnosis TEXT)
LANGUAGE sql LINKS person_id AS $$ ... $$;
```

**Output**

```text
ERROR:  DEP_DECLARATION_PRIVILEGE
DETAIL: Role app_developer may not declare functions LINKS person_id.
HINT:   This function must be created by a role holding LINK ON MATCH DOMAIN person_id.
```

**Explanation:** the developer may write the function, but it must be reviewed and created by a role holding the privilege.

**Notes.** These are the only two privileges DEP adds. They decide who may *write* declared functions, not who may *call* them.

## GRANT EXECUTE on declared functions {#87-grant-execute-on-declared-functions}

**What is it?** Who may *call* a declared function is decided by the ordinary `GRANT EXECUTE`, with one difference: declared functions are never executable by `PUBLIC`.

**Syntax**

```sql
GRANT EXECUTE ON FUNCTION function_name ( argument_types ) TO role_name [, ...];
```

**Example 1: Granting the reports to their jobs**

```sql
GRANT EXECUTE ON FUNCTION monthly_fund_report(DATE) TO billing_job;
GRANT EXECUTE ON FUNCTION appointment_reminders(DATE)      TO reminder_job;
```

**Explanation:** each consumer appears in the grants by name, so the list of who may receive disclosed values is explicit.

**Example 2: Opening a disclosing function to everyone**

```sql
GRANT EXECUTE ON FUNCTION monthly_fund_report(DATE) TO PUBLIC;
```

**Output**

```text
ERROR:  DEP_PUBLIC_EXECUTE
DETAIL: monthly_fund_report is declared DISCLOSES person_id and cannot be executable by PUBLIC.
HINT:   Grant EXECUTE to the specific roles that need it.
```

**Explanation:** in PostgreSQL, new functions are executable by `PUBLIC` by default. For declared functions that default would make every disclosure available to everyone, so DEP removes it.

**Rules in detail**

1. Declared functions are created with no `EXECUTE` privilege for anyone but their owner.
2. Granting `EXECUTE` on a declared function to `PUBLIC` is rejected.
3. A caller needs only `EXECUTE`, not privilege on the protected columns the function matches on internally ([The rules for declared functions](03-functions.md#88-the-rules-for-declared-functions), rule 8).

**Errors.** [`DEP_PUBLIC_EXECUTE`](../reference/errors.md#dep-errors).

## The rules for declared functions {#88-the-rules-for-declared-functions}

| # | Rule | What it means |
| --- | --- | --- |
| 1 | **Location of `REVEAL`** | If `x` has type `T PROTECTED` in domain `d`, then `REVEAL(x)` has type `T` and is valid only in the body of a function declared `DISCLOSES d`. Checked at creation. |
| 2 | **Location of unanchored matching** | Unanchored matching on values of domain `d` is valid only in the body of a function declared `LINKS d`. |
| 3 | **Analyzable bodies** | Declared functions must be `LANGUAGE sql`. They may call user-defined functions only if those are declared for the same purpose and domain; otherwise a protected or revealed value could reach code that was never reviewed. |
| 4 | **Disclosed values are terminal** | Values produced by `REVEAL` may be returned to the client, renamed or cast, but not used in a predicate, join condition, grouping, `DISTINCT`, aggregate, sort key, insert, or as the argument of a function or operator ([Disclosed values and the terminal rule](03-functions.md#89-disclosed-values-and-the-terminal-rule)). |
| 5 | **Paths** | Disclosing functions run only on the operational path. Linking functions run on the operational path, and on the masked path when they take no protected input. |
| 6 | **Declaring is privileged** | `DISCLOSES d` requires `DISCLOSE ON MATCH DOMAIN d`; `LINKS d` requires `LINK ON MATCH DOMAIN d` ([GRANT DISCLOSE and GRANT LINK](03-functions.md#86-grant-disclose-and-grant-link)). |
| 7 | **No public execution** | Declared functions are never executable by `PUBLIC` ([GRANT EXECUTE on declared functions](03-functions.md#87-grant-execute-on-declared-functions)). |
| 8 | **Matching runs under the declaration** | The matches inside a declared function were reviewed as part of the function, so a caller needs only `EXECUTE`, much like `SECURITY DEFINER` in PostgreSQL. That is why the dashboard, which has no privilege on any protected column, can call `diagnoses_per_fund`. |

## Disclosed values and the terminal rule {#89-disclosed-values-and-the-terminal-rule}

**What is it?** A **disclosed value** is a value produced by `REVEAL`. The **terminal rule** says that a disclosed value may only be returned to the client: it is the end of the line inside the database.

**Implementation status:** DEP.js applies the whole rule. DEP Engine 0.5.2 accepts three uses it forbids: a disclosed value as a sort key, as the argument of a function or operator, and as a `DISTINCT` key. See the [known differences](06-server.md#known-difference-sorting-disclosed-values) before relying on the rule in a released Engine.

**Why is it needed?** Without it, the report approved for the health funds could be turned into a tool for linking patients to diagnoses inside the database.

**Example 1: Returning a disclosed value (allowed)**

```sql
SELECT national_id, visit_date FROM monthly_fund_report('2026-10-01');
```

**Output**

| national_id | visit_date |
| --- | --- |
| 123456782 | 2026-10-14 |

**Explanation:** returning the value is exactly what the function is for.

**Example 2: Joining a disclosed value back to diagnoses (rejected)**

```sql
SELECT r.national_id, m.diagnosis
FROM monthly_fund_report('2026-10-01') AS r
JOIN medical_records AS m ON m.patient_national_id = r.national_id;
```

**Output**

```text
ERROR:  DEP_DISCLOSED_VALUE_USE
DETAIL: Column national_id of monthly_fund_report() is a disclosed value and cannot be used in a join condition.
HINT:   Disclosed values can only be returned. Perform the match on PROTECTED fields inside a declared function, before REVEAL.
```

**Explanation:** the signature says `TEXT`, and the developer sees an ordinary column. But the engine recorded that this column comes from `REVEAL`, so it may not be used as a join key. The report cannot become a linking tool.

**Example 3: Filtering a disclosed value (rejected)**

```sql
SELECT r.national_id
FROM monthly_fund_report('2026-10-01') AS r
WHERE r.national_id LIKE '1%';
```

**Output**

```text
ERROR:  DEP_DISCLOSED_VALUE_USE
```

**Explanation:** filtering is a use, not a return. If the billing job needs only some rows, the filter belongs inside the declared function.

**Rules in detail**

1. The engine records which output columns of a declared function come from `REVEAL`.
2. Those columns may be returned to the client, renamed (`AS`) or cast, and nothing else: no predicate, join condition, grouping, `DISTINCT`, aggregate, sort key, insert, or argument of a function or operator (`upper(national_id)`, `national_id || ''`, `CASE`). Nothing is computed from a disclosed value inside the database.
3. A disclosed value keeps its base type. A revealed `DATE` remains a `DATE`; the terminal rule already prevents further processing, so converting everything to text would add no protection.

**What the rule does not prevent**

- **Use after release.** The rule governs what happens inside the database. Once a value is returned, DEP cannot control its use. The billing job can send the ID numbers back as parameters to later queries, but each is then an anchored lookup, audited and rate-limited.
- **Linkage through ordinary keys.** The rule stops the *marked value* from being used as a key. If `monthly_fund_report` also returned `patient_id`, its output could be joined freely to every table that holds `patient_id`:

```sql
-- Would be allowed, if the function returned patient_id: the join uses an ordinary key.
SELECT r.national_id, a.visit_type
FROM monthly_fund_report('2026-10-01') AS r
JOIN appointments AS a USING (patient_id);
```

Two design rules follow, and [Designing with DEP](04-design.md#10-designing-with-dep) applies both: a declared function returns only what its purpose requires (none of the clinic's functions returns `patient_id`), and a key that links sensitive datasets should itself be protected.

**Errors.** [`DEP_DISCLOSED_VALUE_USE`](../reference/errors.md#dep-errors).

## Documenting purpose: COMMENT ON FUNCTION {#810-documenting-purpose-comment-on-function}

**What is it?** The purpose of a declared function is recorded with the standard `COMMENT ON FUNCTION`, which keeps it in the catalog without new syntax.

**Syntax**

```sql
COMMENT ON FUNCTION function_name ( argument_types ) IS 'text';
```

**Example**

```sql
COMMENT ON FUNCTION monthly_fund_report(DATE) IS 'Purpose: monthly visit report to health funds. Owner: billing.';
COMMENT ON FUNCTION appointment_reminders(DATE)      IS 'Purpose: SMS appointment reminders. Owner: operations.';
COMMENT ON FUNCTION diagnoses_per_fund()      IS 'Purpose: diagnosis statistics per health fund. Owner: management.';
COMMENT ON FUNCTION fund_claims_report(DATE)  IS 'Purpose: monthly diagnosis claims to health funds. Owner: billing.';
```

**Explanation:** the comment appears in the `comment` column of `SHOW FUNCTIONS` ([Inspect declared functions](03-functions.md#811-inspect-declared-functions)), next to the declaration it explains.

**Notes.** Whether purpose should become a first-class, machine-checked part of a declaration is an open question ([Roadmap and Release](../design/02-roadmap-and-release.md#7-open-questions)).

## Inspect declared functions {#811-inspect-declared-functions}

Use the implemented `SHOW FUNCTIONS` command to list signatures, `discloses` and `links` domains, owner, masked-path availability, comments, callers and SQL bodies.

```sql
SHOW FUNCTIONS;
SHOW GRANTS;
SHOW PRIVACY MATRIX;
```

The earlier `dep.declared_functions` SQL catalog view remains a design idea, not an implemented query interface. Use [SHOW commands](../reference/show.md#show-functions) for exact columns and examples.

## The mapping to Rust {#812-the-mapping-to-rust}

| Rust | DEP |
| --- | --- |
| An unsafe operation (dereferencing a raw pointer) | `REVEAL`, or unanchored matching on a protected value |
| `unsafe fn` or an `unsafe` block | A function body declared `DISCLOSES d` or `LINKS d` |
| A safe wrapper that exposes an ordinary API | A declared function whose signature returns ordinary types |
| A `# Safety` comment documenting the contract | `COMMENT ON FUNCTION` describing the purpose |
| `cargo geiger`, which counts every `unsafe` in a project | The `SHOW FUNCTIONS` command |

## Future extension: partial disclosure {#813-future-extension-partial-disclosure}

> **Future design.** Suffix disclosure syntax is not supported in DEP Engine 0.5.2.

**What is the idea?** Some recipients need less than the full value. A letter to Dana might show only the last four digits of her ID number, to confirm it reached the right person. A later version could make the *form* of disclosure part of the declaration, so that data minimization becomes a check at creation time:

```sql
DISCLOSES person_id AS SUFFIX(4)
```

**Explanation:** in such a body only `REVEAL_SUFFIX(x, 4)` would be legal; writing the full `REVEAL(x)` would be an error. This is intentionally outside the first version.

**Key points**

- `REVEAL` exists only inside `DISCLOSES` functions; unanchored matching only inside `LINKS` functions.
- Declaring needs a per-domain privilege; calling needs `EXECUTE`, never `PUBLIC`.
- Disclosed values can only be returned, never reused inside the database.
- The catalog lists every declared function, so it is the complete answer to "where can values leave?"

---





<figure markdown="1">

![A permitted disclosure is shown as declared output in the client.](../images/studio/studio-function.png)

<figcaption>A permitted disclosure is shown as declared output in the client.</figcaption>
</figure>
