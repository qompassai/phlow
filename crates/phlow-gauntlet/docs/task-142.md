# task-142: finding validation pipeline

**Kind:** rust · **Status:** pass · **Wave:** 25 · **Commits:** pending (wave 141-145)

## ELI5

Before a finding can be written up, it has to pass four mechanical
checks: is the target actually in scope, is there real evidence
attached, is it not a duplicate of something we already know about,
and was it seen more than once (reproduced)? The checks run in order
and stop at the first failure, naming exactly which check failed — so
the rejection path always knows *why*. If a check itself breaks (say
the scope database is unreachable), the finding is *not* approved —
the pipeline fails closed, the way a broken lock should stay locked,
not swing open. And the operator can bolt on their own extra checks;
adding checks can only make the gate stricter, never weaker.

## What this task attempts

- **Goal:** prove the scaffold's `ValidationPipeline` admits a finding
  to reportability only when every check passes, names the failing
  check on any single failure, fails closed on validator errors, and
  accepts operator-registered checks without weakening.
- **Mechanism:** `crates/phlow-gauntlet/src/tasks/task_142.rs` drives
  `crate::bounty::validate::{ValidationPipeline, Check, CheckResult,
  CheckCtx}` with the scaffold's three built-ins (`InScopeCheck`,
  `EvidencePresentCheck`, `NonDuplicateCheck`) plus the driver's
  `ReproducibleCheck` (the design's fourth check, registered through
  the same extension seam) and `TitlePresentCheck` (the A2 operator
  check).
- **Success criterion:** 4/4 pass ⇒ `Ok(())` and the state machine
  admits Candidate → Validated → Reportable; each single failure
  returns `Err((check_name, Fail))` with the right name and the finding
  stays Candidate; a validator error returns `Err((name, Error))` and is
  never coerced to pass; the operator check runs, blocks, and old
  verdicts stay put.
- **Non-goals:** what happens to rejects (task 143), report rendering
  (task 144). The pipeline's check *order* (short-circuit on first
  non-pass) is the scaffold's; this task verifies the observable
  behavior, not a different order.

## What happened

All four cases passed on the first attempt against scripted fixtures
(MOCK):

- **all_checks_pass_reportable (V1):** clean fixture (in-scope target,
  non-empty evidence, unique fingerprint, `observation_count = 2`)
  validated `Ok(())` across all four named checks; the finding then
  transitioned Candidate → Validated → Reportable through the legal
  state machine.
- **each_check_blocks_with_name (V2):** four arms, each breaking
  exactly one dimension — out-of-scope target, empty evidence,
  duplicate fingerprint, single observation. Each returned
  `Err(("in-scope" | "evidence-present" | "non-duplicate" |
  "reproducible", Fail))` with the exact expected name; every fixture
  stayed Candidate.
- **validator_error_fails_closed (A1):** `CheckCtx { scope: None }`
  (scope store unreachable) → `Err(("in-scope",
  CheckResult::Error))`. The outcome is the `Error` variant, never
  coerced to pass; the finding is not reportable.
- **operator_check_extends_pipeline (A2):** `TitlePresentCheck`
  registered via `pipeline.add`; appears in `check_names()`; blocks an
  untitled finding on exactly `title-present`; lets a clean finding
  through; a previously-failing finding (empty evidence) still fails on
  `evidence-present` — extension is monotone, never weakening.

Verdict: **replicates** — the pipeline gates reportability exactly as
specified.

## The fix — what changed and why

No fix iterations: the first attempt passed.

## Scaffold notes (reported, not fixed)

The design doc lists four built-in checks ("in-scope, reproducible,
non-duplicate, evidence-present"), but the scaffold's
`ValidationPipeline::with_defaults()` ships only three — there is no
built-in reproducibility check. The driver implements
`ReproducibleCheck` locally and registers it through the
operator-extension seam, which simultaneously covers the A2 scenario.
Whether the missing built-in is an oversight or a deliberate seam for
this task is the coordinator's call; the driver's coverage is complete
either way. Also note `CheckCtx` is public in `validate.rs` but not
re-exported by `bounty/mod.rs` — the driver reaches it as
`crate::bounty::validate::CheckCtx`.

## Full technical depth

The pipeline's contract is small and total: `validate` runs each
registered check in order and returns `Ok(())` only if every check
returns `CheckResult::Pass`; the first non-`Pass` short-circuits with
`Err((check_name, result))`. The three-valued `CheckResult`
(Pass/Fail/Error) is the load-bearing design decision: `Fail` means
"the finding is bad" (with a reason string the rejection path records
verbatim in task 143), while `Error` means "the check itself broke"
(infrastructure fault). The A1 case proves the distinction is honored:
with the scope snapshot unreachable, `InScopeCheck` returns `Error`,
and the pipeline surfaces `Err(("in-scope", Error))` — it does not
coerce the error into a pass ("pass on error" would silently admit
out-of-scope findings whenever the scope store hiccups) and does not
relabel it as `Fail` (which would blame the finding for an
infrastructure fault and poison the rejection reason).

The four checks and their fixture toggles: `in-scope` fails when the
finding's target id is absent from the snapshot; `evidence-present`
fails on empty `evidence.raw`; `non-duplicate` fails when the store
already holds the fingerprint under a different id (note the guard:
same id is *not* a duplicate — that's the re-observation path task 143
exploits for stickiness); `reproducible` fails when
`observation_count < 2`, using the scaffold's own counter rather than
inventing new state. The V2 arms are constructed so each fixture passes
the three checks before the toggled one — except the out-of-scope arm,
which correctly short-circuits at the first check, demonstrating the
ordering.

Extensibility without weakening is a monotonicity property: `add`
appends to the check list, and since `validate` requires *all* checks
to pass, adding a check can only turn `Ok` into `Err`, never the
reverse. The A2 case verifies both directions of the claim that
matters: the new check actually runs (it blocks the untitled finding
and appears in `check_names()`), and no old verdict flips (the
empty-evidence fixture still fails on `evidence-present`, not on the
new check — short-circuit order preserved).

## Sources

- OWASP Testing Guide v4 — what "validated" means for a finding:
  in-scope, evidenced, reproducible. (Primary: the check semantics;
  cited by the design doc.)
- The fail-closed rule mirrors authentication-system practice (NIST SP
  800-63: a verifier that cannot complete its checks must not
  authenticate). (Primary: the A1 design rationale.)
- The scaffold itself: `crates/phlow-gauntlet/src/bounty/validate.rs`
  (`ValidationPipeline`, `Check`, `CheckResult`, `CheckCtx`),
  `crates/phlow-gauntlet/src/bounty/store.rs` (`FindingStore`,
  fingerprint-keyed dedup), `crates/phlow-gauntlet/src/bounty/types.rs`
  (the finding state machine).
- Design doc: `~/workspace/gauntlet-design-tasks-131-150.md`, Wave 25,
  task-142.
