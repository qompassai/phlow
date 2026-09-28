# task-127: deadline one-shots and handle hygiene

**Kind:** nvim-lua · **Status:** partial (open, diver gap: no deadline one-shot, no timer tracking in `transition()`) · **Wave:** 126–130 · **Commits:** pending (wave 126-130)

## ELI5

Every job gets a deadline — "finish by 3pm or you're timed out." The
Phase-2 plan sets an alarm clock for each deadline: when it rings, the
supervisor wakes up and marks the job timed out. If the job finishes
early, the alarm is cancelled so it never rings for a dead job. Today
there are no alarm clocks at all — the deadline is just a note on the
job, and somebody has to walk by and check the time (`tick()`). This
task proves the note-checking works, and records exactly what's missing:
the alarm, the per-job alarm list, and the cancel-on-finish step.

## What this task attempts

- **Goal:** characterize deadline enforcement today (timestamp compared
  inside `tick()` only; nothing scheduled at create) and record precisely
  that no one-shot fires at `deadline_ns` and `transition()` cancels no
  timer handles — or record that the surface shipped.
- **Mechanism:** diver's `ai.harness` — `supervisor.create`
  (`deadline_ns`), `M.tick` (deadline check), `transition()` (terminal
  choke point) — via the driver
  `crates/phlow-gauntlet/lua/gauntlet/task_127.lua`.
- **Success criterion:** a run past its deadline stays `running` until
  an explicit `tick()` marks it `timed_out` (reason 'deadline exceeded');
  create schedules zero uv timers and sets no `run._timers`; the gap
  records pin the one-shot and the `transition()` cancellation with
  file/line evidence.
- **Non-goals:** the wake mechanism itself (task-126), retry timers
  (task-128), approval timers (task-129). This task is resource
  correctness of deadline timers — not logic correctness.

## What happened

Partial — two scenarios pass today; two record the gap:

- `deadline-fires-via-tick` passes: a run with `timeout_ms=150` stays
  `running` past its `deadline_ns` with no `tick()` call; an explicit
  `tick()` then marks it `timed_out` with reason 'deadline exceeded'; a
  second `tick()` far past the deadline emits no second `run.finished`
  (exactly one — finish is idempotent at the `transition()` choke
  point). A second run is then put into `waiting_approval` directly
  (nothing in the harness transitions there today — the state exists
  only in `types.lua`; the precondition is simulated and labelled as
  such): it also stays put past its deadline with no `tick()`, and an
  explicit `tick()` marks it `timed_out` — the deadline is absolute, not
  paused, for `waiting_approval` — again with exactly one `run.finished`
  across a second tick.
- `no-one-shot-at-create` passes: uv timer count is identical before
  and after create (delta zero) and `run._timers == nil`.
- `deadline-one-shot-absent` fails with `where = "deadline-one-shot-absent"`:
  `M.create` (supervisor.lua:116–172) schedules no timer; the deadline is
  enforced only by the `tick()` comparison at line 482. The record pins
  the acceptance: one-shot at `deadline_ns` → `wake(sup, now, 'deadline')`.
- `terminal-entry-no-timer-cleanup` fails with
  `where = "timer-handle-hygiene-absent"`: a run finished before its
  deadline, then ticked far past it, stays `completed` with zero uv
  timers — the hygiene property holds vacuously — while `transition()`
  (supervisor.lua:86–112) contains no `_timers` cancellation. The record
  pins the acceptance: handles tracked in `run._timers`, cancelled in
  `transition()` on terminal entry, zero live handles after terminal
  (the handle-leak assertion — leaked handles keep the loop alive).

## The fix — what changed and why

No fix — deadline timers are diver-owned (Phase-2 Decision 3), and diver
findings are never fixed under gauntlet authority; the tick-based
enforcement already works and needed no change. The gauntlet-side work
was getting the evidence right:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_127.lua` (new) —
  four scenarios: the tick-enforcement characterization, the
  create-schedules-nothing characterization, and the two gap records.
- **Why:** this is the only task about one-shot timer lifecycle and
  uv-handle hygiene — resource correctness, not logic correctness. A
  leaked timer handle keeps the event loop alive forever, which is
  exactly the phone-battery failure Decision 3 exists to prevent.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  `M.create` (lines 116–172 — no timer), `M.tick` deadline check (line
  482), `transition()` (lines 86–112 — no cancellation).
- **Validation agents:** the 2 validation tests (`deadline_one_shot_gap`,
  `timer_handle_hygiene_gap`) assert the gap records and pin both
  contracts.
- **Adversarial agents:** the 2 adversarial tests
  (`deadline_fires_via_tick_pass`, `no_one_shot_at_create_pass`) assert
  the tick-only enforcement and the zero-timer create.

## Full technical depth

`M.create` sets `run.deadline_ns = now_ns + timeout_ms * 1e6` (a bare
number; no handle). `M.tick` checks `run.deadline_ns ~= nil and now_ns
>= run.deadline_ns` per non-terminal run (line 482) and finishes with
`'timed_out', 'deadline exceeded'`. `transition()` (lines 86–112) is the
single choke point for every terminal path — it validates via
`types.can_transition`, appends `run.state_changed`, and emits
`run.finished` exactly once per attempt — but touches no timers, because
none exist: no `run._timers` table is ever created. Finishing a run
before its deadline and ticking far past it leaves the run `completed`
(the `is_terminal` skip at line 476) with zero uv handles — correct
today only because there is nothing to leak. Phase 2 must add the
one-shot at create, track it in `run._timers`, and cancel it in
`transition()` — and the handle-leak assertion (zero live uv handles
after terminal entry) is the executable form of that requirement.

Phase-2 acceptance (banked, diver-owned): create schedules a one-shot
at `deadline_ns` → `wake(sup, now, 'deadline')`; the handle is tracked
in `run._timers`; `transition()` cancels all tracked handles on
terminal entry; finishing before the deadline cancels the one-shot
(zero wakes after terminal, zero live handles); a deadline expiring
while the run is in `waiting_approval` still times out (absolute, not
paused); cancel is idempotent so a double-firing timer yields a single
`timed_out` and no double `run.finished`.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  `M.create` (lines 116–172), `M.tick` deadline check (line 482),
  `transition()` (lines 86–112).
- Spec: `~/workspace/your_files/diver-harness-phase2-spec.md`
  "Supervision without polling" — one-shot timers (Decision 3).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_127.lua` (four
  scenarios; two pass, two `where = "deadline-one-shot-absent"` /
  `"timer-handle-hygiene-absent"`).
- Tests: `crates/phlow-gauntlet/tests/task_127.rs` (2V/2A).
- Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no
  Phase-2 branch exists).
