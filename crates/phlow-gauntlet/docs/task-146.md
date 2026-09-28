# task-146: operator-approval boundary on submission

**Kind:** rust (adversarial) · **Status:** pass · **Wave:** 146–150 · **Commit:** pending (wave 26)

## ELI5

Imagine a launch key that only works once, and only for the exact missile it was cut for. Even if everyone agrees the launch is a good idea, nobody can turn the key twice, and nobody can use it on a different missile than the one the commander inspected. Task 146 checks that Phlow's submission gate works exactly like that: nothing submits without a live operator approval stapled to the exact report bytes, and each approval works exactly once.

## What this task attempts

- **Goal:** verify the no-auto-submit rule — `SubmissionGate::submit` refuses every submission that lacks a live, hash-bound, unspent operator approval, with a typed error per refusal.
- **Mechanism:** `src/tasks/task_146.rs` drives the real `SubmissionGate` with fixture approvals and a `ManualClock` in four scenarios: `valid_approval_and_hash_submits` (V1: live approval + matching sha256 → submitted, nonce spent exactly once); `no_approval_refused` (V2: all-green finding, `approval: None` → `GateError::NoApproval`); `hash_mismatch_refused` (A1: approval binds v1 bytes, v2 bytes submitted → `GateError::HashMismatch{expected, got}`, nonce NOT spent); `replay_nonce_refused` (A2: same approval submitted twice → second submit `GateError::ReplayNonce{nonce}`, spent set holds exactly one nonce).
- **Success criterion:** zero submissions without a live, hash-bound, unspent approval across all scenarios; each refusal typed.
- **Non-goals:** approval issuance itself (who may mint approvals — task 134's provenance seam); cross-process nonce persistence (the spent set is per-gate, in-memory).

## What happened

PASS on the exact tree, all four scenarios:

- **V1:** valid approval + matching hash → `Submission` with the right finding id, payload hash, and nonce; spent-nonce count 1.
- **V2:** no approval → `NoApproval`. The "but everything passed" case: the finding was green on every check, and the gate still refused. Greenness is not authorization.
- **A1:** hash binds v1, payload is v2 → `HashMismatch` with `expected` = the approved v1 hash and `got` = the submitted v2 hash; the spent-nonce count stayed 0, so the failed attack did not consume the capability.
- **A2:** first submission ok; replay of the same approval → `ReplayNonce{nonce: 1003}`; spent set size stayed 1.

## Full technical depth

The gate checks, in order: the finding is in `Approved` state (`FindingNotApproved`); an approval is present and issued by `operator` (`NoApproval` — note the forged-issuer case collapses to `NoApproval`, the same typed refusal as absence, which is the correct fail-closed shape); program and scope-version binding (`ScopeVersionMismatch`); liveness against the clock (`Expired`); exact payload-hash binding (`HashMismatch{expected, got}` — the operator approved *these bytes*); and single nonce use (`ReplayNonce{nonce}` — the spent set is checked *after* every other check, so a refused submission never spends the nonce, as A1 verifies).

The hash binding is what makes the approval a capability on bytes rather than a permission on intent: `sha256_hex(payload_bytes) == approved_hash` must hold exactly, so any drift between review and submit — a rebased report, a quietly raised severity (the A1 fixture) — voids the approval and forces re-review. This is the workflow-layer twin of task-92's content-hash binding idea, applied at the submission seam rather than the proposal seam.

Honest limitation: the spent-nonce set is per-`SubmissionGate` (in-memory). A second gate process would not know the nonce was spent. Single-use holds within one gate instance — the unit the workflow runs — but cross-process replay would need a shared spent set (a ledger write), which is out of scope for this task.

## Sources

- `crates/phlow-gauntlet/src/bounty/approve.rs` — `SubmissionGate::submit`, `GateError`, `sha256_hex` (the scaffold under test; read-only for this wave)
- `~/workspace/gauntlet-design-tasks-131-150.md` — task-146 design (Wave 26)
- Task-92 design precedent: hash-bound approvals (`~/workspace/gauntlet-design-tasks-71-100.md`)
