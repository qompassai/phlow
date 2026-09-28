# task-65: plan cost estimation

**Kind:** rust · **Status:** fail (open) · **Wave:** 61–65 · **Commits:** pending (wave 61-65)

## ELI5

Before starting work, a good planner says "this will cost about X" — and if X is more than the budget, it doesn't start doomed work. Phlow enforces budgets (it will kill work that overspends), but it never *estimates* cost before starting. The driver proved the gap: a plan declaring 100 tool calls against a 2-call budget sailed through validation — nothing refused it up front — and was only stopped when it actually tried to spend. Enforcement works; the estimate that would have prevented starting doesn't exist. (Distinct from tasks 02/14, which verified enforcement fails closed — this is estimation *before* enforcement.)

## What this task attempts

- **Goal:** locate the planner's cost model and verify pre-execution estimation (estimate on record before execution; estimate units match enforcement units).
- **Mechanism:** `src/tasks/task_65.rs` scans every phlow crate's `src` tree (live working tree, exact-token case-insensitive, phlow-gauntlet excluded) for `estimate`/`estimator`/`estimated`/`over_budget`/`cost_model`/`pre_execution`; drives the real `BudgetTracker` + `Evaluator` through Validate → Prepare → Execute.
- **Success criterion:** a cost estimator with unit-consistent estimates, or a sourced honest FAIL — the design explicitly allows "if none, the finding is the gap."
- **Non-goals:** inventing an estimator on gauntlet authority.

## What happened

Honest FAIL at `where = "seam"`, first attempt — and the finding IS the gap, exactly as the design allows:

- **V1:** the estimation vocabulary scan found exactly one hit workspace-wide: `footprint_estimate` in `phlow-inference/src/kv_policy.rs` — KV-cache footprint sizing for *one token*, classified UNRELATED (control sample: estimation vocabulary, not plan-cost estimation). Zero plan-cost estimator hits.
- **V2:** the real `Evaluator` walks Validate → Prepare → Execute (`EvalStage::next`) with no Estimate stage and no `estimate()` method; `execute(3, 100)` against a 10-call budget consumed correctly (7 remaining). Enforcement without estimation.
- **A1:** a doomed plan — 100 declared tool calls vs a 2-call budget — PASSED `validate()` and `prepare()` (no estimate gate refused it) and failed closed only at `consume()` inside `execute()` (`BudgetExhausted`). The system starts doomed work and relies on enforcement to kill it.
- **A2:** the estimate/enforcement unit-match check is vacuous — enforced units are `tool_calls` (count), `output_bytes` (bytes), `deadline_ms` (ms) per `BudgetTracker::new`, `consume` speaks the same units, and there is no estimate whose units could mismatch. The classic mismatch (estimate in tokens, enforcement in calls) can't be ruled in or out.

## Full technical depth

`BudgetTracker::new(tool_calls_max, output_bytes_max, deadline_ms)` rejects zero inputs; `consume` fails closed on passed deadline (`DeadlineExceeded`) and over-consumption (`BudgetExhausted`) with checked arithmetic. `Evaluator::execute(tool_calls, output_bytes)` consumes the *declared* cost then advances the stage — the skeleton trusts the declared cost, which is precisely where a pre-execution estimate would plug in. The design's adversarial scenarios map to the gap directly: "estimate exceeds budget → planner refuses to emit the plan" is impossible with no estimator (the doomed plan is emitted and started); "actual cost diverges mid-run → re-estimation triggers replan-or-abort per a named policy" needs both an estimator and the task-62 replan seam (also absent).

Banked for Matt (product decision, not a bug): whether phlow wants pre-execution cost estimation at all; its units (tool calls? output bytes? ms?); over-budget policy (refuse vs flag `over_budget`); and the mid-run estimate-vs-actual divergence policy (replan-or-abort).

## Sources

- `crates/phlow-experiment/src/evaluator.rs` — `BudgetTracker`, `consume`, `Evaluator`, `EvalStage`
- `crates/phlow-inference/src/kv_policy.rs` — `footprint_estimate` (KV-cache control sample)
- `~/workspace/gauntlet-design-tasks-21-70.md` — task-65 design (Wave 12)
