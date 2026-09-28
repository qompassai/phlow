# task-123: `:HarnessRun` argument parsing contract

**Kind:** nvim-lua · **Status:** partial (open, diver gap: no command module) · **Wave:** 121–125 · **Commits:** pending (wave 121-125)

## ELI5

There is supposed to be a command you type — `:HarnessRun a2a deploy --
fix the thing` — where everything after the `--` is your goal, word for
word, quotes and pipes and all. Type just `:HarnessRun` and it asks you
what's missing. Today the command doesn't exist at all: typing it gets
you "not an editor command". "Correct" means: the command exists, the
parsing is hybrid (args when given, prompts for the rest), and the goal
after `--` arrives byte-identical — with exact rules for the tricky bits
(a `--` inside the goal, `%`/`#`, newlines, empty goals).

## What this task attempts

- **Goal:** prove the `:HarnessRun` hybrid parsing contract — or record
  precisely that no command module exists to test it against.
- **Mechanism:** diver's `ai.harness` — the (absent) command module,
  `types.validate_run_spec` (goal non-empty, the "never a goal-less run"
  building block) — via the driver
  `crates/phlow-gauntlet/lua/gauntlet/task_123.lua`.
- **Success criterion:** `:HarnessRun a2a deploy -- fix the "quoted" thing
  | properly` → adapter `a2a`, workflow `deploy`, goal byte-identical;
  only the first `--` is the separator; empty goal after `--` falls back
  to prompting (never a goal-less run); `%`/`#` never filename-expand; a
  newline is preserved or cleanly rejected, never silently truncated.
- **Non-goals:** the interactive prompt behavior itself (task-124) and the
  cancel/resume pickers (task-125). This task is the command-line parsing
  boundary: where shell-like syntax meets free text.

## What happened

Partial — one scenario passes today; three record the gap:

- `parse-harnessrun` fails with `where = "command-module-absent"`:
  `vim.fn.exists(':HarnessRun') == 0`, `vim.cmd('HarnessRun ...')` fails
  E492, and no `HarnessRun` / `nvim_create_user_command` string exists in
  the harness sources. The record pins the acceptance: adapter, workflow,
  goal-verbatim-after-first-`--`.
- `goal-required` passes: `validate_run_spec` accepts a non-empty goal
  and rejects missing/empty goals (types.lua) — the building block the
  "never a goal-less run" rule rests on.
- `double-dash-in-goal` fails with `where = "command-module-absent"`:
  `:HarnessRun a2a w -- -- -- --` must yield goal `-- -- --` (only the
  first `--` is the separator) — unverifiable, acceptance pinned.
- `percent-hash-newline` fails with `where = "command-module-absent"`:
  `%`/`#` must stay literal (no `expand()`/filename expansion on the goal
  span); a newline must be preserved or cleanly rejected, never silently
  truncated — unverifiable, acceptance pinned.

## The fix — what changed and why

No fix — the gap is diver-owned (Phase-2 Decision 2), and diver findings
are never fixed under gauntlet authority. The gauntlet-side work was
getting the evidence right:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_123.lua` (new) —
  four scenarios: command-absence probe, goal-requirement
  characterization, separator semantics, expansion/newline semantics.
- **Why:** parsing is the boundary where shell-like syntax meets free
  text, and each tricky bit (`--` in the goal, `%`/`#`, newlines) is a
  distinct silent-corruption risk. Pinning them as acceptance criteria now
  means the future command is tested against the exact contract.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/` (no command
  module; no `:Harness*` registration), `~/workspace/repos/diver/lua/ai/harness/types.lua`
  `validate_run_spec` (goal non-empty).
- **Validation agents:** the 2 validation tests (`parse_harnessrun_gap`,
  `goal_required_by_validate_run_spec`) assert the absence and the goal
  requirement.
- **Adversarial agents:** the 2 adversarial tests
  (`double_dash_separator_gap`, `percent_hash_newline_gap`) pin the
  separator and expansion rules.

## Full technical depth

The harness ships no user-command surface: no `commands.lua`, no
`nvim_create_user_command` call, no `:HarnessRun` registration anywhere
under `lua/ai/harness/`. `types.validate_run_spec` (types.lua) requires
`spec.goal` to be a non-empty string, which is the backstop the command
will rely on — but the command-level rules (first-`--`-only separator,
literal `%`/`#`, newline preserved-or-rejected, empty-goal-prompts) have
no implementation to test. Each gap record names its exact acceptance so
the Phase-2 command lands against an executable spec rather than a vague
"parse args" ticket.

Phase-2 acceptance (banked, diver-owned): the four parsing rules above,
each byte-exact; an empty goal after `--` prompts for the goal rather than
erroring or creating a goal-less run.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/` — no command module,
  no `:HarnessRun` / `nvim_create_user_command` in harness sources.
- Primary: `~/workspace/repos/diver/lua/ai/harness/types.lua`
  `validate_run_spec` (spec.goal must be a non-empty string).
- Spec: `~/workspace/your_files/diver-harness-phase2-spec.md` Decision 2
  (hybrid args/prompts, `--` verbatim goal).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_123.lua` (four
  scenarios; one passes, three `where = "command-module-absent"`).
- Tests: `crates/phlow-gauntlet/tests/task_123.rs` (2V/2A).
- Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no
  Phase-2 branch exists).
