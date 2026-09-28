# task-128: retry one-shot lifecycle

**Kind:** nvim-lua · **Status:** partial (open, diver gap: no retry one-shot at `retry_at_ns`) · **Wave:** 126–130 · **Commits:** pending (wave 126-130)

## ELI5

When a job fails, the harness can try again — but not immediately. It
sets a wait time: "try again in 5 seconds." The Phase-2 plan sets an
alarm for that moment: when it rings, the supervisor wakes up and
re-launches the job exactly once. Today there is no alarm — the retry is
just a note ("try at 3:00:05"), and the retry only happens if someone
calls `tick()` after that time. This task proves the note-based retry
works (exactly once, never early), that the attempt limit is enforced,
and that cancelling a job mid-wait really kills it — and records the
missing alarm precisely.

## What this task attempts

- **Goal:** characterize retry today (timestamp `retry_at_ns` set by
  `retry_run`, promoted only by `tick()`; ceiling refusal; cancel during
  `retry_wait` is sticky) and record precisely that no one-shot is
  scheduled at `retry_at_ns` — or record that the surface shipped.
- **Mechanism:** diver's `ai.harness` — `supervisor.retry_run`
  (`retry_at_ns`, attempt counter), `M.tick` retry promotion (line 500),
  `M.cancel`, the fake-adapter `start` counter — via the driver
  `crates/phlow-gauntlet/lua/gauntlet/task_128.lua`.
- **Success criterion:** a retry at T promotes only via an explicit
  `tick()` at/after T — re-queued and relaunched exactly once, attempt
  incremented exactly once; past-ceiling retries refuse before any
  mutation; a run cancelled during `retry_wait` never relaunches even
  ticked far past `retry_at_ns` (the adapter `start` is never invoked
  again); the gap record pins the one-shot with file/line evidence.
- **Non-goals:** the wake mechanism itself (task-126), deadline timers
  (task-127), approval timers (task-129). This task is the retry timer
  lifecycle only.

## What happened

Partial — three scenarios pass today; one records the gap:

- `retry-promotes-via-tick` passes: after `retry_run`, a pre-due `tick()`
  leaves the run in `retry_wait`; ticking with `now_ns` at `retry_at_ns`
  re-queues and relaunches exactly once (adapter `start` count 1→2),
  attempt incremented exactly once. A second run then probes the clock
  jump: `retry_at_ns` backdated 60s into the past (simulated — labelled
  as such) promotes immediately on the next `tick()`, exactly once
  (adapter `start` +1, attempt unchanged), and a further tick relaunches
  nothing — no spin, no double launch.
- `retry-ceiling-refusal` passes: pushing attempts past
  `RETRY_ATTEMPTS_MAX` returns `false` with
  `'retry attempt ceiling exceeded'` — no mutation, no timer.
- `cancel-during-retry-wait` passes: cancelling during `retry_wait`,
  then ticking far past `retry_at_ns`, never relaunches the run
  (adapter `start` count stays 1). The stale timestamp is inert.
- `retry-one-shot-absent` fails with `where = "retry-one-shot-absent"`:
  `M.retry_run` (supervisor.lua:181–215) sets `retry_at_ns` and nothing
  else; promotion lives in the `tick()` sweep (line 500). The record
  pins the acceptance: one-shot at `retry_at_ns` → `wake(sup, now,
  'retry')` → re-queue + relaunch, attempt incremented exactly once,
  and cancel during `retry_wait` cancels the timer.

## The fix — what changed and why

No fix — retry timers are diver-owned (Phase-2 Decision 3), and diver
findings are never fixed under gauntlet authority; the tick-based retry
already works correctly and needed no change. The gauntlet-side work was
getting the evidence right:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_128.lua` (new) —
  four scenarios: the tick-promotion characterization, the ceiling
  refusal, the cancel-during-wait race, and the gap record.
- **Why:** this is the only task covering the retry timer lifecycle —
  the exactly-once relaunch, the never-early promotion, and the race
  between cancel and the timer. Retry is where double-launch bugs live;
  the one-shot must be exactly-once and must die on cancel.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  `M.retry_run` (lines 181–215 — timestamp only), `M.tick` retry
  promotion (line 500).
- **Validation agents:** the 2 validation tests (`retry_one_shot_gap`,
  `retry_promotes_via_tick_pass`) assert the gap record and pin the
  promotion contract.
- **Adversarial agents:** the 2 adversarial tests
  (`retry_ceiling_refusal_pass`, `cancel_during_retry_wait_pass`) push
  the ceiling and the cancel race.

## Full technical depth

`M.retry_run` guards `attempts > RETRY_ATTEMPTS_MAX`, refuses with
`'retry attempt ceiling exceeded'` (no mutation), then sets
`run.state='retry_wait'`, `run.retry_at_ns = now_ns + backoff_ms *
1e6`, and appends `run.state_changed`; no timer is scheduled. The
`tick()` sweep (line 500) relaunches when `now_ns >= retry_at_ns`:
`run.state='queued'`, attempt incremented exactly once, `relaunch()`
calls the adapter's `start` again. `M.cancel` on a `retry_wait` run sets
`cancelled` (the only non-terminal state cancel accepts besides
`running`/`queued`/`waiting_approval`, at line 232); subsequent ticks
skip terminal runs (the `is_terminal` guard at line 476) so the stale
`retry_at_ns` never fires — no phantom relaunch, verified by the
adapter's `start` call count. Phase 2 replaces the timestamp promotion
with a one-shot: `retry_run` schedules it at `retry_at_ns`, the fire
calls `wake(sup, now, 'retry')` which re-queues and relaunches exactly
once, and cancel during `retry_wait` must cancel the timer — otherwise
the one-shot fires into a cancelled run and must be a no-op.

Phase-2 acceptance (banked, diver-owned): a one-shot is scheduled at
`retry_at_ns` → `wake(sup, now, 'retry')` → re-queue and relaunch; the
attempt increments exactly once; no early firing (pre-due time causes
nothing); cancel during `retry_wait` cancels the timer (no fire after
cancel); the ceiling refusal still happens before any timer is
scheduled.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  `M.retry_run` (lines 181–215), `M.tick` retry promotion (line 500),
  `M.cancel` (line 232).
- Spec: `~/workspace/your_files/diver-harness-phase2-spec.md`
  "Supervision without polling" — one-shot timers (Decision 3).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_128.lua` (four
  scenarios; three pass, one `where = "retry-one-shot-absent"`).
- Tests: `crates/phlow-gauntlet/tests/task_128.rs` (2V/2A).
- Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no
  Phase-2 branch exists).
