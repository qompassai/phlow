# task-124: prompt fallback and abort atomicity

**Kind:** nvim-lua · **Status:** fail (open, diver gap: no command module) · **Wave:** 121–125 · **Commits:** pending (wave 121-125)

## ELI5

Type `:HarnessRun` with nothing after it and it's supposed to ask you
three questions, in order: which worker, which workflow, what's the goal.
Skip any question — press escape, leave it blank — and the whole thing
cancels: no half-made job, no stray log entries. And a goal that's only
spaces counts as skipping, not as a goal. Today none of this exists
because the command doesn't exist. "Correct" means: the three prompts
happen in order, aborts are all-or-nothing, and whitespace-only is an
abort.

## What this task attempts

- **Goal:** prove the prompt-fallback ordering and abort atomicity — or
  record precisely that no command module exists to test them against.
- **Mechanism:** diver's `ai.harness` — the (absent) command module,
  `vim.ui.select`/`vim.ui.input` contracts (nil/empty on cancel),
  `M.run` (the creation point), `types.validate_run_spec` — via the
  driver `crates/phlow-gauntlet/lua/gauntlet/task_124.lua`.
- **Success criterion:** bare `:HarnessRun` → three prompts in order
  (adapter→workflow→goal), exactly one run, one `run.created` event;
  partial args prompt only for the missing pieces; abort (nil or empty
  string — identical) at any stage → zero runs, zero events; whitespace-
  only goal → abort, never a goal.
- **Non-goals:** the argument parsing rules (task-123) and the
  cancel/resume pickers (task-125). This task is the interactive-fallback
  ordering and the all-or-nothing command atomicity.

## What happened

Fail, open diver gap (Decision 2) — on the first attempt. All four
scenarios record `where = "command-module-absent"`:

- `bare-harnessrun`: no `:HarnessRun`, so no prompt order to observe. The
  record pins the acceptance: adapter (select over registry) → workflow
  (input) → goal (input); one run; one `run.created` event.
- `partial-args`: `:HarnessRun a2a` fails E492. The record pins: prompt
  only for the missing two.
- `abort-atomicity`: unverifiable. The record pins the six scripted
  scenarios (3 stages × {nil, empty}) each leaving run count and event
  count unchanged.
- `whitespace-goal`: unverifiable as a command rule — but the driver
  characterizes the building block: `validate_run_spec` ACCEPTS
  `goal = '   '` (it is a non-empty string), so the trim rule MUST live in
  the command layer; the spec validator alone cannot enforce it.

## The fix — what changed and why

No fix — the gap is diver-owned (Phase-2 Decision 2), and diver findings
are never fixed under gauntlet authority. The gauntlet-side work was
getting the evidence right:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_124.lua` (new) —
  four scenarios, each recording the command-module gap with its full
  acceptance contract, plus the whitespace building-block
  characterization.
- **Why:** abort atomicity is the all-or-nothing property of the whole
  command — without it, a cancelled prompt leaves a half-specified run or
  phantom events. The whitespace finding is the subtle one: it proves the
  trim check cannot be delegated to `validate_run_spec`.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/` (no command
  module), `~/workspace/repos/diver/lua/ai/harness/types.lua`
  `validate_run_spec` (accepts `'   '` — no trim).
- **Validation agents:** the 2 validation tests (`bare_harnessrun_gap`,
  `partial_args_gap`) pin the prompt order and the partial-args rule.
- **Adversarial agents:** the 2 adversarial tests (`abort_atomicity_gap`,
  `whitespace_goal_gap`) pin the all-or-nothing abort contract and the
  command-layer trim rule.

## Full technical depth

`vim.ui.select`/`vim.ui.input` return nil on cancel; the design additionally
treats empty-string returns as abort (both behave identically). The sink
is append-only, so "zero events appended" is checkable: snapshot
`#(sink:events())` before the scripted abort, assert unchanged after.
When Phase 2 ships the command, this driver becomes its executable spec:
headless nvim with scripted `vim.ui` sequences (select/input stubbed to
return scripted values), counting `supervisor.list` runs and sink events
after each scenario.

Phase-2 acceptance (banked, diver-owned): prompt order
adapter→workflow→goal; partial args prompt only for missing pieces;
nil/empty at any stage → zero runs, zero events; whitespace-only goal →
abort (command-layer trim, since `validate_run_spec` accepts `'   '`).

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/` — no command module,
  no `vim.ui.select`/`vim.ui.input` call sites in harness.
- Primary: `~/workspace/repos/diver/lua/ai/harness/types.lua`
  `validate_run_spec` (no trim — `'   '` passes).
- Spec: `~/workspace/your_files/diver-harness-phase2-spec.md` Decision 2
  (hybrid prompts; prompt abort = whole command aborts).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_124.lua` (four
  scenarios, all `where = "command-module-absent"`).
- Tests: `crates/phlow-gauntlet/tests/task_124.rs` (2V/2A).
- Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no
  Phase-2 branch exists).
