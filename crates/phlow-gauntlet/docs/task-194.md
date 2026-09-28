# task-194: double sync is a no-op, mtime-only touch is a no-op

**Kind:** rust (validation) · **Status:** pass · **Wave:** 31 · **Commit:** pending (`gauntlet: wave 31 drivers, tests, docs`)

## ELI5

Syncing twice in a row should do nothing the second time — everything already matches, so the plan should be empty and no files should be touched. Likewise, if someone merely *touches* a file (updates its timestamp without changing what's inside), the sync must notice that the content is identical and leave it alone. This task checks both: the second sync plans zero operations, and a timestamp-only change is invisible to the hash-based comparison.

## What this task attempts

- **Goal:** verify sync idempotence and mtime-independence: a second full sync over an already-synced pair is a no-op; touching a file (mtime change, same bytes) produces an empty plan and rewrites nothing.
- **Mechanism:** `crates/phlow-gauntlet/src/skill_sync.rs` (`sync`, `scan`, `build_plan`) driven by `crates/phlow-gauntlet/src/tasks/task_194.rs`; assertions in `crates/phlow-gauntlet/tests/task_194.rs`.
- **Success criterion:** V1 (`double_sync_noop`): second sync applies 0 ops, backs up nothing, performs 0 fs mutations. V2 (`touch_without_change_noop`): after touching both target files, `build_plan` is empty, and a full sync applies 0 ops with an empty fs log.
- **Non-goals:** plan/apply/backup correctness (task 193), adversarial paths (195), concurrency (196).

## What happened

PASS, all 2 scenarios, on the final tree:

- **V1:** first sync applied 2 ops (2 adds, 0 backups — adds need no backup); second sync: `report.applied` empty, `report.backed_up` empty, fs-access log empty (0 mutations).
- **V2:** after touching both files, `build_plan` returned an empty plan; the follow-up full sync applied 0 ops and logged 0 mutations.

`cargo test -p phlow-gauntlet --test task_194`: `2 passed; 0 failed`.

## Where it went wrong

(Deleted — the task passed its tests on the first test run. Build iterations were shared with the wave; see task-193's doc, fix 3: the `(plan, report)` tuple destructure of `sync()`'s `ApplyReport` return, `E0308`.)

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_194.rs` — the `synced_pair` helper asserts on `report.applied.len()`/`report.backed_up` instead of destructuring a non-existent tuple (same wave-wide `E0308` fix as task-193). No behavioral change: the first sync still applies exactly the 2 adds.
- **Why:** `sync()` returns a single `ApplyReport`; the plan is an internal intermediate. Tests that need the plan build it explicitly via `scan` + `build_plan` (as V2 does); tests that need the outcome read the report. **Source:** the `sync` signature in `skill_sync.rs`.
- **Validation agents:** the wave-31 worker (build + focused tests + lib tests on primo, 2026-09-28).
- **New convention (if any):** none — the report-vs-plan split follows the module's existing API.

## Full technical depth

Idempotence falls out of hash comparison rather than being special-cased. After the first sync, target bytes equal canonical bytes, so the second `scan` produces identical hashes, `build_plan` emits zero ops, and `apply` walks an empty op list: no backups (nothing to replace), no marker write… — actually the marker *is* written and removed even for an empty plan (steps 6–8 run unconditionally), which is harmless and keeps the crash-recovery invariant uniform. V2 is the sharper test of the design choice: `touch` updates mtime but not bytes, and because `build_plan` compares only SHA-256, the plan is empty — no rewrite, no backup, no log entries. A sync that compared mtimes would have rewritten both files here, churning backups and wearing the "backup-before-write" path for no reason. The `NoopCheck` helper struct (mtime pairs before/after) keeps the two assertions' shared shape in one place instead of an 8-argument function.

## Sources

- `crates/phlow-gauntlet/src/skill_sync.rs` — `build_plan` (hash-only diff), `apply` (empty-plan path)
- `crates/phlow-gauntlet/src/tasks/task_194.rs`, `crates/phlow-gauntlet/tests/task_194.rs`
- `~/workspace/scratch/ghostex/packages/agent-sync/src/plan.rs` @ `c91146607205ac49303d1bcfe2fd6f9a86741500` — the Ghostex concept source for plan/apply separation
- `~/workspace/gauntlet-design-tasks-151-200.md` — Wave 31 design (lines 943–1030)
