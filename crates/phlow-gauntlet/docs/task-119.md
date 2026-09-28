# task-119: resume mutation ordering

**Kind:** nvim-lua · **Status:** fail (open, diver defect) · **Wave:** 116–120 · **Commits:** pending (wave 116-120)

## ELI5

"Resume" means: take a finished job and put it back in the queue for
another try. The safe order is: first check "am I allowed to re-queue
this?", and only then touch anything. The harness does it backwards: it
rewrites the job's counters (attempt number, generation, flags) FIRST, and
only then checks. For a finished job the check says "not allowed" — but
the damage is already done: the job record now claims attempt 2, a new
generation, and a cleared flag, even though nothing was re-queued. "Correct"
means: validate first; if the resume is rejected, the record must be
byte-identical to before the call.

## What this task attempts

- **Goal:** prove `resume` validates before mutating, and that a rejected
  resume leaves the run untouched.
- **Mechanism:** diver's `ai.harness` — `supervisor.resume`
  (`supervisor.lua` line 313), the transition table (`types.lua`:
  `completed = {}`), `harness.resume`/`harness.cancel` (`init.lua`) — via
  the driver `crates/phlow-gauntlet/lua/gauntlet/task_119.lua`.
- **Success criterion:** failed runs resume cleanly (attempt+1,
  generation+1, same handle); completed runs are rejected with the record
  untouched; running runs are rejected; double resume advances attempt
  monotonically (+1 per resume) without double-counting generation.
- **Non-goals:** redefining which states are resumable. The docs
  (`init.lua`: "Completed runs are not resumable") are the contract; the
  driver tests the contract, it doesn't renegotiate it.

## What happened

Fail, open diver defect (Fix 5 absent) — on the first attempt. The
mutation order:

- `supervisor.resume` (line 313) checks terminality, then mutates
  `attempt`, `generation`, `_terminal_emitted`, and `handle`, then attempts
  the created→queued-style transition.
- For a completed run the transition is invalid (`completed = {}`), so
  resume reports the error — after corrupting the record. The driver
  snapshots the run before and after: attempt, generation,
  `_terminal_emitted`, and handle all changed.

Scenarios: `default` passes weakly (failed run re-queued and re-launched to
running, attempt 2, generation+1 — but the ordering itself is unverifiable
from outside, and the evidence says so); `generation-stale` passes (cancel
after resume bumps the generation, stale callbacks dropped);
`completed-resume` fails (`where = "fix-5-absent"` — corruption before
the error); `running-rejection` passes (running resume rejected; double
resume on a failed run advances attempt to 3 monotonically).

## The fix — what changed and why

No fix — this is a documented diver finding (Phase-2 Fix 5), and diver
findings are never fixed under gauntlet authority. The "fix" for the
gauntlet side was getting the evidence right:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_119.lua` (new) —
  four scenarios with before/after snapshots of the run record.
- **Why:** the defect is the *order*, not the resume feature; only a
  snapshot comparison on a rejected resume can prove mutate-before-validate.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  line 313 (`M.resume` — mutates, then validates),
  `~/workspace/repos/diver/lua/ai/harness/types.lua` (`completed = {}` —
  no outgoing transitions),
  `~/workspace/repos/diver/lua/ai/harness/init.lua` (docs: completed runs
  are not resumable).
- **Validation agents:** the 2 validation tests
  (`failed_run_resumes_with_clean_counters`,
  `cancel_after_resume_invalidates_old_generation`) assert the working
  resume path and pin the weak-pass caveat.
- **Adversarial agents:** the 2 adversarial tests
  (`completed_resume_corrupts_before_rejecting`,
  `running_resume_rejected_and_double_resume_monotonic`) prove the
  corruption and pin the monotonicity that already holds.

## Full technical depth

`M.resume(sup, run_id)` rejects non-terminal runs, then performs the
mutations (`run.attempt += 1`, `run.generation += 1`,
`run._terminal_emitted = false`, `run.handle = nil`) before calling the
transition to `queued`. For `failed`/`cancelled`/`timed_out` the transition
is legal and the mutations are the intended fresh-attempt setup. For
`completed` the transition fails — but the mutations persist: the record
now disagrees with reality (a completed run claiming attempt 2 with a
fresh generation and a cleared handle). Nothing repairs it; a later
reader cannot distinguish this corrupted record from a legitimately
re-queued one.

Phase-2 acceptance (banked, diver-owned): validate first — only terminal
non-completed runs may resume; a rejected resume leaves the run record
byte-identical (snapshot-compared); double resume advances attempt
monotonically, one per call, without double-counting generation.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/supervisor.lua` line 313
  (`M.resume` — mutate-then-validate).
- Primary: `~/workspace/repos/diver/lua/ai/harness/types.lua`
  (`TRANSITIONS.completed = {}`).
- Primary: `~/workspace/repos/diver/lua/ai/harness/init.lua` (`M.resume`
  docs: completed runs are not resumable).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_119.lua` (four
  scenarios; `completed-resume` `where = "fix-5-absent"`).
- Tests: `crates/phlow-gauntlet/tests/task_119.rs` (2V/2A).
- Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no
  Phase-2 branch exists).
