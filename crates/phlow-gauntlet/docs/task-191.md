# task-191: query literal handling

**Kind:** rust · **Status:** fail → fixed · **Wave:** 30 · **Commits:** pending (wave 30)

## ELI5

Task 189 proved *session content* can't attack the database. This
task proves the *search box* can't either. When the user types `%`,
SQL's `LIKE` would normally treat that as "match everything" — so
the engine escapes it to mean a literal percent sign before the
query ever reaches SQLite. And when someone types something shaped
like an injection attack (`" OR "1"="1`), the engine must treat it
as boring search text: zero matches, table untouched. Two
adversarial cases prove the query is data, never code.

## What this task attempts

- **Goal:** prove user queries are matched literally — `%` matches
  only literal `%`, and injection-shaped input matches nothing and
  changes nothing.
- **Mechanism:** `session_find.rs` (`escape_like`, `search_sql`,
  `query_terms`, query log); driver `src/tasks/task_191.rs`.
- **Success criterion:** query `%` → exactly 1 hit (the session with
  a literal `%`), not the whole table; query `" OR "1"="1` → 0
  hits, row count unchanged, and the query log shows the escaped
  pattern form.
- **Non-goals:** session-content injection (task 189), ranking
  (task 187).

## What happened

Two failures before the fix, then pass. Final gate:
`cargo test -p phlow-gauntlet --test task_191` → 2/2 pass
(`percent_matches_literally`, `or_injection_matches_literally`).
The query `%` matched exactly the 1 literal-`%` session out of the
corpus; the injection-shaped query matched 0 with the table
unchanged (rows before == rows after).

## Where it went wrong

Two driver bugs, both in the test scaffolding — the engine was
correct throughout:

**Failure 1 — silent verdict.**

- **Stage:** first gate run of `task_191`.
- **Symptom:** `task-191 case or_injection_matches_literally failed: `
  — with *nothing* after the colon. The test failed, and the failure
  message was empty.
- **Evidence:** test output on primo (panic at
  `tests/task_191.rs:17`).
- **Root cause:** `case_or_injection_matches_literally` set
  `report.passed = failures.is_empty()` but never assigned
  `report.failures = failures`. The verdict said "fail" while the
  reason stayed in a local variable that was dropped. The sibling
  case (A1) had the assignment; A2 lost it, most likely in an edit.

**Failure 2 — over-strict log assertion.**

- **Stage:** second gate run, after fixing failure 1.
- **Symptom:** `task-191 case or_injection_matches_literally
  failed: query log does not show the escaped query form:
  terms=["\"", "or", "\"1\"=\"1"]
  like_patterns=["%\"%", "%or%", "%\"1\"=\"1%"] escape='\'`
- **Evidence:** the panic message above — note the log *does* show
  the escaped patterns; the assertion was wrong, not the engine.
- **Root cause:** the driver asserted
  `log.contains("\" OR \"1\"=\"1")` — the raw contiguous query —
  but the engine tokenizes on whitespace before escaping, so the
  contiguous string can never appear in the log. Then, after
  switching to the escaped fragment `%"1"="1%`, the assertion still
  failed because the log records the pattern list with Rust Debug
  formatting, which escapes the quotes (`%\"1\"=\"1%`).

(The pre-gate E0521 lifetime fix is documented in task-186.md.)

## The fix — what changed and why

- **Changed:** `src/tasks/task_191.rs`:
  (1) added the missing `report.failures = failures;` in A2;
  (2) replaced the raw-query log assertion with
  `format!("{:?}", format!("%{}%", escape_like("\"1\"=\"1")))` —
  the Debug-escaped form of the escaped LIKE pattern, which is
  exactly what the log line contains.
- **Commit:** pending (wave 30).
- **Why:** (1) a failing verdict with an empty reason is a
  reporting bug that hides real causes — the evidence concat in the
  driver already cloned `failures` into evidence, but the
  `CaseReport.failures` field is what both the test and the gauntlet
  runner surface. (2) The test must assert what the code actually
  promises: the log records terms and escaped patterns. Asserting
  the Debug representation is honest about the log's format; an
  alternative — changing the log format to avoid Debug escaping —
  would have churned the engine to satisfy a test.
- **Source:** `session_find.rs` `query_terms` (whitespace
  tokenization) and the `query_log` recording in `search`
  (`format!("like_patterns={patterns:?}")`).
- **Validation agents:** `cargo test -p phlow-gauntlet --test task_191`
  on primo → 2/2 pass after the fix.
- **Adversarial agents:** the two cases are the adversarial half —
  `%` is the classic LIKE-wildcard probe, `" OR "1"="1` the classic
  injection shape. The engine treats both as literal text: the
  wildcard matched 1/6 sessions (not 6/6), the injection matched 0
  and left the table untouched.
- **Citations:** SQLite `LIKE` documentation (`%`, `_` wildcards,
  `ESCAPE` clause: https://www.sqlite.org/lang_expr.html#like);
  rustc `Debug` formatting of strings.

## Full technical depth

`query_terms` splits the raw query on whitespace (capped at 16
terms, 512 bytes total). `escape_like` prefixes every `\`, `%`,
`_` with a backslash. `search_sql` builds the conjunction of
`(title LIKE ? ESCAPE '\' OR transcript LIKE ? ESCAPE '\')`
clauses with one bound parameter per term — the pattern text is a
*value*, never SQL syntax. The prefilter therefore matches only
literal occurrences.

A1's corpus: 6 sessions, exactly one containing a literal `%`
("100% coverage" title). Query `%` → 1 hit; the driver asserts
`hits < total` explicitly so a wildcard leak (6/6) fails. The
query log assertion checks the escaped pattern `%\%%` appears —
the backslash-escaped form proving the wildcard was neutralized
before SQLite saw it.

A2's corpus reuses the same 6 sessions. Query `" OR "1"="1`
tokenizes to [`"`, `or`, `"1"="1`]; the conjunction requires all
three literally present in one session's title or transcript —
none are, so 0 hits. Row count before/after is identical, proving
a query never writes. The escaped pattern for the injection
fragment appears in the log in its Debug-escaped form.

Scoring note: with 0 prefilter candidates the scorer never runs —
literal handling is entirely a prefilter property, which is why
these cases need no ranking corpus.

## Sources

- Primary: `crates/phlow-gauntlet/src/session_find.rs`
  (`escape_like`, `search_sql`, `query_terms`, query log in
  `SessionIndex::search`);
  `crates/phlow-gauntlet/src/tasks/task_191.rs`;
  `crates/phlow-gauntlet/tests/task_191.rs`.
- Secondary: SQLite `LIKE` / `ESCAPE` documentation.
