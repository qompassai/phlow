# task-138: restart recovery without rescan

**Kind:** rust · **Status:** pass · **Wave:** 24

## ELI5

A bug-bounty cycle can be killed halfway — power cut, crash, operator
stop. When it restarts, it must not redo the tests that already
finished; that would waste the program's goodwill and the rate budget.
This task checks the restart logic: after 3 of 10 targets finish and
the cycle dies, the restart probes exactly the other 7. If the saved
record of what finished is corrupted, the restart refuses to guess and
probes nothing until a human looks. If a finished test's result file
went missing, that one test is redone exactly once. And if the new
cycle brings a new list of targets, the finished ones that are still
listed are skipped, dropped ones are never retested, and new ones are
queued.

## What this task attempts

- **Goal:** a killed cycle resumes and probes only what never completed; completed targets are never rescanned.
- **Mechanism:** a driver-local file-backed ledger (`FileLedger` in `src/tasks/task_138.rs`, MOCK): append-only tab-separated run log plus `blobs/<run_id>` result files; resume replays the log and plans against the scaffold's `RunLedger::pending_target_ids`.
- **Success criterion:** V1 exactly 7 resumed probes, ledger ends with 10 `Finished`, driver plan agrees with the scaffold; V2 corrupt log → typed `LedgerError::Corrupt`, 0 probes; A1 lost blob → exactly-once re-probe of that target; A2 new scope version → probed set is exactly the new targets minus the already-finished.
- **Non-goals:** concurrency (task 136), dedup absorbing the re-probe duplicate (task 139's job).

## What happened

Gating caught two driver bugs before the final green run: the
scaffold cross-check compared against `pending_target_ids` over all
scope targets (the scaffold only knows recorded targets — fixed to
the recorded-target projection), and the corruption fixture used
`fs::write`, truncating the valid lines instead of appending the
garbage line after them. Fixed; all four cases pass. V1: cycle 1 probes t01–t03
(`Running` → `Finished` lines + blobs), then "dies"; resume plans 7
probes, skips 3 finished; the plan is cross-checked against the
scaffold's `pending_target_ids` over the recorded-target projection
(the scaffold only knows recorded targets, and counts a target
terminal only when every recorded run is terminal, so the comparison
uses the latest run per target — the same last-wins projection
`plan_resume` uses) and agrees exactly; final ledger
covers 10 targets, t01–t03 still have exactly 2 lines each (never
rescanned). V2: a garbage line planted after one valid probe → resume
`load()` returns `LedgerError::Corrupt { line_no: 3, ... }` and 0
probes run — fail closed, no silent full rescan. A1: t02's blob
deleted before resume → t02 re-probed exactly once (2 run records, 2
distinct run ids, fresh blob written); t01/t03 untouched. A2: scope v2
= t03–t12 → resume probes exactly t04–t12 (9), skips t03, never
re-probes dropped t01/t02.

## Full technical depth

The log format is strict by design: 4 tab-separated fields
(`run_id`, `target_id`, state, reason), ids non-empty and separator-free,
state token from the closed 5-variant set, lines capped at 4096 bytes,
runs capped at 10,000. `load()` rejects any deviation as
`LedgerError::Corrupt` — it never skips a bad line, because a skipped
line is a guess about what finished, and guessing is how rescans (or
missed targets) happen. The one deliberate exception to the scaffold's
"Finished/failed/cancelled are never re-probed" rule: a `Finished` run
whose blob is missing is re-probed once. Rationale: the scaffold's
terminal rule assumes the completion artifact exists; a hollow
completion (no result blob) is at-least-once territory, and the
duplicate it produces is exactly what task 139's fingerprint dedup
absorbs. `Failed`/`Cancelled` stay terminal on resume (no retry policy
smuggled in). New-scope intersection falls out of the plan loop: it
only visits targets in the current scope snapshot, so dropped ids are
unreachable by construction.

## Sources

- Primary: `crates/phlow-gauntlet/src/bounty/store.rs` (`RunLedger::pending_target_ids`); the write-ahead-log / checkpoint-restart literature (e.g. sqlite.org/wal.html — atomic commit via append-only log, replay on recovery); the design doc's "fail closed, never guess" rule for the corruption arm.
