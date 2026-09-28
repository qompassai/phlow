# task-96: cross-agent approval laundering

**Kind:** rust · **Status:** fail (open) · **Wave:** 96–100 · **Commits:** pending (wave 96-100)

## ELI5

Imagine a signed permission slip that says "you may proceed with plan X." A supervisor signed it — genuinely, no forgery — but the slip names no specific plan, no specific day, and names no specific person. The sub-agent running plan Y (a different, riskier plan) can pick up that same slip and wave it at the gate, and the gate lets it through: the slip is real, it is used only once *here*, and the person who handed it over is not the sub-agent. What the slip never says — "this is only for plan X, on this run, for this sub-agent" — the gate never checks. The approval is laundered across actions and across runs.

## What this task attempts

- **Goal:** verify that a human approval consumed by the parent supervisor cannot be replayed by a subagent to promote its own unrelated proposal — or document the gap with mechanism evidence.
- **Mechanism:** `src/tasks/task_96.rs` drives the REAL `phlow-experiment` types with deterministic Ed25519 + ML-DSA-65 fixtures (the task-11 pattern): a genuine approval presented by the parent promotes the approved action (V1); an approving operator acting as its own agent is rejected with `SelfApproval` (V2); the parent's genuine approval (covering digest X) is replayed to promote the subagent's unrelated proposal Y (A1); the same approval is forwarded into a second run with a fresh replay store (A2).
- **Success criterion:** the laundering is stopped, or the missing bindings are documented with mechanism evidence.
- **Non-goals:** changing the approval record format on gauntlet authority (product decision — banked, never implemented here).

## What happened

Honest FAIL at `where = "launder_across_actions"`, first attempt — the seam is REAL but incompletely bound:

- **V1:** the human operator's genuine approval, presented by the parent supervisor, promotes the approved action — the dual-signature enforcement, replay store, and `SelfApproval` check all work against the real types.
- **V2:** the approving operator cannot act as its own agent — `PromotionGate` rejects with `SelfApproval`, the gate's one principal check.
- **A1:** the gap — the subagent replays the parent's genuine approval (covering digest X) to promote its own unrelated proposal Y, and the promotion *succeeds*. The approval binds operator + approval id + candidate digest + scope + expiry; the gate checks the approver is enrolled, both signatures verify, the id is fresh, scope/expiry match, and the *presenting* agent is not the approver. But the approval binds no proposal *content* (the gate copies the candidate digest into the record but never compares it to the proposal — task-91's banked gap, now attacked across the delegation boundary) and the "acting agent" is only a caller-supplied string, not an authenticated principal.
- **A2:** the same approval consumed in run A is forwarded into run B with a fresh replay store and the promotion *succeeds*. The v2 record format cannot carry a run id — adding a `run_id` key to the record fails at parse — so cross-run replay protection depends on store-sharing discipline, not on the approval itself.

## Full technical depth

The driver generates deterministic v2 operator records (Ed25519 identity key + ML-DSA-65 approval key, fixed seeds — the task-11 fixture pattern) and signs approvals in-process with `ApprovalMessage::from_fields`. Each case builds its own `PromotionGate`, so nothing is mocked: the V1 case asserts the promotion succeeds with the parent's agent name; the V2 case asserts `SelfApproval` when operator == acting agent.

The A1 case is the finding: it signs a genuine approval for candidate digest X and calls `promote` with the subagent's agent name and proposal Y (unrelated content, same well-formed shape). The promotion succeeds. The gate verifies six things about the *approval* and one thing about the *presenting agent*, and none of them binds the proposal. The record then claims `candidate_digest` X — faithfully recording which approval was used, not which proposal was promoted.

The A2 case forwards the same approval to a second gate with a fresh `ConsumedApprovals`; the promotion succeeds because replay tracking lives in the store, not the approval. Parsing the record with an extra `run_id` key fails, so run-binding cannot even be expressed in the format.

The 50/50 split means the driver stops at A1: A2 is exercised by its own case and integration test, where the assertions live.

Product decisions banked for Matt: (1) proposal-content binding for approvals (already banked from task-91); (2) run-id binding on the approval record (requires a format version and a policy for what a run id is).

## Sources

- `crates/phlow-experiment/src/promotion.rs` — `HumanApproval`, `PromotionGate::promote` (the real seam; the missing proposal-content and run bindings)
- `crates/phlow-experiment/src/error.rs` — `ExperimentError::SelfApproval`, `ApprovalReplayed`
- `crates/phlow-gauntlet/src/tasks/task_11.rs` — the deterministic Ed25519 + ML-DSA-65 fixture pattern reused here
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-96 design (Wave 96–100)
