# task-129: approval-expiry one-shots

**Kind:** nvim-lua · **Status:** partial (open, diver gap: no approval-expiry one-shot; expiry-with-no-decision silently drops the run) · **Wave:** 126–130 · **Commits:** pending (wave 126-130)

## ELI5

Some jobs need a human to say "go ahead" — the job waits for approval,
and the approval has its own expiry ("answer by 3pm or the request
dies"). The Phase-2 plan sets an alarm for that expiry: when it rings,
the supervisor wakes up and marks the approval expired. If the human
answers in time, the alarm is cancelled. Today there is no alarm — the
expiry is just a note swept by `tick()`. Worse, when an approval expires
with nobody answering, the job is just… left there, untouched — the
tick notes the approval died but never does anything about the waiting
job. This task proves all of that, and records the missing alarm
precisely.

## What this task attempts

- **Goal:** characterize approval expiry today (timestamp swept by
  `tick()`; request schedules no timer; a grant/deny at the wire
  resolves cleanly under tick serialization; **expiry with no decision
  leaves the run untouched — the silent drop**) and record precisely
  that no one-shot fires at approval expiry, with the exact hook
  location — or record that the surface shipped.
- **Mechanism:** diver's `ai.harness` — `approval.request`
  (`deadline_ns`), `approval.decide`, `supervisor.tick` expiry sweep
  (lines 473–477) — via the driver
  `crates/phlow-gauntlet/lua/gauntlet/task_129.lua`.
- **Success criterion:** requesting an approval schedules zero uv timers
  and carries no handle; an approval past expiry marks 'expired' only
  via an explicit `tick()` — and the run is NOT handled per policy (no
  transition, no outcome event); grant-at-T-eps and deny-then-expiry
  resolve with no double-decision and no phantom expiry; the gap record
  pins the one-shot with file/line evidence and the exact hook point
  (`approval.request`, `approval.lua:71`).
- **Non-goals:** the wake mechanism itself (task-126), deadline timers
  (task-127), retry timers (task-128). This task is the approval timer
  lifecycle only.

## What happened

Partial — three scenarios pass today; one records the gap:

- `no-approval-timer` passes: uv timer delta across `request` is zero,
  the approval entry has no timer handle — the request path is
  timer-free.
- `approval-expiry-via-tick` passes: two approvals on different runs with
  staggered expiries (A 150ms, B 60000ms) — past A's expiry, nothing
  fires until an explicit `tick()`, which marks A 'expired' while B stays
  'pending' (independent expiry, no cross-talk); **the runs themselves
  are untouched** (no transition, no outcome event). Granting B then
  survives a later tick (stays 'approved') while A stays 'expired'
  (independent decisions, no cross-talk). The sweep marks; it never
  acts. This documents the silent drop that Phase 2 must handle.
- `grant-before-expiry-no-double-decision` passes: grant at T-eps
  survives the later expiry sweep (approval stays 'granted'), and
  deny-then-expiry stays 'denied' — decide is silent (zero sink
  events), so no phantom expiry decision exists.
- `approval-one-shot-absent` fails with
  `where = "approval-one-shot-absent"`: `approval.request`
  (`approval.lua:71`) stores `deadline_ns` and schedules nothing; the
  expiry sweep lives in `M.tick` (lines 473–477). The record pins the
  acceptance: one-shot at expiry → `wake(sup, now, 'approval')` →
  `sweep_expired`; grant/deny cancels the timer; no double-decision;
  per-run approval independence — and the currently unhandled
  expiry-with-no-decision stays banked as a design decision Phase 2
  must make (not as a silent drop).

## The fix — what changed and why

No fix — approval timers are diver-owned (Phase-2 Decision 3), and
diver findings are never fixed under gauntlet authority; the tick-based
sweep already works as documented and needed no change. The
gauntlet-side work was getting the evidence right:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_129.lua` (new) —
  four scenarios: the timer-free request, the expiry sweep, the
  grant/deny races, and the gap record.
- **Why:** this is the only task covering approval timer lifecycle —
  and the only one that caught the silent drop: expiry-with-no-decision
  marks the approval but never handles the run. The approval docs say
  expiry means "denied", but no `transition()` follows, so the run
  sits in `waiting_approval` forever. Phase 2 must decide what expiry
  *does* to the run — the one-shot makes the question unavoidable.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/approval.lua`
  `M.request` (line 71 — timestamp only, no timer), `M.decide`
  (silent state change, no event), `supervisor.lua` expiry sweep
  (lines 473–477 — marks, never transitions).
- **Validation agents:** the 2 validation tests (`approval_one_shot_gap`,
  `no_approval_timer_pass`) assert the gap record and pin the hook
  location.
- **Adversarial agents:** the 2 adversarial tests
  (`approval_expiry_leaves_run_untouched`,
  `grant_before_expiry_no_double_decision_pass`) document the silent
  drop and the race resolution.

## Full technical depth

`approval.request` (approval.lua:71) stores
`approval.deadline_ns = now_ns + timeout_ms * 1e6` in the `approvals`
table keyed by `run_id`; no handle, no timer. `approval.decide`
changes `approval.status` silently — zero sink events — so decide
cannot race a timer (there is none) and cannot double-decide. The
`tick()` sweep (supervisor.lua:473–477) marks overdue approvals
`'expired'` and that's all: no `transition()`, no outcome event, the
run stays in `waiting_approval` indefinitely. The design doc ("an
approval expiry means 'denied'") is an aspiration, not behavior — the
tick marks the approval but the run is never denied. Phase 2's one-shot
forces the design decision: at expiry the wake calls
`sweep_expired`, which must do *something* with the run — deny it
(transition to `failed` with reason, or `denied`), or emit exactly one
approval-outcome event. Grant/deny before expiry must cancel the
one-shot (no phantom expiry), and the no-double-decision rule holds:
decide on an expired approval is refused.

Phase-2 acceptance (banked, diver-owned): an approval request schedules
a one-shot at expiry → `wake(sup, now, 'approval')` →
`sweep_expired`; grant/deny cancels the timer (no fire after decision);
expiry with no decision produces exactly one approval-outcome event and
handles the run (denied — transitioned, not silently dropped); approvals
on different runs are independent (no cross-talk); decide-after-expiry
is refused (no double-decision).

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/approval.lua`
  `M.request` (line 71), `M.decide` (silent state change).
- Primary: `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  expiry sweep (lines 473–477 — marks, never transitions).
- Spec: `~/workspace/your_files/diver-harness-phase2-spec.md`
  "Supervision without polling" — one-shot timers (Decision 3);
  "an approval expiry means 'denied'" (aspiration vs behavior).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_129.lua` (four
  scenarios; three pass, one `where = "approval-one-shot-absent"`).
- Tests: `crates/phlow-gauntlet/tests/task_129.rs` (2V/2A).
- Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no
  Phase-2 branch exists).
