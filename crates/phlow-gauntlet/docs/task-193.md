# task-193: skill-sync plan, apply, backup

**Kind:** rust (validation) · **Status:** pass · **Wave:** 31 · **Commit:** pending (`gauntlet: wave 31 drivers, tests, docs`)

## ELI5

Imagine you keep a master copy of your skills on one machine and copies on others. Syncing means: look at both copies, figure out exactly what differs, and copy only the differences — but never lose data. This task checks the machinery that does that: the scanner that lists files, the planner that turns "what differs" into an ordered to-do list (add this, update that, remove the other), the applier that executes the list, and the backup step that saves every file about to be replaced or deleted *before* touching it. It also checks the dry-run mode: print the whole plan and change absolutely nothing on disk.

## What this task attempts

- **Goal:** verify the Ghostex-adapted scan → plan → apply pipeline end to end: plan is exact (add/update/remove by content hash), apply executes it, every replaced/removed file is backed up first, and dry-run writes nothing.
- **Mechanism:** `crates/phlow-gauntlet/src/skill_sync.rs` (`scan`, `build_plan`, `apply`, `sync`) driven by `crates/phlow-gauntlet/src/tasks/task_193.rs`; assertions in `crates/phlow-gauntlet/tests/task_193.rs`. All fixtures live in temp dirs, never in `~/workspace/skills`.
- **Success criterion:** V1 (`plan_apply_backup`): plan is exactly `[update a.md, add b.md, remove c.md]`; apply executes 3 ops; the fs-access log shows writes `[a.md, b.md]`, remove `[c.md]`, backups `[a.md, c.md]`; backup contents are byte-identical to the pre-sync target versions; target converges to canonical. V2 (`dry_run_writes_nothing`): plan prints `["update a.md"]`, zero fs mutations, no backup dir created, target untouched.
- **Non-goals:** idempotence across runs (task 194), adversarial paths (task 195), concurrency and crash recovery (task 196).

## What happened

PASS, all 2 scenarios, on the final tree:

- **V1:** plan exact `[update a.md, add b.md, remove c.md]`; `report.applied.len() == 3`; fs log writes `[a.md, b.md]`, removes `[c.md]`, backups `[a.md, c.md]`; both backups SHA-256-matched the pre-sync target bytes; post-sync target byte-identical to canonical.
- **V2:** `plan.describe() == ["update a.md"]`; fs-access log empty (0 mutations); backup dir never created; target `a.md` still the old content.

`cargo test -p phlow-gauntlet --test task_193`: `2 passed; 0 failed`.

## Where it went wrong

Two failures during the wave, one in this task's test and one in the shared module:

1. **Stage:** `cargo test --test task_193`, V1 `plan_apply_backup`, first test run.
   **Symptom:** `task-193 case plan_apply_backup failed: backups ["/tmp/gauntlet-193-plan_apply_backup-…/backup/a.md", "…/backup/c.md"], want exactly [a.md, c.md]`.
   **Root cause:** `check_fs_log` normalized every logged path against the *target* root, but `FsAccess::Backup` paths live under the *backup* dir, so `strip_prefix` fell through to the full path. A test-side normalization bug, not a sync bug — the backups themselves were correct.
2. **Stage:** `cargo build -p phlow-gauntlet`, first build.
   **Symptom:** `error: this file contains an unclosed delimiter` at `src/skill_sync.rs:464`.
   **Root cause:** the initial write of `skill_sync.rs` was truncated in transit — the file on disk ended mid-function (`restore_backups`, at `.map`) with a literal truncation marker. The exact transport mechanism was not verified; what was verified is the file state (463 lines, 3 unclosed braces) before the rewrite.

## The fix — what changed and why

