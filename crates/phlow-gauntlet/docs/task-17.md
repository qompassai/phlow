# task-17: nvim edit→check→fix loop

**Kind:** nvim-lua · **Status:** pass · **Wave:** 4a · **Commits:** pending (wave 4a)

## ELI5

Imagine a robot assistant that edits code for you. It writes a file,
then it needs to check its own work: run a syntax checker, read the
error message, fix the mistake, and check again. This task proves that
whole loop works. The "agent" is played by a test script running inside
headless Neovim; it deliberately writes a Lua file with a syntax error,
runs `luac -p` (the Lua syntax checker) on it, reads the error (which
file, which line, what went wrong), fixes the file, and runs the
checker again until it passes.

Two things make this loop *trustworthy* rather than just functional.
First, the checker never *runs* the file — `luac -p` only reads it, the
way a spell-checker reads an essay without following its instructions.
So even a hostile file can't do anything through this loop. Second, the
loop gives up after a bounded number of tries: if the fixes never work,
it stops and says so honestly instead of spinning forever or pretending
it succeeded.

## What this task attempts

- **Goal:** prove the edit→check→fix loop converges: a broken Lua file
  is detected by `luac`, the error is surfaced legibly, a fix is
  applied, and re-verification passes — all driven through headless
  Neovim with job control.
- **Mechanism:** `crates/phlow-gauntlet/src/tasks/task_17.rs`
  (`run`/`run_scenario`/`run_scenario_with_luac` →
  `phlow_gauntlet::run_nvim_lua_driver_with_env`) spawns headless nvim
  on `lua/gauntlet/task_17.lua`, which writes `target.lua` into the
  task work dir, runs `luac -p` via `vim.system` (Neovim's job-control
  API), parses `file:line: message` out of stderr, applies the
  scenario's fix plan (up to 3 attempts), and prints one JSON verdict.
  `tests/task_17.rs` asserts the verdicts and the operator-visible
  transcript.
- **Success criterion:** default scenario passes with the transcript
  `edit → check(error) → fix → check(clean)`; the bad-fix scenario
  catches the *new* error and still converges; the no-converge scenario
  stops after exactly 3 fixes and reports honest failure.
- **Non-goals:** driving diver's real `ai.harness` supervisor (tasks
  01–05 cover harness lifecycle; this task isolates the loop
  mechanism); semantic checks beyond syntax (luac is a parser, not a
  linter); a TUI surface — the loop's UI is its stdout transcript (see
  below).

## What happened

Iteration 1. The Lua driver was written and run directly through
headless nvim (`~/.local/nvim-nightly/bin/nvim --headless -l`):
**all three scenarios behaved exactly as designed on the first run** —
`default` passed with the golden transcript, `bad-fix` caught the new
error (`unexpected symbol near '+'`) and converged on the second fix,
and `no-converge` oscillated visibly between the two errors and stopped
after exactly 3 fixes with `where: "fix"`.

The Rust integration tests (`cargo test -p phlow-gauntlet --test
task_17`, 2 validation + 2 adversarial + metadata) needed one
scaffolding iteration: the first compile failed on a missing `TaskKind`
import and on `std::env::set_var`, which is `unsafe` in edition 2024.
Rather than wrapping it in `unsafe`, the fix introduced
`run_scenario_with_luac(ctx, scenario, luac_bin)`, which forwards the
luac path as an explicit driver environment variable — no process-wide
env mutation, so concurrent test threads cannot race. After that:
5/5 green.

Evidence excerpts (default scenario transcript — the operator sees
exactly these lines):

```
[task-17] stage=edit file=target.lua lines=5
[task-17] stage=check attempt=1 cmd="luac -p target.lua"
[task-17] luac-error file=target.lua line=6 msg=')' expected (to close '(' at line 5) near <eof>
[task-17] hint: fix the error above, then re-run luac
[task-17] stage=fix attempt=1 file=target.lua lines=5
[task-17] stage=check attempt=2 cmd="luac -p target.lua"
[task-17] luac-clean file=target.lua
[task-17] result: pass checks=2 fixes=1
```

## Full technical depth

**The loop, end to end.** The Rust wrapper resolves the `luac` binary
(`GAUNTLET_LUAC_BIN` env wins, then a `PATH` search for an executable
file named `luac`; missing → fail-closed `Fail{where: "check"}`) and
spawns `nvim --headless -l lua/gauntlet/task_17.lua` with
`GAUNTLET_WORK_DIR`, `GAUNTLET_SCENARIO`, and `GAUNTLET_LUAC_BIN` in
the child's environment. The Lua driver:

