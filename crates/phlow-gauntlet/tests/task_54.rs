//! tests/task_54.rs — gossip convergence (task-54, rust).
//!
//! Four integration tests, 50/50 validation/adversarial. The task
//! verdict is `fail` at `"seam"`: no cluster membership / config
//! dissemination path exists in any phlow crate — the gossip and
//! membership vocabulary scans return zero, and driving the real
//! `phlow_config::load_config` twice shows each load is independent
//! and per-process: no rounds, no peers, no convergence.
//!
//! - V1 `no_gossip_vocabulary`: the gossip vocabulary scan returns
//!   zero — the design's "locate; document if absent" step.
//! - V2 `config_does_not_disseminate`: two real config loads keep
//!   their own values; no rounds, no convergence protocol.
//! - A1 `no_membership_primitive`: the membership vocabulary scan
//!   returns zero — partition/rejoin scenarios are vacuous.
//! - A2 `conflicting_updates_never_converge`: two conflicting loads
//!   disagree forever with nothing to converge them — no LWW
//!   tiebreak, no liveness argument.

use phlow_gauntlet::tasks::task_54;
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;

/// Build a `Ctx` from the live crate directory, like the other gauntlet
/// tests do.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-54-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: rust task, no nvim involved"),
        PathBuf::from("unused: rust task, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

#[test]
fn no_gossip_vocabulary() {
    assert_eq!(task_54::ID, "task-54");
    assert_eq!(task_54::NAME, "gossip convergence");
    assert_eq!(task_54::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_54::run_case("no_gossip_vocabulary").expect("V1 case must run to completion");
    assert!(report.passed, "V1 failed: {:?}", report.failures);
    assert_eq!(report.metrics["gossip_seam_hits"], serde_json::json!(0));
}

#[test]
fn config_does_not_disseminate() {
    let report =
        task_54::run_case("config_does_not_disseminate").expect("V2 case must run to completion");
    assert!(report.passed, "V2 failed: {:?}", report.failures);
    assert_eq!(report.metrics["converged"], serde_json::json!(false));
    assert_eq!(report.metrics["rounds"], serde_json::json!(0));
}

#[test]
fn no_membership_primitive() {
    let report =
        task_54::run_case("no_membership_primitive").expect("A1 case must run to completion");
    assert!(report.passed, "A1 failed: {:?}", report.failures);
    assert_eq!(report.metrics["membership_seam_hits"], serde_json::json!(0));
}

#[test]
fn conflicting_updates_never_converge() {
    let report = task_54::run_case("conflicting_updates_never_converge")
        .expect("A2 case must run to completion");
    assert!(report.passed, "A2 failed: {:?}", report.failures);
    assert_eq!(report.metrics["converged"], serde_json::json!(false));
    assert_eq!(
        report.metrics["tiebreak_protocol"],
        serde_json::json!(false)
    );

    // The task-level verdict is the honest fail at the absent seam.
    let outcome = task_54::run(&test_ctx());
    match outcome {
        TaskOutcome::Fail { where_, how, .. } => {
            assert_eq!(where_, "seam", "task-54 must fail at the absent seam");
            assert!(
                how.contains("no cluster membership / config dissemination path exists"),
                "the 'how' must name the absent seam: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-54 passed: gossip machinery was invented, not found\nevidence: {evidence:?}"
        ),
    }
}
