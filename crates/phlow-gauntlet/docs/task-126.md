# task-126: sink-append wakes supervision, no poll

**Kind:** nvim-lua · **Status:** partial (open, diver gap: no sink wake hook, no wake coalescing) · **Wave:** 126–130 · **Commits:** pending (wave 126-130)

## ELI5

Right now the harness's to-do list only gets checked when someone
explicitly asks "anything new?" — that's the `tick()` call. The Phase-2
plan is to make the to-do list shout instead: every time a new event is
written into the log (the "sink"), the supervisor should wake up on its
own and do its round. No shouting mechanism exists today, so this task
proves two things: nothing is secretly polling in the background (good —
that's the design's core promise), and nothing wakes up on new events
either (the gap — that's what Phase 2 must build).

## What this task attempts

- **Goal:** prove the no-poll half of Decision 3 holds today (zero
  repeating uv timers after setup — a regression guard), characterize
  `tick()` as the sole supervision driver, and record precisely that
  the sink has no `on_append` hook and the supervisor has no wake — or
  record that the surface shipped.
- **Mechanism:** diver's `ai.harness` — `events.new_sink` / `append`,
  `supervisor.tick`, `drain_completions` — via the driver
  `crates/phlow-gauntlet/lua/gauntlet/task_126.lua`.
- **Success criterion:** zero `vim.uv` repeating timers after setup; a
  `model.completed` appended straight to the sink leaves the run live
  until an explicit `tick()` drains it; the gap records pin the exact
  missing surface (`sink:on_append`, `supervisor.wake`, `waking` flag)
  with file/line evidence.
- **Non-goals:** what the wake triggers (tasks 127–129 cover the
  one-shots); the command UX (tasks 123–125); policy (tasks 118, 121–122).
  This task is the wake mechanism itself.

## What happened

Partial — two scenarios pass today; two record the gap:

- `no-repeating-timers` passes: `vim.uv.walk` after `harness.setup({})`
  finds zero repeating timers (`repeating=0`). The regression guard is
  live: a future periodic timer fails this test by construction.
- `tick-drives-completions` passes: appending `model.completed` directly
  to the sink leaves the run `running`; an explicit `tick()` then drains
  it to `completed`. `tick()` is the sole supervision driver today —
  and the body Phase-2 `wake` will invoke.
- `wake-hook-absent` fails with `where = "sink-wake-hook-absent"`:
  `sink.on_append == nil` and `supervisor.wake == nil`;
  `events.lua` `sink:append` (lines 108–117) is a bare table insert with
  no subscriber invocation. The record pins the acceptance: `sink:on_append(fn)`
  invoking subscribers in `pcall` on every append; supervisor registers
  its wake at setup; wake runs the `tick()` body.
- `wake-coalescing-absent` fails with `where = "wake-coalescing-absent"`:
  1000 rapid appends (10 runs × 99 harmless `model.stream_delta` + one
  `model.completed` each; RUNS_MAX is 256, so 1000 runs are impossible)
  cause zero supervision passes (sampled runs stay `running`); one
  explicit `tick()` then drains all 10 with exactly one `run.finished`
  each and a second `tick()` acts on nothing. The drain pass itself is
  storm-safe (single synchronous pass, no recursion) — only the wake
  trigger is missing. The record banks the ≤3-wake-passes per 1000
  appends criterion.

## The fix — what changed and why

No fix — the wake surface is diver-owned (Phase-2 Decision 3), and diver
findings are never fixed under gauntlet authority; the no-poll property
already holds and needed no change. The gauntlet-side work was getting
the evidence right:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_126.lua` (new) —
  four scenarios: the repeating-timer regression guard, the tick-driver
  characterization, and the two gap records.
- **Why:** this is the event-driven core — the only task testing the wake
  mechanism itself, as opposed to what the wake triggers. Proving the
  no-poll half holds today means Phase 2 only has to add the wake, not
  remove a poller.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/events.lua`
  `sink:append` (lines 108–117 — bare insert, no hook),
  `supervisor.lua` `drain_completions` (line 436) and `M.tick` (line 468),
  zero `vim.uv` timer creations anywhere in `lua/ai/harness/`.
- **Validation agents:** the 2 validation tests (`wake_hook_gap`,
  `wake_coalescing_gap`) assert the gap records and pin both contracts.
- **Adversarial agents:** the 2 adversarial tests
  (`no_repeating_timers_pass`, `tick_drives_completions_pass`) assert the
  no-poll guard and the tick-only drain.

## Full technical depth

`events.new_sink()` builds a closure over `stored` with three methods;
`append` validates the envelope and inserts — no hook table, no
subscriber iteration (events.lua:108–117). `supervisor.tick` (line 468)
is the only caller of `drain_completions` (line 436), which finishes runs
on `model.completed` events with outcome in
`completed`/`failed`/`cancelled` (lines 441–447) and advances
`sup.last_seq` so a second tick is a no-op. Nothing in the harness
creates a `vim.uv` timer (verified by live `vim.uv.walk` handle census
and by source grep: zero matches for timer creation in
`lua/ai/harness/`), so the "no periodic timer exists" half of Decision 3
is already true. The missing half: `sink:on_append(fn)` with `pcall`
invocation per append, `supervisor.wake` registered at setup running the
tick body, and the `waking` reentrancy flag so N rapid appends cause at
most one in-flight pass plus one trailing pass.

Phase-2 acceptance (banked, diver-owned): `sink:on_append(fn)` exists
and invokes subscribers in `pcall` on every append; the supervisor
registers its wake at setup; wake runs the tick body; the `waking` flag
coalesces storms (≤3 wake passes per 1000 appends); nested append during
a wake pass does not recurse; every run finishes exactly once; zero
repeating timers still (the regression guard keeps passing).

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/events.lua`
  `sink:append` (lines 108–117 — no subscriber hook).
- Primary: `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  `drain_completions` (line 436), `M.tick` (line 468); no `M.wake`, no
  `waking` flag, no `vim.uv` timer creation anywhere in the harness.
- Spec: `~/workspace/your_files/diver-harness-phase2-spec.md`
  "Supervision without polling" (Decision 3).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_126.lua` (four
  scenarios; two pass, two `where = "sink-wake-hook-absent"` /
  `"wake-coalescing-absent"`).
- Tests: `crates/phlow-gauntlet/tests/task_126.rs` (2V/2A).
- Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no
  Phase-2 branch exists).
