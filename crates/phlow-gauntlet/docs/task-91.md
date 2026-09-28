# task-91: propose→approve→apply pipeline

**Kind:** rust · **Status:** fail (open) · **Wave:** 91–95 · **Commits:** pending (wave 91-95)

## ELI5

Imagine a suggestion box with a lock that needs two keys to open — one held by the person who suggests an improvement, one held by a trusted supervisor. The supervisor's key only works for the *specific* suggestion they reviewed: the key has the suggestion's fingerprint etched into it. Phlow's pipeline has the two-key lock and checks both keys are genuine — but nobody ever compares the fingerprint on the key to the suggestion in the box. A supervisor's key for suggestion X will happily open the box for unrelated suggestion Y. The "apply" step — actually making the change — is deliberately left to a human merging by hand; the machine never rewrites the code itself.

## What this task attempts

- **Goal:** verify the propose→approve→apply pipeline — a genuine dual-signed approval promotes a proposal, replayed approvals are rejected, no approval-less path exists, and the approval is bound to the proposal it reviewed — or document the gap with mechanism evidence.
- **Mechanism:** `src/tasks/task_91.rs` drives the REAL `phlow-experiment` types with deterministic Ed25519 + ML-DSA-65 fixtures (copied conceptually from task-11): a genuine dual-signed operator record promotes a well-formed proposal (V1); empty/garbage records are rejected at parse and the gate takes the approval by value — no `Option`, no default (V2); a genuine approval for candidate digest X is used to promote unrelated proposal Y (A1); the same approval id is used twice (A2).
- **Success criterion:** the pipeline verified end to end, or the binding gap documented with mechanism evidence.
- **Non-goals:** adding proposal-content hashing on gauntlet authority (product decision — banked, never implemented here).

## What happened

Honest FAIL at `where = "approval_candidate_mismatch"`, first attempt — the seam is REAL but incomplete:

- **V1:** a genuine dual-signed approval from an enrolled operator promotes a well-formed proposal — the pipeline works end to end; the record links proposal (baseline/rollback revisions), approval (id, operator), and candidate digest. Apply is human-driven merge by design, so no applied-tree hash exists in-band.
- **V2:** no approval-less path — the gate takes `approval: HumanApproval` by value; the type system enforces it, stronger than a runtime check. Empty and garbage records are rejected at parse; model output cannot become an approval.
- **A1:** the gap — a genuine approval for digest X promotes unrelated proposal Y. The gate copies `approval.candidate_digest()` into the record but never compares it to the proposal; `ImprovementProposal` has no digest field at all. `ExperimentError` has no `approval_mismatch` variant.
- **A2:** replay rejected — the second use of the same approval id fails with `ApprovalReplayed`. By-value consumption plus the replay store close both the token-reuse and the re-parse holes.

## Full technical depth

The driver builds real `OperatorRegistry` instances with generated v2 operator records (Ed25519 identity key + ML-DSA-65 approval key, deterministic from fixed seeds) and exercises `PromotionGate::promote` directly — no mocks, no stubs. The V1 case asserts the promotion succeeds and the `PromotionRecord` links all three parties; the V2 case asserts the type-level enforcement (by-value approval parameter) and parse-time rejection; the A2 case asserts `ApprovalReplayed` on the second promotion with the same id.

The A1 case is the finding: it constructs a valid dual-signed approval covering candidate digest X and promotes proposal Y (whose content is unrelated to X). The promotion *succeeds*. The gate's `promote` copies the approval's digest into the record — so the record *claims* a digest — but performs no comparison because there is nothing to compare against: `ImprovementProposal` carries no content hash or candidate digest field. This is not a missing check on an existing field; it is a missing field. Fixing it requires a product decision: add content hashing to proposals and a digest comparison in the gate (and decide what "the proposal's digest" means — of the diff? of the resulting tree?).

Distinct from task-92 (the lua-side approval→content-hash binding, which is absent entirely): this is the rust half — the approval *has* a digest, but the proposal has nothing to match it against.

Product decision banked for Matt: whether to add proposal-content hashing and approval↔proposal digest verification. Not implemented on gauntlet authority.

## Sources

- `crates/phlow-experiment/src/promotion.rs` — `ImprovementProposal`, `HumanApproval`, `PromotionGate::promote`, `PromotionRecord`, `ConsumedApprovals` (the real seam; the missing digest comparison)
- `crates/phlow-experiment/src/error.rs` — `ExperimentError` (no `approval_mismatch` variant; `ApprovalReplayed` exists)
- `crates/phlow-gauntlet/src/tasks/task_11.rs` — the deterministic Ed25519 + ML-DSA-65 fixture pattern reused here
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-91 design (Wave 91–95)
