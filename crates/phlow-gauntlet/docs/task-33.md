# task-33: checkpoint durability

**Kind:** nvim-lua · **Status:** fail (seam absent — checkpoints are in-memory only; diver-owned finding, flagged not fixed) · **Wave:** 31–35 · **Commits:** pending (wave 31-35)

## ELI5

A checkpoint is a saved-game file for a run. The supervisor writes one
at every safe step boundary; if the whole supervisor is killed
(SIGKILL — not a polite shutdown, the process just dies), a brand-new
supervisor starts up, loads the last checkpoint, and finishes the run.
The tricky parts: if the kill lands *while* a checkpoint is being
written, the new supervisor must use the last *complete* checkpoint
(never a half-written one — that's why durable systems write to a temp
file and rename it); and if checkpoints from an older version have a
different shape, restore must reject them with a clear typed error, not
resume from corrupt state.

## What this task attempts

- **Goal:** drive diver's real harness run persistence
  (`ai.harness.store`: checkpoint write/read) through the design's
  scenarios — checkpoint at step boundaries, supervisor dies, new
  supervisor restores and completes; adversarial: kill during a
  checkpoint write (atomic write or write-then-rename asserted);
  adversarial: checkpoint schema changed between versions (typed
  rejection).
- **Mechanism:** the `task_33.lua` driver in headless Neovim against the
  REAL diver Lua tree — real `store.checkpoint` / `get_checkpoint` /
  `save_run` with real run tables. No mocks; the "new supervisor" is a
  fresh `store.new()`, exactly what a new supervisor process
  constructs.
- **Success criterion:** no step executes twice after restore unless
  idempotent (documented per step); at-most-once effects hold across
  the kill.
- **Non-goals:** fixing diver. Diver-owned findings stay flagged, never
  fixed on gauntlet authority.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly. The
checkpoint API works; durability does not exist:

- `checkpoint_roundtrip_in_memory` (V): `save_run` + `checkpoint` at
  `step-1` and `step-2`, then `get_checkpoint` for both — both restore
  with matching state. The API half of the seam exists.
- `post_mortem_store_is_empty` (V): a fresh `store.new()` — the
  post-mortem view, what a new supervisor process starts with — returns
  nil for `get_checkpoint('run-33', 'step-2')`. SIGKILL takes the
  checkpoints with the supervisor: there is no durable checkpoint to
  restore, so the run cannot be completed from a checkpoint.
- `checkpoint_writes_no_disk_artifact` (A): after checkpointing twice,
  `GAUNTLET_WORK_DIR` holds 0 files. Kill-during-write atomicity is
  vacuous — an in-memory table assignment cannot tear — but durability
  is zero: there is no write to be atomic *about*, no write-then-rename
  to assert.
- `checkpoint_carries_no_schema_version` (A): checkpoint records are
  `{ label, at_ns, state }` — no schema version field —
  `get_checkpoint` performs no schema validation, so a schema-changed
  checkpoint could not be rejected with a typed error.

Two structural facts from the source scan, cited in the evidence:
`store.lua`'s header documents "Phase 1 store is in-memory with deep
copies at the boundary … SQLite backing is Phase 5 work", and
`supervisor.lua` accepts the store as a collaborator
(`supervisor.new({ store = ... })`) but never calls `checkpoint` —
the only `store` reference in the supervisor is the constructor
assignment. So even the in-memory checkpoints are never written by
the supervisor itself.

## The fix — what changed and why

No product fix was made — diver-owned, flagged not fixed. The
gauntlet-side work was an honest probe:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_33.lua` (new) —
  drives the real `ai.harness.store` through checkpoint/write/read,
  the post-mortem fresh-store check, the empty-scratch-dir check, and
  the checkpoint-shape inspection; fail-closed (`where = "recon"` if
  checkpoints ever become durable).
- **Changed:** `crates/phlow-gauntlet/src/tasks/task_33.rs` (new) —
  thin `nvim-lua` shim, mirroring `task_30.rs`.
- **Why:** a durability claim needs a durable write. The probe proves
  the writes never leave the process — so the honest verdict is
  seam-absent, not a faked pass on the in-memory round-trip.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/store.lua`
  (`M.checkpoint`, `M.get_checkpoint`, `M.save_run`; Phase-1 header),
  `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  (store accepted, never used), `~/workspace/repos/diver/lua/ai/harness/init.lua`
  (wires `store_mod.new()` into the supervisor).
- **Validation agents:** the 2 validation tests
  (`probe_reports_seam_absence`, `probe_exercises_the_real_checkpoint_api`)
  pin the `fail`-at-`seam` verdict and prove the real API was
  exercised before concluding.
- **Adversarial agents:** the 2 adversarial tests
  (`verdict_is_a_finding_not_a_probe_crash`,
  `checkpoint_writes_no_disk_artifact`) rule out a crashing probe
  masquerading as the finding and pin the zero-disk-artifact /
  no-schema-version evidence.

## Full technical depth

`M.checkpoint(store, run_id, label)` deep-copies the run into
`store.checkpoints[run_id][label] = { label, at_ns, state }`
(`at_ns` from `types.now_ns()`, libuv hrtime headless). `M.save_run`
deep-copies into `store.runs[run.id]`, dropping the `handle` (live
process handles are never persisted — correct). `M.get_checkpoint`
deep-copies back out. `M.mark_interrupted` marks orphaned non-terminal
runs `interrupted` on "startup-equivalent recovery" — explicitly
*never* replaying a mutating call. All of this is sound in-memory
bookkeeping; none of it survives the process.

The supervisor's constructor takes `opts.store` and stashes it, but no
code path in `supervisor.lua` calls `store.checkpoint`,
`store.save_run`, or `store.get_checkpoint` — checkpointing at step
boundaries is not wired up at all. So the design's default scenario
fails twice over: even if the store were durable, nothing writes
checkpoints during a run.

What durability would need (banked for Matt, not implemented here):
a durable backend (the module says SQLite is Phase 5), atomic
checkpoint writes (temp-file + rename, fsync), checkpoint calls at the
supervisor's step boundaries, a schema version on the checkpoint
record with typed rejection on mismatch, and a restore path in the new
supervisor that replays at-most-once from the last complete
checkpoint. Until then, a SIGKILLed supervisor loses the run's
in-flight state entirely — `mark_interrupted` can only mark runs the
*new* process never saw, which is to say, none of them.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/store.lua`
  (checkpoint API; "Phase 1 store is in-memory … SQLite backing is
  Phase 5 work").
- Primary: `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  (store accepted as collaborator, never used).
- Primary: `~/workspace/repos/diver/lua/ai/harness/init.lua`
  (wires `store_mod.new()` into `supervisor.new`).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_33.lua` (real store,
  headless Neovim).
- Shim: `crates/phlow-gauntlet/src/tasks/task_33.rs`.
- Tests: `crates/phlow-gauntlet/tests/task_33.rs` (2V/2A).
