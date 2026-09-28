# task-67: audit completeness

**Kind:** rust · **Status:** fail (open) · **Wave:** 66–70 · **Commits:** pending (wave 66-70)

## ELI5

An audit log is the security camera of an agent runtime: every stage transition, every approval, every boundary crossing should land on tape. The design asks for a mechanical cross-check — every transition in the run's state machine must have a matching event on the audit tape, and vice versa. Phlow has no tape: there is no audit event emitter in any crate. The one audit-adjacent writer (`EvaluationRecord::record_event`) is a manual opt-in recorder with zero call sites, the experiment lifecycle transitions with no event channel (transition returns only the next state), and the evaluator's stage machine likewise. The design's adversarial weapon — "the emitter errors mid-run, the run must fail closed" — has no target because there is no emitter to fail. The driver's verdict is the honest FAIL at the absent seam: the cross-check is impossible because the event log doesn't exist.

## What this task attempts

- **Goal:** verify that every transition emits an auditable event and the event log cross-checks against the transition log (adversarial: emitter failure → fail closed).
- **Mechanism:** `src/tasks/task_67.rs` is an audit-only driver (no seam to drive). It scans every phlow crate's `src` tree (live working tree, exact-token case-insensitive, phlow-gauntlet excluded) for emitter-mechanism vocabulary; drives the real `promotion::Lifecycle` through 6 transitions and the real `Evaluator` through 3 stage transitions; verifies fail-closed behavior on 6 unusual paths; checks the public API for any emitter/sink type.
- **Success criterion:** a complete emitted event trail with mechanical cross-check, or a sourced honest FAIL.
- **Non-goals:** building the emitter on gauntlet authority; inventing call sites.

## What happened

Honest FAIL at `where = "seam"`, first attempt:

- **V1:** the emitter-vocabulary scan found 0 hits across all phlow crates, and the manual `EvaluationRecord::record_event` has 0 *production* call sites (the only known invocations are the crate's own integration test and the gauntlet harness's fixture drivers — test code, not transition emission). No transition emitter exists anywhere.
- **V2:** the real lifecycle walks 6 transitions and the real evaluator walks 3 stage transitions; every transition returns only the next state — emission is impossible *by construction*, not merely unobserved. The `transitions: u32` counter counts transitions but records no events (classified adjacent, not the seam).
- **A1:** six unusual paths — rejection (`Proposed + ContractInvalid`), terminal-state rejection, expiry, rollback (the compensation path), illegal transition, budget exhaustion — all behave per contract (fail closed where designed) and emit nothing on any of them.
- **A2:** the design's "emitter errors mid-run" adversarial has no target: the public API contains no emitter/sink/audit type. You can't fail closed on an emitter failure when there is no emitter.

## Full technical depth

`Lifecycle::transition(&self, event)` returns `Result<Lifecycle, PromotionError>` — the next state, nothing else. `Evaluator::validate/prepare/execute` return `Result<(), EvalError>`. There is no sink parameter, no event return, no channel — the call signature physically cannot emit. `EvaluationRecord` is a JSONL-consumable manual recorder (`record_event` is opt-in per call site) with zero callers, so even the adjacent recorder produces no tape. The `Evaluator`'s `transitions: u32` counter is the closest thing to a transition record and it is a counter, not a log — no event log exists to cross-check against the transition log, so the design's mechanical audit is undefined.

Banked for Matt (product decision, not a bug): whether phlow should gain an audit event emitter wired into `Lifecycle::transition` and the `Evaluator` stage machine; whether emission is synchronous at the transition site or via a subscribed sink; what fail-closed means when emission fails (abort the transition? mark the run tainted?); and whether the manual `EvaluationRecord::record_event` becomes that emitter or stays a manual recorder.

## Sources

- `crates/phlow-experiment/src/promotion.rs` — `Lifecycle::transition` (no event channel)
- `crates/phlow-experiment/src/evaluator.rs` — `Evaluator` stage machine, `transitions: u32` counter
- `crates/phlow-experiment/src/lib.rs` — `EvaluationRecord::record_event` (zero call sites)
- `~/workspace/gauntlet-design-tasks-21-70.md` — task-67 design (Wave 12)
