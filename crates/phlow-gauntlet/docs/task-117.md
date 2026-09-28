# task-117: A2A completion integrity

**Kind:** nvim-lua · **Status:** fail (open, diver defect) · **Wave:** 116–120 · **Commits:** pending (wave 116-120)

## ELI5

When the remote AI finishes your job, it calls back with a status report
— "done", "failed", "cancelled". But the harness's callback is listening
for the report in the wrong shape: it expects two envelopes
(`result, task_err`) while the real code sends one (`task`). The second
envelope is therefore always empty, and the harness reads "empty" as
"success". So a job that crashed remotely gets written down as completed.
"Correct" means: read the status from the report itself (`task.state`) —
done→completed, failed→failed, cancelled→cancelled — and never call an
unknown status "completed".

## What this task attempts

- **Goal:** prove the A2A adapter derives the recorded outcome from
  `task.state`.
- **Mechanism:** the REAL `adapters/a2a.lua` with a stubbed
  `ai.a2a.tasks` transport (package.preload) — the driver captures the
  submit's `on_done` and invokes it with fabricated task tables in each
  terminal state — via `crates/phlow-gauntlet/lua/gauntlet/task_117.lua`.
- **Success criterion:** completed→completed, canceled→cancelled,
  failed/rejected→failed, anything else→failed (never completed); a double
  `on_done` stays a clean no-op (exactly one `run.finished`).
- **Non-goals:** changing the A2A transport or the wire protocol. The
  stub only stands in for `ai.a2a.tasks.submit` so the driver controls what
  the callback receives; the adapter code under test is byte-identical to
  diver.

## What happened

Fail, open diver defect (Fix 2 absent) — on the first attempt. The
contract mismatch:

- `ai/a2a/tasks.lua` line 43 declares `on_done? fun(task: A2aTask)` —
  one argument; the call site passes `on_done(task)`.
- `adapters/a2a.lua` line 63 declares
  `on_done = function(result, task_err)` — `task_err` is therefore always
  nil, and the recorded outcome is always `'completed'`, regardless of
  `task.state`.

Three of four scenarios report `fail` with `where = "fix-2-absent"`:
`default` (failed→recorded completed), `rejected-canceled`
(rejected→completed, canceled→completed), `garbage-nil-double`
(bogus-state→completed, nil-state→completed; the double-invoke half
already holds — exactly one `run.finished`). The fourth,
`completed-maps-completed`, passes — correctly *by accident*, since
`task_err` is nil for every invocation; the driver evidence says so
explicitly.

## The fix — what changed and why

No fix — this is a documented diver finding (Phase-2 Fix 2), and diver
findings are never fixed under gauntlet authority. The "fix" for the
gauntlet side was getting the evidence right:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_117.lua` (new) —
  four scenarios driving the real adapter with fabricated terminal tasks.
- **Why:** the defect is the arity mismatch, not the transport; stubbing
  only `ai.a2a.tasks` keeps the adapter under test real while making the
  callback's input fully controlled.
- **Source:** `~/workspace/repos/diver/lua/ai/a2a/tasks.lua` line 43
  (real contract: one argument),
  `~/workspace/repos/diver/lua/ai/harness/adapters/a2a.lua` lines 63-68
  (`task_err` always nil → outcome always `'completed'`).
- **Validation agents:** the 2 validation tests
  (`failed_task_misreported_as_completed`,
  `completed_maps_completed_by_accident`) assert the misreporting and pin
  the one accidentally-correct mapping so a regression there is caught.
- **Adversarial agents:** the 2 adversarial tests
  (`rejected_and_canceled_misreported_as_completed`,
  `garbage_states_fail_closed_and_double_invoke_idempotent`) demand
  fail-closed semantics for unknown states and pin the idempotent-finish
  behavior that already holds.

## Full technical depth

The adapter's `on_done` appends `model.completed` with
`outcome = task_err == nil and 'completed' or 'failed'`. Because the real
call site invokes `on_done(task)`, `task_err` binds nil on every call:
the payload outcome is `'completed'` and `result` is the whole task table.
`supervisor.tick` drains `model.completed` events and finishes the run
with the payload outcome — so a remotely failed task lands the run in
`completed` with no error recorded. There is no other consumer of
`task.state` anywhere on this path.

Phase-2 acceptance (banked, diver-owned): derive the outcome from
`task.state` — completed→completed, canceled→cancelled,
failed/rejected→failed, anything else (garbage, nil)→failed, never
completed; a duplicate `on_done` for the same task is a clean no-op
(exactly one `run.finished`, no corruption).

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/a2a/tasks.lua` line 43
  (`on_done? fun(task: A2aTask)` — one argument).
- Primary: `~/workspace/repos/diver/lua/ai/a2a/tasks.lua` ~lines 166-170
  (call site passes `on_done(task)`).
- Primary: `~/workspace/repos/diver/lua/ai/harness/adapters/a2a.lua` lines
  63-68 (`task_err` always nil → outcome always `'completed'`).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_117.lua` (four
  scenarios; three `where = "fix-2-absent"`, one accidental pass).
- Tests: `crates/phlow-gauntlet/tests/task_117.rs` (2V/2A).
- Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no
  Phase-2 branch exists).
