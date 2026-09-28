# task-125: run selection TOCTOU for cancel/resume

**Kind:** nvim-lua · **Status:** partial (open, diver gap: no picker commands) · **Wave:** 121–125 · **Commits:** pending (wave 121-125)

## ELI5

`:HarnessCancel` with no argument is supposed to show you a list of your
live jobs and let you pick one to stop. But jobs are alive: between you
seeing the list and picking one, the job might finish on its own. Hitting
"cancel" on a finished job must just say "already done" — not crash, not
corrupt anything, not write a confused log entry. And the picker must act
on the job's ID, never its name, so two identically-named jobs can't get
mixed up. Today the picker commands don't exist — but the underlying
cancel is already well-behaved, and this task proves it.

## What this task attempts

- **Goal:** prove the TOCTOU core (cancel-after-terminal fails cleanly)
  and pin the picker contracts — or record precisely that the picker
  commands don't exist.
- **Mechanism:** diver's `ai.harness` — `supervisor.cancel` (line 288)
  terminal guards, `supervisor.finish` (line 260), `M.list` (line 421),
  the (absent) `:HarnessCancel`/`:HarnessResume` commands — via the driver
  `crates/phlow-gauntlet/lua/gauntlet/task_125.lua`.
- **Success criterion:** cancel picker lists exactly the live runs;
  resume picker lists failed/cancelled/timed_out/interrupted (never
  completed, never running); selection is by run id; a run that goes
  terminal between listing and acting makes cancel return 'run is already
  terminal' with no corruption and no new events; unknown ids get a clean
  'unknown run' error.
- **Non-goals:** the prompt-fallback behavior (task-124) and the resume
  execution path itself (task-119 covered resume ordering). This task is
  the time-of-check/time-of-use seam in the UX layer.

## What happened

Partial — two scenarios pass today; two record the gap:

- `cancel-picker-missing` fails with `where = "command-module-absent"`:
  `vim.fn.exists(':HarnessCancel') == 0`. The record pins: picker over
  live runs only, selection by run id (identical workflow names
  disambiguated).
- `resume-picker-missing` fails with `where = "command-module-absent"`:
  `vim.fn.exists(':HarnessResume') == 0`. The record pins: picker over
  failed/cancelled/timed_out/interrupted; never completed, never running.
- `cancel-after-terminal` passes: a running run is finished by the driver
  (standing in for the adapter completing on its own), then
  `supervisor.cancel` returns `nil, 'run is already terminal: completed'`
  — run state stays `completed`, sink event count unchanged (no error
  event).
- `cancel-unknown-run` passes: `supervisor.cancel(sup,
  'run-that-never-existed')` returns `nil, 'unknown run:
  run-that-never-existed'` — run list unchanged, no state touched.

## The fix — what changed and why

No fix — the picker gap is diver-owned (Phase-2 Decision 2), and diver
findings are never fixed under gauntlet authority; the cancel guards
themselves are already correct and needed no change. The gauntlet-side
work was getting the evidence right:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_125.lua` (new) —
  four scenarios: two picker-absence probes, the TOCTOU core against
  `supervisor.cancel` directly, the unknown-run path.
- **Why:** this is the only task about time-of-check/time-of-use in the
  UX layer — the list the user saw vs the state at act time. Proving the
  act-time guard is clean today means Phase 2 only has to build the
  picker, not harden the cancel path.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  `M.cancel` (line 288 — unknown-run and already-terminal guards before
  any mutation), `M.finish` (line 260), `M.list` (line 421).
- **Validation agents:** the 2 validation tests (`cancel_picker_gap`,
  `resume_picker_gap`) assert the picker absence and pin both contracts.
- **Adversarial agents:** the 2 adversarial tests
  (`cancel_after_terminal_clean_refusal`, `cancel_unknown_run_clean_error`)
  assert the clean TOCTOU refusal and the clean unknown-run error.

## Full technical depth

`M.cancel` checks `sup.runs[run_id] == nil` → `'unknown run: ...'` and
`types.is_terminal(run.state)` → `'run is already terminal: ...'` before
touching generation, handle, or state — both guards return before any
mutation, so the refused cancel appends no events (verified by counting
`#(sink:events())` across the call). `running → completed` is a legal
transition (types.lua `TRANSITIONS`), so the driver can finish the run
directly to simulate the adapter completing between list and act. The
picker commands, when they ship, only need to list `M.list` filtered by
liveness and pass the selected run *id* to `M.cancel`/`M.resume` — the
act-time guards already make the TOCTOU window safe.

Phase-2 acceptance (banked, diver-owned): cancel picker lists exactly the
live runs; resume picker lists exactly
failed/cancelled/timed_out/interrupted; selection by id (identical
workflow names never confused); the act-time guards keep passing.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  `M.cancel` (line 288), `M.finish` (line 260), `M.list` (line 421).
- Primary: `~/workspace/repos/diver/lua/ai/harness/types.lua`
  `TRANSITIONS` (running → completed legal) and `is_terminal`.
- Spec: `~/workspace/your_files/diver-harness-phase2-spec.md` Decision 2
  (commands: hybrid args/prompts).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_125.lua` (four
  scenarios; two pass, two `where = "command-module-absent"`).
- Tests: `crates/phlow-gauntlet/tests/task_125.rs` (2V/2A).
- Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no
  Phase-2 branch exists).
