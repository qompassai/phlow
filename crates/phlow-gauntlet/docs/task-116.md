# task-116: goal propagation

**Kind:** nvim-lua · **Status:** fail (open, diver defect) · **Wave:** 116–120 · **Commits:** pending (wave 116-120)

## ELI5

The harness asks you what you want done (`spec.goal` — "deliver the
quarterly report"), checks that you actually wrote something there, and
then... throws the note away before handing the job to the worker. The
worker (`adapters/a2a.lua`) looks at the empty slot where the note should
be (`run.goal`) and sends an empty message to the remote AI. "Correct" for
this task means: the goal you wrote survives byte-for-byte from your spec
into the run, and the adapter receives exactly what you wrote. No
filtering, no trimming, no translation — the goal is data, and data must
arrive intact.

## What this task attempts

- **Goal:** prove the validated `spec.goal` reaches the adapter intact.
- **Mechanism:** diver's `ai.harness` — `types.validate_run_spec`
  (`types.lua` line 229), `supervisor.create` (`supervisor.lua` line 116),
  the A2A adapter submit (`adapters/a2a.lua` line 61) — via the driver
  `crates/phlow-gauntlet/lua/gauntlet/task_116.lua`.
- **Success criterion:** `run.goal == spec.goal` after create, and the
  adapter's `start` captures the same string.
- **Non-goals:** filtering or sanitizing the goal. Prompt-injection text
  must pass through byte-identical — filtering belongs to adapters and
  `ai.security`, not to the run table. The correct result for an
  unfilterable seam is the evidenced defect, not a filter built in the
  driver.

## What happened

Fail, open diver defect (Fix 1 absent) — on the first attempt. The run
table drops the field:

- `types.validate_run_spec` requires `spec.goal` to be a non-empty string
  — the validation passes.
- `supervisor.create` builds the run table field-by-field (workflow,
  adapter, workspace, budget, ...) but never copies `goal` — the field
  is silently dropped.
- The A2A adapter submits with `message = run.goal` — nil — so every A2A
  run currently sends an empty message to the remote agent.

All four driver scenarios report `fail` with `where = "fix-1-absent"`:
`default` (run table has no goal after create), `launch-delivers-goal`
(stub adapter captures nil at start), `injection-pass-through`
(prompt-injection text is dropped, not filtered — no filtering layer
exists here to mutate it), `unicode-whitespace` (unicode + edge whitespace
lost with the field, so byte-identical transport is unverifiable).

## The fix — what changed and why

No fix — this is a documented diver finding (Phase-2 Fix 1), and diver
findings are never fixed under gauntlet authority. The "fix" for the
gauntlet side was getting the evidence right:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_116.lua` (new) —
  four scenarios, each ending in `fix_absent()` with file-level evidence.
- **Why:** the defect is the *drop*, not filtering; the adversarial
  scenarios pin that no filtering layer exists at this seam, so a future
  "fix" that adds filtering would itself fail the acceptance criterion.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/types.lua` line 229
  (`validate_run_spec` requires `spec.goal`),
  `~/workspace/repos/diver/lua/ai/harness/supervisor.lua` line 116
  (`M.create` builds the run table with no `goal` field),
  `~/workspace/repos/diver/lua/ai/harness/adapters/a2a.lua` line 61
  (`message = run.goal`).
- **Validation agents:** the 2 validation tests
  (`default_goal_dropped_by_create`,
  `launch_delivers_nil_to_adapter`) assert the create-time drop and the
  nil-at-start evidence.
- **Adversarial agents:** the 2 adversarial tests
  (`injection_text_dropped_not_filtered`,
  `unicode_whitespace_lost_with_field`) prove the gap is total loss, not
  sanitization, and that fidelity is unverifiable until the field exists.

## Full technical depth

`supervisor.create` validates the spec (`types.validate_run_spec` rejects
a missing/empty `goal`), then constructs the run record from individual
spec fields. `goal` is simply never assigned: `run.goal` reads nil.
Downstream, `adapters/a2a.lua` `M.start` builds the A2A submit with
`message = run.goal` — nil is passed to `ai.a2a.tasks.submit`, and the
remote agent receives an empty message. There is no error anywhere: the
validation passed, the submit succeeded, the run completed. The failure is
silent data loss at the seam between validation and execution.

Phase-2 acceptance (banked, diver-owned): `run.goal == spec.goal`
byte-identical after `create`; the adapter receives it intact at `start`;
injection text passes through unfiltered (filtering, if any, is
explicitly an adapter/`ai.security` concern with its own tests); unicode
and leading/trailing whitespace survive byte-identical.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/types.lua` line 229
  (`validate_run_spec` — `spec.goal` required, non-empty string).
- Primary: `~/workspace/repos/diver/lua/ai/harness/supervisor.lua` line 116
  (`M.create` — run table has no `goal` field).
- Primary: `~/workspace/repos/diver/lua/ai/harness/adapters/a2a.lua` line 61
  (`message = run.goal` — nil today).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_116.lua` (four
  scenarios, all `where = "fix-1-absent"`).
- Tests: `crates/phlow-gauntlet/tests/task_116.rs` (2V/2A).
- Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no
  Phase-2 branch exists).
