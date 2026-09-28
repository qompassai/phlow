//! Integration tests for task-58 (dual control).
//!
//! 50/50 split: 2 validation, 2 adversarial. Every test drives the REAL
//! phlow-experiment APIs (`HumanApproval`, `PromotionGate` with the
//! 7-argument call-site shape, `OperatorRegistry`, `ManualClock`) — no
//! mocks, no test doubles. Fixtures are genuine v2 operator records:
//! deterministic Ed25519 + ML-DSA-65 keypairs (fixed test seeds, never
//! real key material) dual-sign the records in the driver itself, and a
//! real TOML registry pins the keys.
//!
//! Honest result: the dual-control requirement is UNMET —
//! `PromotionGate::promote` takes exactly one `HumanApproval`, with no
//! quorum parameter and no cross-approval distinctness. The tests pin
//! the `fail` verdict at the seam, not a pass.

use phlow_gauntlet::tasks::task_58;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// A `Ctx` whose work dir is fresh under the system temp dir. The nvim
/// paths are unused by this Rust-kind task; `Ctx::new` only rejects empty
/// paths, so placeholder values are fine.
fn ctx_for(test: &str) -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-58-{test}-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("/nonexistent/nvim"),
        PathBuf::from("/nonexistent/diver-lua"),
        work_dir,
    )
    .unwrap_or_else(|error| panic!("task-58: cannot build Ctx: {error}"))
}

// --- validation ---

/// V: metadata contract pins the task; the control scenario holds — a
/// single genuine operator approval promotes. This is the gate's real
/// one-approval contract, the baseline dual control would strengthen.
#[test]
fn task_metadata_and_single_approval_control() {
    assert_eq!(task_58::ID, "task-58");
    assert_eq!(task_58::NAME, "dual control");
    assert_eq!(task_58::KIND, TaskKind::Rust);
    let scenario = task_58::scenario_single_genuine_approval_promotes();
    assert!(scenario.validation, "V1 must be a validation scenario");
    assert!(
        scenario.requirement_met,
        "the one-approval control must hold: {:?}",
        scenario.evidence
    );
    let joined = scenario.evidence.join("\n");
    assert!(
        joined.contains("one approval per promotion"),
        "evidence must state the gate's contract:\n{joined}"
    );
}

/// V: the driver aggregates honestly — the dual-control requirement is
/// unmet, so it reports `fail` at the seam, naming the three unmet
/// scenarios, with mechanism-level evidence, and writes its report
/// under the task work dir.
#[test]
fn driver_reports_fail_at_dual_control_seam() {
    let ctx = ctx_for("driver");
    let outcome = task_58::run(&ctx);
    match &outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(where_, "dual-control-seam");
            for missing in [
                "two-distinct-operators-no-quorum",
                "same-operator-twice-not-distinguished",
                "params-change-needs-no-second-approval",
            ] {
                assert!(
                    how.contains(missing),
                    "the 'how' must name the unmet scenario '{missing}': {how}"
                );
            }
            let joined = evidence.join("\n");
            assert!(
                joined.contains("no quorum parameter"),
                "evidence must name the missing quorum API:\n{joined}"
            );
            assert!(
                joined.contains("never verified against the promoted proposal"),
                "evidence must name the unverified candidate binding:\n{joined}"
            );
        }
        TaskOutcome::Pass { evidence } => panic!(
            "driver reported pass: dual control was invented, not found\nevidence: {evidence:?}"
        ),
    }
    let report = ctx.work_dir.join("task-58").join("report.md");
    assert!(
        report.is_file(),
        "driver must write its report, missing: {}",
        report.display()
    );
}

// --- adversarial ---

/// A: the same operator approves twice (two genuine records, distinct
/// ids) — both promote independently. The design's adversarial scenario
/// (reject on identity distinctness) is unassertable: the gate never
/// holds two approvals at once, so no distinctness check exists.
#[test]
fn same_operator_twice_is_not_distinguished() {
    let scenario = task_58::scenario_same_operator_twice_not_distinguished();
    assert!(!scenario.validation, "A1 must be an adversarial scenario");
    assert!(
        !scenario.requirement_met,
        "the distinctness requirement must be unmet (honest fail): {:?}",
        scenario.evidence
    );
    let joined = scenario.evidence.join("\n");
    assert!(
        joined.contains("BOTH promote independently"),
        "evidence must show both promotions succeeded:\n{joined}"
    );
    assert!(
        joined.contains("never compared operator identities"),
        "evidence must name the missing cross-approval check:\n{joined}"
    );
}

/// A: the action's parameters change after an approval — a proposal with
/// a different baseline, rollback target, and changed surface promotes
/// with an equivalent approval. The record's candidate-digest binding is
/// signed but never verified against the proposal, so no second
/// approval is required.
#[test]
fn params_change_requires_no_second_approval() {
    let scenario = task_58::scenario_params_change_needs_no_second_approval();
    assert!(!scenario.validation, "A2 must be an adversarial scenario");
    assert!(
        !scenario.requirement_met,
        "the second-approval requirement must be unmet (honest fail): {:?}",
        scenario.evidence
    );
    let joined = scenario.evidence.join("\n");
    assert!(
        joined.contains("candidate digest"),
        "evidence must show the binding exists in the record:\n{joined}"
    );
    assert!(
        joined.contains("87c182d -> de4db33f"),
        "evidence must show the changed parameters:\n{joined}"
    );
}
