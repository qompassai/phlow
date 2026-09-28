# task-189: poisoned session index

**Kind:** rust · **Status:** pass · **Wave:** 30 · **Commits:** pending (wave 30)

## ELI5

Session files come from the outside world — a user could paste
anything into a session title, including text that looks like a
database attack (`'; DROP TABLE sessions; --`) or a 20-megabyte
wall of text meant to blow up the index. This task attacks the
engine twice: first with SQL metacharacters in every text field,
proving the payload is stored as harmless data and the table
survives; second with a 20 MiB field, proving the engine cuts it
down to a fixed maximum (1 MiB) and marks the row as truncated
instead of choking.

## What this task attempts

- **Goal:** prove hostile session content is stored literally and
  cannot alter the index; prove oversized fields truncate safely.
- **Mechanism:** `session_find.rs` (`scan_and_index`, `SqlLog`
  statement-template audit, `MAX_FIELD_BYTES`, `truncated` flag);
  driver `src/tasks/task_189.rs`.
- **Success criterion:** the SQL payload round-trips byte-identical
  through write→read, the table still has exactly 1 row, the audit
  shows exactly 1 parameterized write template; a 20 MiB field
  stores exactly 1,048,576 bytes with `truncated = 1`.
- **Non-goals:** query-time injection (task 191), corrupt database
  files (task 190).

## What happened

Pass on the first executable gate run (the wave's shared compile blockers were fixed before any test executed). Integration tests 2/2:
`sql_metacharacters_stored_literally` (payload stored literally in
title and transcript, row count 1, exactly 1 write template) and
`huge_field_truncated` (20,971,520 input bytes → 1,048,576 stored,
`truncated` true). ~0.6s on primo.

## Where it went wrong

Nothing failed at the gate. The one pre-gate compile fix (E0521
lifetime) is documented in task-186.md. One design decision was
deliberated before writing the driver: whether the `SqlLog` audit
should also cover the manifest/schema statements. It does — all
statements go through the log — and the assertion deliberately
filters to `INSERT INTO sessions` templates only, because the
*claim* is about session content reaching SQL, not about the
engine's own DDL.

## The fix — what changed and why

No fix iterations were needed for this task beyond the shared
E0521 signature change. The relevant design, and why it is the way
it is:

- **Changed:** n/a — design as built.
- **Why this design:** the single write template
  (`INSERT INTO sessions (...) VALUES (?,?,?,?,?,?,?,?) ON
  CONFLICT(path) DO UPDATE ...`) takes all eight values as bound
  parameters; session content is never concatenated into SQL
  anywhere in the module. The `SqlLog` records the template string
  of every statement, so the test can *prove* the claim instead of
  asserting it: exactly one distinct template starting with
  `INSERT INTO sessions`. If a future edit added a second,
  content-carrying template, this test would fail loudly.
- **Why truncation at 1 MiB:** `MAX_FIELD_BYTES` is a named bound
  (Tiger Style: named limits, reject overflow before unbounded
  allocation). Truncation is UTF-8-safe (floor to the last char
  boundary) and sets the row's `truncated` flag, so information
  loss is visible, not silent. The test asserts the stored byte
  count equals the bound *exactly*, not just "less than input".

## Full technical depth

A1 writes one session whose title and transcript are the literal
string `'); DROP TABLE sessions; --`. `scan_and_index` parses the
JSON, binds the fields as parameters, and upserts. The driver then:
(1) counts rows — exactly 1, proving no second statement executed
(the `DROP TABLE` would have emptied the table, and a second
statement would have errored or changed the count); (2) reads the
row back through `SessionIndex::row_title` and requires
byte-equality with the payload, proving it was stored literally
rather than interpreted or mangled; (3) audits `SqlLog` for
distinct statement templates beginning with
`INSERT INTO sessions` — exactly 1.

A2 writes one session with a 20 MiB (`20 * 1_048_576` byte)
transcript. `parse_session` truncates the field to
`MAX_FIELD_BYTES` at a UTF-8 boundary and sets `truncated`. The
driver asserts input bytes (20,971,520), stored bytes
(1,048,576 = bound), and the flag — plus a rescan, to prove the
truncated row is stable and not re-read as "changed".

Both cases are adversarial in the sheepdog sense: A1 is the exact
shape a real injection attempt would take, and A2 is the exact
shape of a resource-exhaustion attempt.

## Sources

- Primary: `crates/phlow-gauntlet/src/session_find.rs`
  (`scan_and_index`, `parse_session`, `MAX_FIELD_BYTES`,
  `SqlLog`, `SessionIndex::row_title`);
  `crates/phlow-gauntlet/src/tasks/task_189.rs`;
  `crates/phlow-gauntlet/tests/task_189.rs`.
- Secondary: rusqlite `params_from_iter` docs (bound-parameter
  API); Tiger Style Rust on named bounds.