- **Fix 1 (test failure):** `crates/phlow-gauntlet/src/tasks/task_193.rs` — `check_fs_log` now takes both `target` and `backup` roots and strips each log kind against its own root (writes/removes vs target, backups vs backup). Why: the log records paths from two different roots; a single normalization root cannot be correct for both. Alternative (strip longest matching prefix) rejected as cleverer than the explicit two-root version. **Source:** the `FsAccess::Backup` contract documented in `skill_sync.rs` (backup paths mirror relative paths under the backup dir).
- **Fix 2 (build failure):** `crates/phlow-gauntlet/src/skill_sync.rs` — rewrote the complete file from the authored content (643 lines), then verified: brace/paren balance zero, tail is the `sync` function, header bytes intact, no truncation marker. Why a full rewrite instead of appending the tail: the truncation point was mid-expression, and a whole-file rewrite from the known-good authored text is the only way to be certain nothing else was silently dropped. **Validation:** `cargo build` green after the rewrite; the four compile errors found next (see below) were in the task drivers, proving the module itself was complete.
- **Fix 3 (build failures, task drivers):** the shared `sync()` returns `Result<ApplyReport, SyncError>`, not a `(plan, report)` tuple — three drivers destructured a tuple that doesn't exist (`E0308`). Fixed each call site to build the plan separately (`scan` + `build_plan`) where the test asserts on it, and use `report.applied`/`report.backed_up` where it asserts on the outcome. `task_195.rs` additionally passed `case: &str` into `CaseReport::pass`, which requires `&'static str` (`E0521`) — the helper now takes `&'static str`, which the `CASES[i]` call sites already satisfy — and referenced a `plan` binding that no longer existed after the tuple fix (`E0425`). **Source:** the `sync` signature in `skill_sync.rs` itself.
- **Validation agents:** the wave-31 worker (build + focused tests + lib tests on primo, 2026-09-28). **Adversarial agents:** none assigned beyond the task's own adversarial siblings (195/196); the backup-before-write ordering is red-teamed by task 196's kill test.

## Full technical depth

The pipeline is scan → plan → apply, with the plan as the auditable seam. `scan` walks iteratively (no recursion, `MAX_SCAN_FILES = 10_000`, `MAX_FILE_BYTES = 8 MiB`) and records SHA-256 per file; symlinks are indexed but never followed. `build_plan` diffs the two `BTreeMap`s by hash — mtime is deliberately ignored, because mtimes lie across filesystems and copies — producing `PlanOp`s sorted by relative path, with the plan-time target hash attached to Update/Remove ops (`expected_hash`). `apply` then: (1) crash recovery if the marker is present, (2) validate every op path with `contained_path` (lexical component check, no canonicalization of not-yet-existing paths), (3) dry-run short-circuit, (4) re-hash targets and compare against `expected_hash` (the task-196 concurrency check), (5) back up every Update/Remove target by copying (not renaming) into the mirrored backup dir, (6) write the `.sync-in-progress` marker, (7) execute ops in order via atomic temp-file + rename at mode `0o644`, (8) remove the marker. The fs-access log (`Write`/`Mkdir`/`Remove`/`Backup`/`Restore`) records every mutation in order, which is what V1 asserts on. Backups cover *removed* files too — `c.md` is in the backup set, which is the point of backup-before-write: a remove is the most destructive op and the one most likely to be regretted.

## Sources

- `crates/phlow-gauntlet/src/skill_sync.rs` — the adapted pipeline (primary source for every mechanism claim above)
- `crates/phlow-gauntlet/src/tasks/task_193.rs`, `crates/phlow-gauntlet/tests/task_193.rs`
- `~/workspace/scratch/ghostex/packages/agent-sync/src/{lib,scan,plan,apply}.rs` @ `c91146607205ac49303d1bcfe2fd6f9a86741500` — the Ghostex concept source (scan → plan → apply, backup-before-write); the phlow implementation is independent, not a port
- `~/workspace/ghostex-recon/adaptation-map.md` — adaptation doctrine
- `~/workspace/gauntlet-design-tasks-151-200.md` — Wave 31 design (lines 943–1030)
- https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md — no recursion, named bounds, ≤70-line functions
