# task-120: M.run failure-path legality

**Kind:** nvim-lua · **Status:** fail (open, diver defect) · **Wave:** 116–120 · **Commits:** pending (wave 116-120)

## ELI5

When starting a job fails, the harness writes the failure down the same
way every time: "mark it failed." But the state machine only allows
certain moves: a job that was never queued (`created`) may go to `queued`
or `cancelled` — never straight to `failed`. When the failure happens
*before* the job is queued, the "mark it failed" step attempts an illegal
move. The machine refuses, logs a confusing "invalid transition"
complaint, and the real failure reason is lost. "Correct" means: if the
failure happened before queueing, don't touch the state machine — just
report the error; if it happened after, `failed` is legal and the reason
must be preserved.

## What this task attempts

- **Goal:** prove `M.run`'s failure paths stay within legal transitions.
- **Mechanism:** diver's `ai.harness` — `init.lua M.run` (line 83),
  `supervisor.start_run` (created→queued, then launch), `supervisor.finish`
  → `transition` (`supervisor.lua`), `types.TRANSITIONS` (`types.lua`) —
  via the driver `crates/phlow-gauntlet/lua/gauntlet/task_120.lua`.
- **Success criterion:** failures after queueing finish legally with the
  reason preserved; failures before queueing emit no diagnostic and leave
  the run in `created`; `M.run` before setup gives the clear error; a
  raising adapter start stays contained.
- **Non-goals:** changing the transition table. `created = { queued,
  cancelled }` is the contract; the driver tests paths against it.

## What happened

Fail, open diver defect (Fix 6 absent) — on the first attempt. The
failure-path fork:

- Unknown adapter: `start_run` transitions created→queued, then `launch`
  fails with "unknown adapter" — `M.run` calls `finish(..., 'failed')` on a
  queued run, which is legal. Reason preserved, no spurious diagnostic.
  Passes today.
- Adapter start returns an error after queueing: same legal path, reason
  preserved on `run.finished`. Passes today.
- Pre-queued failure (simulated exactly as `M.run` does — `finish` while
  the run is still `created`): the illegal created→failed transition is
  attempted, a spurious `diagnostic.observed` (kind
  `invalid_transition`) is emitted, and the intended outcome is lost.
  `where = "fix-6-absent"`.
- Not-set-up: `M.run` before `setup()` returns the clear "harness not set
  up" error (passes today), but a *raising* adapter start propagates out
  of `harness.run` uncaught — `launch` calls `chosen.start` with no pcall
  (supervisor.lua lines 182-224) — leaving the run stranded in `queued`
  instead of landing it in `failed`. `where = "fix-6-absent"` (containment
  half).

Note: the real diagnostic event kind is `diagnostic.observed` with
`payload.kind = "invalid_transition"` (from `transition()`), not a
`diagnostic.invalid_transition` kind — the driver matches the real shape.

## The fix — what changed and why

No fix — this is a documented diver finding (Phase-2 Fix 6), and diver
findings are never fixed under gauntlet authority. The "fix" for the
gauntlet side was getting the evidence right:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_120.lua` (new) —
  four scenarios separating the legal post-queued path from the illegal
  pre-queued path, plus the not-set-up error and the raising-start
  containment probe (caught with `pcall` so the driver never crashes).
- **Why:** the defect only bites when the failure predates the queued
  transition; lumping all failures together would hide the legal half and
  the illegal half alike.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/init.lua` line 83
  (`M.run` — unconditional `finish(..., 'failed')`),
  `~/workspace/repos/diver/lua/ai/harness/supervisor.lua` (`start_run`:
  created→queued then launch; `transition`: emits `diagnostic.observed`),
  `~/workspace/repos/diver/lua/ai/harness/types.lua`
  (`TRANSITIONS.created = { queued, cancelled }`).
- **Validation agents:** the 2 validation tests
  (`unknown_adapter_after_queued_is_legal`,
  `queued_start_failure_preserves_reason`) assert the legal path keeps
  working.
- **Adversarial agents:** the 2 adversarial tests
  (`pre_queued_failure_is_illegal_today`,
  `not_set_up_clear_error_but_raising_start_uncontained`) prove the
  illegal transition and the uncontained raise.

## Full technical depth

`M.run` does: `create` → `start_run` (which transitions created→queued,
then calls `launch`) → on any start failure, `finish(run, 'failed',
'invalid_adapter: ' .. err)`. When the failure occurs inside `launch`
*after* the queued transition, `finish` attempts queued→failed — legal,
`run.finished` carries the reason. When the failure occurs *before* the
queued transition (create succeeded but queueing never ran — e.g. a
future validation hook, or a caller invoking the finish path early as the
driver simulates), `finish` attempts created→failed: `can_transition`
fails, `transition` appends `diagnostic.observed { kind =
'invalid_transition', from = 'created', to = 'failed' }`, returns the
error — and the run's intended failure outcome never lands.

The containment half is a separate gap on the same theme: `launch` calls
`chosen.start(run, sup.sink)` with no pcall, and neither `start_run` nor
`M.run` wraps it, so a raising adapter start propagates to the caller of
`harness.run` with the run stranded in `queued` (no timeout has fired
yet, no failure recorded).

Phase-2 acceptance (banked, diver-owned): `finish` is legal when the
failure happened after queueing (reason preserved); no diagnostic is
emitted and no illegal transition attempted when the failure predates the
queued transition; `M.run` contains raising adapter starts and finishes
the run legally with the reason preserved.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/init.lua` line 83
  (`M.run` — unconditional `finish(..., 'failed')` on start failure).
- Primary: `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  (`start_run`: created→queued then `launch`; `launch` lines 182-224: no
  pcall around `chosen.start`; `transition`: `diagnostic.observed` on
  illegal moves).
- Primary: `~/workspace/repos/diver/lua/ai/harness/types.lua`
  (`TRANSITIONS.created` permits only queued/cancelled).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_120.lua` (four
  scenarios; pre-queued + not-set-up `where = "fix-6-absent"`).
- Tests: `crates/phlow-gauntlet/tests/task_120.rs` (2V/2A).
- Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no
  Phase-2 branch exists).
