# Phlow Experimental Self-Improvement

> **Status: EXPERIMENTAL — scaffolding only. No behavior enabled.**
>
> This page describes the staged experiment's contract. The scaffolding
> lives in `crates/phlow-experiment`; it defines types, manifests,
> fixtures, and pure-logic invariants. It adds no runtime concurrency, no
> scheduler execution, no self-editing behavior, and changes no existing
> crate. `PromptEvolver::evolve()` in `phlow-self-improve` remains
> disabled.

## One-page contract

Phlow may eventually propose improvements to its own code, but it must
never approve, merge, deploy, alter its evaluator, expand its permissions,
or erase its audit trail by itself. The experiment tests whether bounded
orchestration plus external promotion control can produce safe,
measurable improvement — the hypothesis fails if added agency reduces
security, determinism, task completion, maintainability, or operator
control.

Priority is always **safety > performance > developer experience**.

## Hard constraints

1. **No autonomous self-modification.** An `ImprovementProposal` is inert
   data. It becomes a candidate only through the promotion gate.
2. **No self-approval, self-promotion, or gate removal.** The approval
   token (`HumanApproval`) is opaque and constructible only from a
   shape-validated operator record — never from model output. Promotion
   consumes it by value: one token, one promotion.
3. **Promotion gates stay human.** The lifecycle
   (`Proposed → Isolated → Tested → Reviewed → AwaitingHuman → Promoted → Monitored`,
   with `Rejected`/`RolledBack` terminals) reaches `Promoted` only on an
   explicit `HumanApproved` event.
4. **Read-only default.** No worker role can write production state —
   `WorkerRole::can_write_production()` is false for all eight roles by
   assertion, not configuration. Delegation passes a *strict* subset of
   the parent's capabilities; escalation is denied.
5. **Fail closed.** Missing evidence, passed deadlines, exhausted budgets,
   stale generations, duplicate results, and incomplete records are typed
   errors or explicit `false` — never inferred success. A cancelled, stale,
   timed-out, or superseded result never publishes.
6. **Evaluator independence.** The candidate cannot control its benchmark
   inputs, hidden tests, the evaluator, promotion thresholds, check argv,
   policies, artifact storage, or signing material. Holdout and safety
   manifests are not candidate-writable; safety cases are immutable.

## What's in the crate

| Area | Module | Contents |
|---|---|---|
| Control plane | `control_plane` | `NodeState` (one exhaustive enum), `WorkerRole`/`WriteAccess`, `CapabilitySet` with strict-subset delegation, `SchedulerNode` with all plan-required fields, `SchedulerLimits`, and the pure-logic `Scheduler` (bounded admission, generation-checked at-most-once publication, terminal cancellation) |
| Promotion | `promotion` | `Lifecycle` + `LifecycleEvent` transitions, opaque `HumanApproval`, `ImprovementProposal`, protected-surface policy, `PromotionGate` |
| Evaluator | `evaluator` | `EvalStage` order, `BudgetTracker` (fail-closed, checked arithmetic), `EvidenceBundle` with `is_complete()`, `Evaluator` skeleton |
| Record | `record` | `EvaluationRecord` matching the plan's JSON schema; `verified`/`eligible`/`human_approved` start false and move only on complete evidence or a real approval token |
| Manifests | `manifest` | `toml::Value` + explicit validation for `manifests/*.toml` and the task-manifest contract |

Data: `crates/phlow-experiment/manifests/` (suites, language tiers A–E,
budget defaults, promotion thresholds), `evals/` (public / holdout /
safety / regressions split contracts), `fixtures/` (apps, attacks, mcp,
failures).

## Current phase

**Phase 0 (scaffolding):** this crate. No trial runs yet. The first valid
result from the plan is a *baseline measurement*, not a self-modification.

## Pointers

- Trial protocol: `~/workspace/user/files/phlow-experimental-plan_5_euo1.md`
- Safety constraints: `~/workspace/user/files/phlow-experiment_4_vvx3.md`
- Crate: `crates/phlow-experiment` (`src/`, `manifests/`, `evals/`, `fixtures/`, `tests/`)
