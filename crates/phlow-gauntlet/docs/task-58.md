# task-58: dual control

**Kind:** rust · **Status:** fail (open) · **Wave:** 56–60 · **Commits:** pending (wave 56-60)

## ELI5

Some actions are so risky that one person's "yes" isn't enough —
like a bank vault that needs two different keys turned at the same
time. The rule is: for the scariest actions, TWO different people
must approve, and the second person can't just rubber-stamp the
first person's paperwork — they have to check the actual plan
themselves. This task asked phlow's promotion gate (the thing that
approves promoting an experiment to the next stage) for two-key
control — and found it takes exactly ONE key: one approval promotes,
with no second slot, no quorum, and no check that the approval
matches the plan being promoted.

## What this task attempts

- **Goal:** probe phlow's real promotion gate —
  `PromotionGate::promote` in `crates/phlow-experiment/src/promotion.rs`
  — for dual-control machinery: a two-approval quorum for high-risk
  promotions, operator-distinctness across approvals, and approval
  binding to the exact promoted parameters, per the design's pass
  criteria.
- **Mechanism:** `src/tasks/task_58.rs`, a Rust driver running four
  scenarios (2 validation, 2 adversarial) against the REAL gate with
  genuine cryptographic approval records — deterministic test-only
  Ed25519 + ML-DSA-65 keys generated fresh per scenario and loaded
  through a real TOML `OperatorRegistry` (the task-11 fixture
  pattern). The driver also writes `task-58/report.md` per scenario.
  No network calls.
- **Success criterion:** the design's dual-control pass criteria
  (two distinct validated operators for high-risk promotion;
  approval cryptographically bound to the action hash) — the driver
  reports failure at the real seam instead.
- **Non-goals:** adding a quorum API to the gate. Whether high-risk
  promotion SHOULD require two distinct operators is a product
  decision — banked for Matt, not implemented under gauntlet
  authority.

## What happened

Fail at the real `dual-control-seam` — on the first and only
attempt. `PromotionGate::promote` accepts exactly ONE `HumanApproval`:
there is no quorum parameter, no second-approval slot, and no
cross-approval operator-distinctness check. The one-approval baseline
itself works (the control): a genuine single approval promotes, and
agent self-approval is still rejected by the existing `SelfApproval`
check. The driver:

- `task_metadata_and_single_approval_control` (V): the control — a
  genuine single approval promotes, and the metadata contract is
  pinned. The requirement's baseline holds; the quorum above it does
  not exist.
- `driver_reports_fail_at_dual_control_seam` (V): two distinct
  operators each approve — both promote INDEPENDENTLY; the
  `two-distinct-operators-no-quorum` scenario records the
  requirement as unmet because there is no gate decision requiring
  both. The aggregate verdict fails at `where = "dual-control-seam"`
  with evidence naming the missing quorum parameter.
- `same_operator_twice_is_not_distinguished` (A): the same operator
  approving twice is not distinguished from two operators — there is
  no cross-approval distinctness check, so a future two-approval
  rule built on this API could be satisfied by one person twice.
- `params_change_requires_no_second_approval` (A): the approval
  record cryptographically contains `candidate_digest`, but `promote`
  never compares that digest to the proposal — changed parameters
  promote under the old approval with no second approval required.

Fail-closed: the scenarios are written against the real signature
(`promote` takes exactly one `HumanApproval`); if a quorum API ever
appears, the driver must be re-cut — it cannot silently pass.

Product decision banked for Matt (not gauntlet work): whether
high-risk promotion should require two distinct validated operators
AND bind each approval to the exact action hash (the record already
carries `candidate_digest`; the gate just never checks it).

## The fix — what changed and why

No fix — this is a documented product gap, never fixed under gauntlet
authority. The gauntlet-side work was making the failure honest:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_58.rs` (new)
  — the driver generates genuine deterministic Ed25519 + ML-DSA-65
  keys per scenario and loads them through a real TOML
  `OperatorRegistry`, so the "two distinct operators" scenario uses
  real distinct operator identities rather than string labels.
- **Why:** with string labels, the "no distinctness check" finding
  would be circular — the driver would be checking its own labels.
  Real operator identities make the absence of a cross-approval
  check a statement about the PRODUCT, and the still-working
  `SelfApproval` rejection proves the fixture is genuine.
- **Source:** `crates/phlow-experiment/src/promotion.rs`
  (`PromotionGate::promote`, `HumanApproval`, `OperatorRegistry`,
  the `SelfApproval` check).
- **Validation agents:** the 2 validation tests
  (`task_metadata_and_single_approval_control`,
  `driver_reports_fail_at_dual_control_seam`) pin the metadata and
  assert the honest seam verdict with the unmet-quorum evidence.
- **Adversarial agents:** the 2 adversarial tests
  (`same_operator_twice_is_not_distinguished`,
  `params_change_requires_no_second_approval`) document the
  distinctness gap and the unchecked `candidate_digest`.

## Full technical depth

The driver builds a fresh `OperatorRegistry` from TOML in a per-scenario
scratch dir, generates deterministic test-only Ed25519 and ML-DSA-65
keypairs (fixed seeds — reproducible, and clearly test-only), and
mints `HumanApproval` records signed over the candidate digest. The
control scenario (`single-genuine-approval-promotes`) passes:
`promote` accepts the record and the promotion proceeds. The quorum
scenario (`two-distinct-operators-no-quorum`) mints records from two
distinct operators and shows each promotes on its own — there is no
gate decision that requires both, because `promote`'s signature has
exactly one approval slot. The adversarial scenarios push at the two
natural "almost" properties: the API cannot tell one operator twice
from two operators once (no distinctness check), and the record's
`candidate_digest` — present and signed — is never compared against
the proposal being promoted, so a changed plan rides the old
approval.

What is missing for the design's dual control: a quorum parameter on
the promotion path (N distinct validated operators for high-risk
actions), a cross-approval operator-distinctness check, and
action-hash binding — compare the record's `candidate_digest`
against the proposal at promote time. The design gap is a product
decision: the gate's one-approval model may be the intended scope,
and the cost of the quorum (two humans on call, slower promotions) is
real.

## Sources

- Primary: `crates/phlow-experiment/src/promotion.rs`
  (`PromotionGate::promote` — one `HumanApproval` slot, no quorum;
  `candidate_digest` present but unchecked; `SelfApproval` rejection
  intact).
- Driver: `crates/phlow-gauntlet/src/tasks/task_58.rs` (real-gate
  scenarios, genuine crypto fixtures, per-scenario `report.md`).
- Tests: `crates/phlow-gauntlet/tests/task_58.rs` (2V/2A).