1. **Edit** — writes `target.lua` with a missing `)` on the `print`
   call (5 lines). Writes are confined to `GAUNTLET_WORK_DIR`; the
   driver touches nothing else.
2. **Check** — `vim.system({luac, '-p', path}, {text=true}):wait(10000)`
   spawns the checker as a job (separate argv, no shell — so a hostile
   filename cannot inject shell syntax) and waits up to 10 s. `luac`
   exits 1 with `target.lua:6: ')' expected (to close '(' at line 5)
   near <eof>` on stderr. The driver parses the first `:line: message`
   off that line; if parsing fails it surfaces the raw line rather
   than inventing a line number.
3. **Fix** — writes the scenario's next fix content and re-checks.
   `FIX_ATTEMPTS_MAX = 3` bounds the loop.
4. **Verify** — the loop returns `pass` only when a check exits 0.
   The `no-converge` plan (fixes oscillate `BROKEN_V2 → BROKEN →
   BROKEN_V2`) proves the bound: after 3 unsuccessful fixes the driver
   reports `Fail{where: "fix", how: "fix did not converge after 3
   attempts; last luac error: target.lua:6: unexpected symbol near
   '+'"}` — the operator learns exactly where the loop gave up.

**Why `luac -p` is the right checker here.** The `-p` flag means
parse-only: luac compiles the chunk and discards it without executing
a single instruction. The loop therefore has no code-execution
surface — a file that is syntactically valid but semantically hostile
(e.g. `os.execute("...")`) still *parses clean*, and nothing in this
loop would run it. The task's threat model is the loop itself
(non-termination, blind trust of fixes, unsound checks), not the
checked file's runtime behavior — and the three scenarios cover
exactly those: oscillation (non-termination), bad-fix (blind trust),
and the driver's own guard that a clean first check on a broken file
is a `Fail` at stage `check` (unsound checker).

**UI/UX coverage.** There is no TUI surface for this loop, so
ratatui's `TestBackend` does not apply — the loop's UI *is* its stdout
transcript, and the tests assert on it directly (the sourced
alternative for CLI surfaces: golden-output assertions on the rendered
text). The driver writes every progress line with `io.stdout:write`
(not `print`, which `nvim --headless -l` routes to stderr) and also
embeds the same lines in the verdict evidence, so the operator and the
report see identical text. The tests assert:

- the *exact* golden transcript for the default scenario (order,
  wording, counts) — any rendering drift fails the test;
- legibility: the error line names `file=`, `line=`, `msg=`; a
  `hint:` line with the next action immediately follows the error;
  each stage appears exactly once (sane progress, no duplicated or
  skipped steps);
- boundedness is visible: the no-converge transcript shows exactly 3
  `stage=fix` lines and 4 `stage=check` lines, and the failure's `how`
  carries the last luac error with file, line, and message.

**Job control details.** `vim.system` (Neovim ≥ 0.10) is the
structured job-control API: it takes argv as a list (no shell
interpolation), captures stdout/stderr separately with `text = true`,
and `:wait(timeout_ms)` bounds the wait. The outer Rust runner
additionally enforces `ctx.timeout` (120 s in tests) by killing a
wedged nvim child — two independent deadlines, so a hung checker can
neither hang the driver nor the gauntlet.

## Sources

- Primary: `crates/phlow-gauntlet/lua/gauntlet/task_17.lua` (driver),
  `crates/phlow-gauntlet/src/tasks/task_17.rs` (spawn + luac
  resolution), `crates/phlow-gauntlet/src/lib.rs`
  (`run_nvim_lua_driver_with_env`, verdict parsing).
- Neovim documentation: `vim.system()` (`:h vim.system`) — argv-list
  job spawning, `text = true` capture, `:wait()` semantics; `nvim
  --headless -l` (`:h -l`) — Lua `print()` goes to stderr, hence
  `io.stdout:write` for the verdict.
- Lua 5.4 reference manual: `luac` — the `-p` option performs "parse
  only" (no execution); exit status nonzero with `file:line: message`
  on stderr for syntax errors.
- Secondary: `docs/00-design.md` task-17 brief (spec: "harness-driven
  agent edits a Lua file, runs `luac` via job control, reads the error,
  fixes it, re-verifies clean"); `lua/gauntlet/task_05.lua` (driver
  conventions imitated: verdict JSON on stdout, `GAUNTLET_SCENARIO`
  dispatch, evidence cap).
