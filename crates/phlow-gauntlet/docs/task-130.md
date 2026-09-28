# task-130: idle-loop quietness (the phone-battery test)

**Kind:** nvim-lua · **Status:** partial (open, diver gap: no per-deadline pending handles, no `VimLeavePre` timer teardown — but the loop is quiet today by absence) · **Wave:** 126–130 · **Commits:** pending (wave 126-130)

## ELI5

This is the "phone battery" test. Matt's rule for this design is: when
there's nothing to do, the program must sleep — not check every second
"anything yet? anything yet?" A program that checks every second keeps
the phone awake and drains the battery; a program that sets an alarm for
the next real thing and sleeps until then uses almost no power. Today
the harness sleeps — but only because nothing *can* wake it (no timers,
no wake hook). Phase 2 adds alarms (one per deadline) and must keep the
sleep: the acceptance is exactly one pending alarm per live deadline,
zero wakeups when idle, and every alarm cancelled when the program
quits. This task runs the actual 10-second sleep test, settles 100 jobs,
and records the two missing pieces.

## What this task attempts

- **Goal:** run the real quietness acceptance — a 10s idle window with
  zero live runs (zero new uv handles, `last_tick_ns` unmoved, zero new
  sink events), 100 settled runs back at the handle baseline — and
  record precisely that no per-deadline pending handle exists and no
  `VimLeavePre` timer teardown exists — or record that the surface
  shipped.
- **Mechanism:** diver's `ai.harness` — `harness.setup`, a 24h-deadline
  run, `vim.uv.walk` handle census (real `/usr/bin/nvim` headless),
  source scan for `VimLeavePre` — via the driver
  `crates/phlow-gauntlet/lua/gauntlet/task_130.lua`.
- **Success criterion:** after 10s idle with zero live runs: uv handle
  delta = 0, `sup.last_tick_ns` delta = 0 (no background ticking), sink
  delta = 0; after 100 runs created and settled: handle count back at
  the pre-setup baseline (zero residue); the gap records pin the
  one-handle-per-deadline and teardown acceptance with file/line
  evidence.
- **Non-goals:** the wake mechanism (task-126), individual timer
  lifecycles (tasks 127–129). This task is the *justification* for "no
  poll" over 250ms/1s — the design's core tradeoff, made measurable.

## What happened

Partial — two scenarios pass today; two record the gap:

- `idle-window-zero-activity` passes: after a real 10s idle observation
  with zero live runs — zero new uv handles, `sup.last_tick_ns` delta =
  0 (no background ticking), sink delta = 0. The loop sleeps. The
  24h-deadline run contributes nothing because nothing is scheduled for
  it. Measurement note: `nvim --headless -l` materializes one repeating
  uv timer (repeat=200) of its own the first time the loop idles —
  verified with no harness loaded — so the scenario warms up 1s before
  the baseline census and asserts zero *delta* across the window.
- `settled-runs-baseline-handles` passes: 100 runs created and settled
  return the handle count to the pre-setup baseline — zero residue.
  (Vacuous today: there was never anything to leak.)
- `deadline-handle-absent` fails with
  `where = "deadline-handle-absent"`: a run with a 24h `deadline_ns`
  has exactly 0 pending uv handles — the live source scan shows no
  timer-scheduling path (zero matches for `new_timer`/`timer_start`),
  so there is nothing *to* hold. The record pins the acceptance:
  exactly one pending uv handle per live deadline.
- `timer-teardown-absent` fails with `where = "timer-teardown-absent"`:
  a source scan finds zero `VimLeavePre` handlers in
  `lua/ai/harness/` — no teardown exists because no timers exist. The
  record pins the acceptance: a `VimLeavePre` autocmd that closes every
  tracked handle, emits no errors, and returns the handle count to
  baseline.

## The fix — what changed and why

No fix — the timer surface is diver-owned (Phase-2 Decision 3), and
diver findings are never fixed under gauntlet authority; the loop is
already quiet and needed no change. The gauntlet-side work was getting
the evidence right:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_130.lua` (new) —
  four scenarios: the real 10s idle observation, the 100-run residue
  check, and the two gap records (one runtime, one source-scan).
- **Why:** this is the design's central tradeoff made measurable —
  "no poll" is justified by quietness, and quietness is only real if
  the alarms themselves are bounded (one per deadline) and cleaned up
  (teardown). Proving today's quietness is by-absence means Phase 2's
  job is precisely bounded: keep the baseline this task measures.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/` — zero uv timer
  creations; zero `VimLeavePre` handlers.
- **Validation agents:** the 2 validation tests (`deadline_handle_gap`,
  `timer_teardown_gap`) assert the gap records and pin both contracts.
- **Adversarial agents:** the 2 adversarial tests
  (`idle_window_zero_activity_pass`,
  `settled_runs_baseline_handles_pass`) run the actual quietness
  acceptance.

## Full technical depth

The driver runs under real headless Neovim (`/usr/bin/nvim` on primo),
using `vim.uv.walk` (with a `pcall` fallback to `vim.loop.walk` for
older builds) to census live uv handles. The 10s window uses
`vim.wait(10000)` — the event loop genuinely idles; there is no harness
code path that could tick in the background (`tick()` is only called
explicitly, and the driver calls it only where the scenario requires).
`sup.last_tick_ns` is set only inside `M.tick`, so its delta of zero
proves no background ticking. The handle census counts only uv handles
(process-level), excluding per-window/tabpage noise — the setup handle
count is taken after `harness.setup({})` and compared at the end.
Source scanning for the teardown walks `lua/ai/harness/` (both RTP-root
and Lua-dir layouts, shared `resolve_diver_dirs()` bootstrap) and
matches the literal `VimLeavePre` — zero matches.

Phase-2 acceptance (banked, diver-owned): exactly one pending uv handle
per live deadline (a 24h-deadline run holds exactly one timer, not a
polling interval and not zero); the idle window shows zero wakeups
with live deadlines present — only the one-shot fires at its time;
`VimLeavePre` teardown closes every tracked handle, emits no errors,
and the handle count returns to baseline; leaked handles fail the
build (the residue check becomes a real guard, not a vacuous one).

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/` — zero `vim.uv`
  timer creations; zero `VimLeavePre` handlers (verified by runtime
  `vim.uv.walk` census and source scan).
- Spec: `~/workspace/your_files/diver-harness-phase2-spec.md`
  "Supervision without polling" — the no-poll justification (Decision 3).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_130.lua` (four
  scenarios; two pass, two `where = "deadline-handle-absent"` /
  `"timer-teardown-absent"`).
- Tests: `crates/phlow-gauntlet/tests/task_130.rs` (2V/2A).
- Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no
  Phase-2 branch exists).
