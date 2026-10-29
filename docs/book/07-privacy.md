# Privacy as Code & GDPR

## Privacy as Code, privacy by design, and the GDPR {#3-privacy-as-code-privacy-by-design-and-the-gdpr}

**In one sentence:** DEP takes three established ways of thinking about privacy (Privacy as Code, privacy by design, and the GDPR's data protection by design and by default) and turns each of them into something the database enforces on every query.

This chapter explains each framework in plain terms, shows what it asks for, and shows how DEP applies it, and where it does not. It uses DEP's constructs before they are defined in detail; each is linked to the chapter that explains it.

### What is Privacy as Code? {#31-what-is-privacy-as-code}

**What is it?** *Privacy as Code* is the practice of writing privacy policy as an executable artifact: kept in a repository, reviewed like code, tested, and checked automatically.

**How is it practiced today?** Most tools in this space work *around* the database:

| Kind of tool | Example | What it does |
| --- | --- | --- |
| Policy evaluation in CI | Fides | Describes systems and datasets with a shared taxonomy of data categories, uses, and subjects, and rejects a change whose declared use violates a policy |
| Code scanning | Privado | Scans application code to find which personal data is collected and where it flows |
| Authorization as code | Open Policy Agent, Cedar | Evaluates access rules written as code, when an application or gateway asks |

**What is missing?** These tools check *code that someone wrote and submitted*. They do not see the query an AI agent composes at run time, or an ad-hoc query an analyst types. The database, the one component every query passes through, enforces none of the policy.

**Example: one policy, two ways.** The clinic's policy is: *patients' ID numbers may leave the database only for reporting to the health funds.*

**Without DEP**, the policy lives outside the database. A CI check can confirm that the billing code declares the right data use, and a scanner can flag code that reads `national_id`. But any role with `SELECT` on the column can still export it, and the new AI agent's query never passes through CI.

**With DEP**, the policy is part of the schema:

```sql
-- The only place ID numbers may leave
CREATE FUNCTION monthly_fund_report(report_month DATE)
RETURNS TABLE (national_id TEXT, visit_date DATE, visit_type TEXT, health_fund TEXT)
LANGUAGE sql
DISCLOSES person_id
AS $$ ... $$;

GRANT EXECUTE ON FUNCTION monthly_fund_report(DATE) TO billing_job;
```

And it can be checked by a query:

```sql
SHOW FUNCTIONS;
```

**Output**

| function_name | kind |
| --- | --- |
| `monthly_fund_report` | DISCLOSES |
| `fund_claims_report` | DISCLOSES, LINKS |

**Explanation:** in DEP, `REVEAL` is legal only inside functions declared `DISCLOSES` ([REVEAL](03-functions.md#82-reveal)). So this list is not documentation that might be out of date: it is the complete set of places where ID numbers can leave, by construction. Any other query, including the agent's, gets `NULL` or an error.

**Comparison**

| | Privacy as Code around the database | DEP |
| --- | --- | --- |
| Where the policy is written | Policy files, annotations, CI configuration | The schema: `PROTECTED`, declared functions, grants |
| When it is checked | Before deployment, on code that was submitted | On every statement, at run time |
| What it can see | Code that passes through the pipeline | Every query, from every client |
| A new AI agent or ad-hoc query | Depends on whether its access passes through the policy enforcement point | The implemented DEP rules apply to its queries |
| Evidence it produces | Scan and CI reports | Metadata and audit records; a project can implement drift reporting |

**How the two fit together.** DEP does not replace these tools; it gives them a point of enforcement. A Fides-style taxonomy can decide which columns should be `PROTECTED`, and a CI pipeline can check that every declared function matches an approved decision ([Step 7: Keep the matrix and the database in sync](04-design.md#107-step-7-keep-the-matrix-and-the-database-in-sync)). The development method in [Designing with DEP](04-design.md#10-designing-with-dep) is built on exactly that idea.

### What is privacy by design? {#32-what-is-privacy-by-design}

**What is it?** *Privacy by design* is a framework developed by Ann Cavoukian, then Information and Privacy Commissioner of Ontario, which holds that privacy should be built into systems from the start rather than added afterwards. It was recognized by the International Conference of Data Protection and Privacy Commissioners in a 2010 resolution adopted in Jerusalem, and its idea is now part of the GDPR as *data protection by design* ([What does the GDPR require?](07-privacy.md#33-what-does-the-gdpr-require)).

**Its seven foundational principles, and how DEP applies them**

| Principle | What it asks | How DEP applies it | Coverage |
| --- | --- | --- | --- |
| **1. Proactive not reactive; preventative not remedial** | Prevent privacy-invasive events before they happen | A forbidden query is rejected before any row is read; ordinary queries cannot export protected values or perform unanchored protected linkage; unsupported forms fail closed | Applies |
| **2. Privacy as the default** | Protect personal data without any action by the individual or the user | Protected values are suppressed by default; a role must be granted its endpoint; analytics consumers can be assigned MASKED_READ; declared functions are never executable by `PUBLIC`; internal keys are ordinary by default | Applies |
| **3. Privacy embedded into design** | Make privacy part of the architecture, not an add-on | Protection is part of the type system and the query semantics; the design method starts from a privacy matrix, before the first table | Applies |
| **4. Full functionality: positive-sum, not zero-sum** | Achieve privacy without giving up legitimate functionality | Reception, doctors, reminders, reports, and statistics all keep working; `SELECT *` keeps working, with `NULL` in place of protected values | Applies |
| **5. End-to-end security: lifecycle protection** | Secure data through its whole lifecycle | Encrypted storage, keyed matching, redaction from logs, and audit inside the database | Partly: DEP covers storage and use inside the database, not collection at the source, retention periods, or deletion schedules |
| **6. Visibility and transparency** | Keep practices open and verifiable | The catalog lists every disclosure and linkage; purposes are recorded; the audit log attributes every lookup; teams can compare metadata with their approved matrix | Applies, to the organization and its reviewers |
| **7. Respect for user privacy** | Keep the individual's interests central: strong defaults, notice, user-friendly controls | Strong defaults, and a subject-access export can be a declared function | Limited: notice and consent are outside the database |

**Explanation:** DEP is strongest on the principles that a database can enforce by itself (1 to 4 and 6). It contributes to the lifecycle and the individual's rights, but those also depend on applications and on organizational processes.

### What does the GDPR require? {#33-what-does-the-gdpr-require}

**Which parts are relevant?** Three parts of the GDPR matter most for a database.

**Article 5: the principles.** Personal data must be processed lawfully, fairly, and transparently; collected for specified purposes (*purpose limitation*); limited to what is necessary (*data minimisation*); kept accurate; kept no longer than necessary (*storage limitation*); and processed securely (*integrity and confidentiality*). The controller must be able to demonstrate all of this (*accountability*, Article 5(2)).

**Article 25(1): data protection by design.** The controller must implement appropriate technical and organisational measures, such as pseudonymisation, designed to implement the data-protection principles effectively, both when deciding how processing will be done and during the processing itself, taking into account the state of the art, the cost, and the risks.

**Article 25(2): data protection by default.** By default, only the personal data necessary for each specific purpose may be processed. The obligation applies to four dimensions: the amount of data collected, the **extent of processing**, the period of storage, and **accessibility**. In particular, personal data must not be made accessible, by default, to an indefinite number of people.

**The EDPB's guidance.** The European Data Protection Board's *Guidelines 4/2019 on Article 25* (version 2.0, adopted 20 October 2020) explain how these duties are applied. Among other things, they expect measures to be effective rather than formal, the state of the art to be assessed continuously, access to personal data to be limited to those who need it and to the types of access they need, and technical limitations on the reuse of data.

**Example: the four dimensions of Article 25(2) in the clinic**

| Dimension | What it means | How DEP applies it |
| --- | --- | --- |
| Amount collected | Collect only what is necessary | Not addressed: DEP stores what is inserted |
| **Extent of processing** | Process only as much as each purpose needs | Matching is limited to one value at a time; linking whole datasets requires a declared `LINKS` function; the only operation on a protected value is equality |
| Period of storage | Keep data only as long as necessary | Not addressed: retention is set by the organization |
| **Accessibility** | Limit who can access the data | Protected values are never returned by ordinary queries; column privileges decide who may match on what; disclosure goes only to roles granted `EXECUTE` by name, and never to `PUBLIC` |

**Explanation:** a receptionist who may look up a patient no longer has access, by default, to all 50,000 ID numbers. The extent of what each role can do is limited to what its purpose needs, and the only exceptions are named, reviewed functions. That is Article 25(2) expressed as database behavior.

**Mapping of DEP to GDPR principles and duties**

| Principle or duty | Supporting DEP construct |
| --- | --- |
| Data minimisation, Art. 5(1)(c) | Ordinary queries never return protected values; analytics and agents use the masked path ([Two access paths](02-sql.md#6-two-access-paths)). |
| Purpose limitation, Art. 5(1)(b) | Each linkage and disclosure is a named, described function with explicit `EXECUTE` grants; each matrix cell records its reason ([Declared functions](03-functions.md#8-declared-functions), [Step 3: Build the privacy matrix](04-design.md#103-step-3-build-the-privacy-matrix)). |
| Data protection by design and by default, Art. 25 | Protection is the default for a `PROTECTED` column; every exception must be declared; internal keys are ordinary by default. |
| Security of processing, Art. 32 | Engine-enforced typing, per-domain keyed matching, encrypted storage of protected values, audited and rate-limited lookups ([Storage and matching](08-internals.md#16-storage-and-matching)). |
| Accountability, Art. 5(2) | The catalog lists every disclosure and linkage; the audit log attributes every lookup to an identity ([DEP and Privacy as Code](04-design.md#9-dep-and-privacy-as-code)). |
| Records of processing, Art. 30 | The catalog supplies the technical part of the record: where identifiers leave, and for which described purpose. |
| Access and erasure, Arts. 15 and 17 | A subject-access export as a `DISCLOSES` function anchored to the requester's identifier; erasure as an anchored, audited `DELETE`. |


### What does privacy engineering add? {#34-what-does-privacy-engineering-add}

**What is privacy engineering?** The discipline of turning privacy goals into system properties. NIST's *Introduction to Privacy Engineering and Risk Management in Federal Systems* (NISTIR 8062, 2017) defines three privacy engineering objectives:

| Objective | NIST's definition | How DEP applies it |
| --- | --- | --- |
| **Predictability** | Enabling reliable assumptions by individuals, owners, and operators about personal information and its processing by a system | The rules are fixed in the schema, and the catalog states exactly where values may leave or be linked |
| **Manageability** | Providing the capability for granular administration of personal information, including alteration, deletion, and selective disclosure | Anchored updates and deletes; disclosure only through declared functions that return only what their purpose requires |
| **Disassociability** | Enabling the processing of personal information or events without association to individuals or devices beyond the operational requirements of the system | The core of DEP: values are used for matching without being returned, and records are associated one value at a time unless linkage is declared |

**Explanation:** *disassociability* is the closest existing term for DEP's principle. The NIST Privacy Framework (version 1.0) makes it concrete in its *Disassociated Processing* category, whose first subcategory (CT.DP-P1) is that data are processed to limit observability and linkability. Anchored matching is a direct way to limit linkability: a query can associate records only for the value it names.

### What DEP does not cover {#35-what-dep-does-not-cover}

**Where are the limits?** DEP is a technical measure inside the database. It does not address:

- the **lawful basis** for processing, or **consent** and its withdrawal;
- **notice** to individuals and transparency toward them;
- **what is collected** in the first place, and **how long** it is kept;
- **data protection impact assessments** and organizational governance;
- **international transfers**;
- data that has **already been disclosed** and left the database.

The mappings in this chapter describe technical contributions, not compliance. Any mapping to specific legal duties should be validated by privacy counsel.

**Key points**

- Privacy as Code checks policy around the database; DEP enforces it inside the database, on every query.
- DEP applies most privacy-by-design principles directly, and the lifecycle and user-centric principles only in part.
- For the GDPR, DEP's strongest contribution is to Article 25(2): it limits the **extent of processing** and the **accessibility** of protected values by default.
- In privacy-engineering terms, DEP is a mechanism for **disassociability**: using values without associating them beyond what each operation needs.

---

## A practical implementation workflow

1. **Classify:** identify personal identifiers and ordinary fields that can identify people.
2. **Specify:** record the legitimate application tasks and the data each task requires.
3. **Declare:** map identifiers to PROTECTED columns and meaningful MATCH DOMAINs.
4. **Grant:** separate application roles and access paths. Set match limits for lookup roles.
5. **Review exceptions:** inspect each DISCLOSES or LINKS function, its owner, purpose and callers.
6. **Test:** check both permitted operations and attempts that must be refused.
7. **Observe:** inspect SHOW PRIVACY MATRIX and SHOW AUDIT; review access and function changes.
8. **Operate:** configure TLS, separate key custody, backups, retention and incident procedures.

[Build a small application with these controls →](../guides/first-app.md)

## What remains outside DEP

DEP does not decide the lawful basis for processing, collect consent, issue notices, set retention periods, perform impact assessments or govern international transfers. Rights of access, rectification, erasure and portability need complete workflows across all systems, including backups and recipients.

**Protected identifiers are not anonymous data.** A successful lookup reveals that a record exists; ordinary names, dates and join keys may identify someone. Encrypted or pseudonymised data may still be personal data. Aggregates are not automatically anonymous, particularly for small groups.

**The server is trusted.** DEP does not protect against an administrator who can read server memory or a compromised operating system. The simulator provides no server-side protection. The model's [guarantees and limits](08-internals.md#18-guarantees-and-limits) should be part of any evaluation.



## From concept to implementation

The [Privacy as Code introduction](../../site/content/privacy-as-code.md) is the short entry point for readers who do not need the whole book. Continue to [Design with DEP](04-design.md) for classification and the matrix, or [Build an application](05-application.md) to run the controls.
