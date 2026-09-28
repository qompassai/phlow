# task-97: gate self-modification attempt

**Kind:** rust · **Status:** fail (open) · **Wave:** 96–100 · **Commits:** pending (wave 96-100)

## ELI5

Imagine a building's security desk has a rule: nobody may hand in a work order that changes the security desk itself — no rewiring the alarm, no rewriting the rulebook. The desk enforces it: try to submit "remove the rulebook" and the submission is rejected. But the rulebook's *fence list* covers the rulebook, the alarm, and the guard roster — and it does not cover the logbook where every entry and exit is written. A work order to "stop recording certain entries in the logbook" sails through, because the fence was drawn around the gate but not around its own audit trail.

## What this task attempts

- **Goal:** verify the promotion gate's protected-surface check blocks proposals that modify the gate itself — and find what the fence does not cover — or document the gap with mechanism evidence.
- **Mechanism:** `src/tasks/task_97.rs` drives the REAL `phlow-experiment` types: an ordinary proposal promotes (V1); the protected prefix set is reconned against the five documented entries (V2); a proposal modifying `src/promotion.rs` is rejected (A1); a proposal modifying `src/record.rs` — the immutable evaluation-record writer — is promoted (A2).
- **Success criterion:** the fence covers the gate and its audit trail, or the hole is documented with mechanism evidence.
- **Non-goals:** editing the protected set on gauntlet authority (product decision — banked, never implemented here).

## What happened

Honest FAIL at `where = "audit_writer_unprotected"`, first attempt — the protected set is REAL but incomplete:

- **V1:** a proposal touching ordinary code (`src/tools.rs`) promotes normally — the fence does not over-block the sanctioned path.
- **V2:** the recon confirms the protected set is exactly the five documented prefixes: `evals/holdout`, `evals/safety`, `src/evaluator.rs`, `src/promotion.rs`, `manifests/promotion.toml`. The case fails closed if the set ever changes.
- **A1:** a proposal rewriting the promotion gate itself is rejected with `ProtectedSurface` — the guard guards itself.
- **A2:** the gap — a proposal modifying `src/record.rs`, the immutable per-experiment evaluation record (the pipeline's audit log writer), promotes normally. The fence was drawn around the promotion machinery but not around the audit trail it writes.

## Full technical depth

`check_proposal_surface` is a pure prefix match over the proposal's changed files, and `PromotionGate::promote` calls it before any other promotion logic — the ordering is right (fence before approval consumption). The A1 case asserts `ExperimentError::ProtectedSurface` with the offending path named in the evidence; the V2 case asserts each of the five expected prefixes individually so a future product change to the set breaks the case loudly instead of silently shifting the fence.

The A2 case is the finding: `src/record.rs` writes the per-experiment records that make runs auditable; a proposal that alters it (the case adds `drop_entries`) promotes with a genuine dual-signed approval. There is no content-level judgment — the fence is path-prefixes only, so this is by design as written, and changing the design (which writer paths belong inside the fence) is Matt's product decision.

The 50/50 split means the driver stops at A2: V2 follows A1 in the runner's case order (the adversarial pair runs last), and A1's defeat is itself evidence that the V2-documented fence is doing its job where it covers.

Product decision banked for Matt: whether `src/record.rs` (and by extension the audit path) belongs in the protected set.

## Sources

- `crates/phlow-experiment/src/promotion.rs` — `check_proposal_surface`, the `PROTECTED_PREFIXES` set (the real seam; the missing `src/record.rs` entry)
- `crates/phlow-experiment/src/record.rs` — the per-experiment evaluation record writer (unprotected)
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-97 design (Wave 96–100)
