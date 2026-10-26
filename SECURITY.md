# Reporting a bypass

DEP's central claim is that protected values cannot leave the database except through declared disclosures, and that whole datasets cannot be linked through them except in declared linking functions. If you find a way around that in the design, please tell us privately first.

## What counts

Anything that lets a client do something the design says is impossible, for example:

- obtain a protected value outside a `DISCLOSES` function;
- link whole datasets through a protected value outside a `LINKS` function;
- match on protected values from the masked read path;
- use a disclosed value as a key, filter, or argument;
- learn a protected value from an error message, a log, a statistic, or any other channel;
- change a connection's access path after authentication.

Weaknesses that the design already documents as limitations (for example, that a successful match reveals that a record exists) are welcome as discussion in public issues.

## How to report

Contact the author through <https://lp.reuven.click> and put "DEP security" in the subject. Do not post details in a public issue. Include the schema, the statements, the path and role, and what you expected versus what the design allows.

You will receive an acknowledgment within a few days. Reporters are credited in the documentation and release notes unless they prefer to remain anonymous.

## Scope

Reports about the design are in scope, and so are reports about the implementations: DEP Engine, DEP CLI, Hoopeo Studio (distributed as binaries) and DEP.js (open source). DEP Engine is an experimental concept that demonstrates the model and its trust boundary (see the [terms of use](site/content/legal/terms.md)). Design-level bypasses are especially valuable while the rules can still change.
