# task-196: concurrent modification aborts, kill midway recovers

**Kind:** rust (adversarial) · **Status:** pass · **Wave:** 31 · **Commit:** pending (`gauntlet: wave 31 drivers, tests, docs`)

## ELI5

Two bad things can happen while a sync is running. First: someone edits a target file *after* the sync looked at it but *before* it writes — the sync would then back up and overwrite a version it never inspected. The fix is to re-check every file right before writing and abort if anything changed. Second: the machine crashes halfway through — some files new, some old, some deleted. The fix is backups plus a "work in progress" marker: the next sync sees the marker, restores everything from backup first, and only then starts over. This task tests both: a mid-sync edit must abort with zero writes, and a simulated crash must recover to the exact pre-sync state and then converge.

## What this task attempts

- **Goal:** verify (A1) a target file modified between scan and apply aborts the whole apply with `SyncError::ConcurrentModification` before any write, and (A2) a kill after the first op leaves the marker, and the rerun restores the backups and converges the target to canonical, byte-identical to a clean reference sync.
- **Mechanism:** `crates/phlow-gauntlet/src/skill_sync.rs` (`reverify_target`, `restore_backups`, `BACKUP_MARKER`, `ApplyOptions::kill_after_ops`) driven by `crates/phlow-gauntlet/src/tasks/task_196.rs`; assertions in `crates/phlow-gauntlet/tests/task_196.rs`. The crash is simulated by the test-only `kill_after_ops` hook, not a real signal.
- **Success criterion:** A1: after the target is modified post-scan, `apply` → `Err(SyncError::ConcurrentModification{..})` with an empty fs log (zero mutations) and all target files untouched. A2: kill after 1 op → `Err(SyncError::Killed)`, marker present, exactly `["a.md"]` restored on rerun; rerun applies 2 ops, removes the marker, and the target is byte-identical to a from-scratch reference sync of the same inputs.
- **Non-goals:** benign paths (193/194), hostile filenames (195), real process-crash semantics (the hook simulates the crash point deterministically).

## What happened

PASS, all 2 scenarios, on the final tree:

- **A1:** post-scan modification → `Err(SyncError::ConcurrentModification { rel: "a.md" })`; fs-access log empty (0 writes, 0 backups — the abort precedes backup_targets); `a.md`/`b.md` still the tampered/original bytes respectively; marker absent.
- **A2:** kill after 1 op → `Err(SyncError::Killed)`; marker `.sync-in-progress` present; rerun restored exactly `["a.md"]` and then applied 2 ops; marker removed; all four target files byte-identical to the reference sync outputs.

`cargo test -p phlow-gauntlet --test task_196`: `2 passed; 0 failed`.

## Where it went wrong

(Deleted — the task passed its tests on the first test run. One precision improvement was made to the shared module during the wave: see below.)

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/src/skill_sync.rs` — `reverify_target` now maps a post-scan *deletion* (`ErrorKind::NotFound` on re-read) to `SyncError::ConcurrentModification` instead of a generic `SyncError::Io`.
- **Why:** a file deleted after the scan is the same class of event as a file modified after the scan — the plan-time state no longer holds, and writing against a stale plan is exactly what the check exists to prevent. Returning `Io` would still abort before any write (fail-closed either way), but the typed error tells the operator what actually happened instead of masquerading as a filesystem failure. The signature was also collapsed to one line (rustfmt normalization). **Source:** the `expected_hash` contract documented on `PlanOp` — the hash is "the target hash seen at plan time", and a missing file trivially fails that comparison.
- **Commit:** pending (wave 31).
- **Validation agents:** the wave-31 worker — full focused suite + lib tests rerun on primo after the change, 9/9 + 64/64 green.
- **Adversarial agents:** none separately assigned; A1/A2 are the adversarial cases. Considered and deliberately not covered: a modification landing *between* reverify and write (single-threaded apply makes the window one re-hash wide; closing it fully would need filesystem-level locking, out of scope for this adaptation).

## Full technical depth

The concurrency check exploits the plan itself: Update/Remove ops carry `expected_hash`, the target hash at plan time. `reverify_target` re-hashes each named target file after validation but before `backup_targets` — ordering matters: backups must not be taken of a state the plan no longer describes, and no write may precede the check, so the abort is total (empty fs log, verified in A1). The crash protocol is marker + mirrored backups: `backup_targets` copies (never renames) every Update/Remove target into the backup dir *before* the marker is written, so the marker's presence implies the backups are complete — a crash between "last backup" and "marker write" leaves no marker and no partial state (backups without a marker are simply overwritten next run). On rerun, `restore_backups` walks the backup dir (iterative, symlink entries skipped by the `is_file` gate, the marker itself excluded) and atomically rewrites each target, logging `Restore` entries; then the marker is removed and the apply proceeds as normal — the rerun is not a special code path, it's the normal path after restoration, which is why the reference-convergence check is meaningful: the recovered target must equal a clean sync byte-for-byte, and it does. `kill_after_ops` is test-only and clearly labeled; it returns *after* the op's write and *before* marker removal, which is exactly the crash window the protocol is designed for.

## Sources

- `crates/phlow-gauntlet/src/skill_sync.rs` — `reverify_target`, `restore_backups`, `backup_targets`, `apply`, `ApplyOptions::kill_after_ops` (primary source for every mechanism claim)
- `crates/phlow-gauntlet/src/tasks/task_196.rs`, `crates/phlow-gauntlet/tests/task_196.rs`
- `~/workspace/scratch/ghostex/packages/agent-sync/src/apply.rs` @ `c91146607205ac49303d1bcfe2fd6f9a86741500` — the Ghostex concept source for backup-before-write; the marker/restore protocol here is the phlow design
- `~/workspace/gauntlet-design-tasks-151-200.md` — Wave 31 design (lines 943–1030)
