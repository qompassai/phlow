//! Integration tests for task-84 (provider outage failover).
//!
//! The seam is ABSENT: there is no provider router in the workspace
//! — exact-token scans for `failover` / `fail_over` over
//! crates/*/src/**/*.rs find zero hits. The client is
//! single-provider by construction: OllamaBackend holds exactly one
//! base_url and OllamaConfig has no secondary field. A scripted
//! outage fails closed (error propagates, is_available() false)
//! after exactly 1 recorded HTTP request — no reroute, no logged
//! failover decision, nothing to log with cause. The design's
//! local-only invariant holds vacuously at the HTTP layer (0 cloud
//! requests: one loopback URL, nothing to discover), but the routing
//! POLICY the design asks to verify — workload marked local-only,
//! router refuses cloud failover — has no implementation.
//!
//! Whether phlow should gain multi-provider routing with a failover
//! policy (pre-approved secondaries, logged decisions, local-only
//! data-boundary enforcement) is a product decision for Matt —
//! banked, not implemented on gauntlet authority.
//!
//! Four cases — 2 validation, 2 adversarial — each self-checking:
//! cases probe the seam and record measured mechanism evidence; the
//! driver then reports the honest seam failure.

use phlow_gauntlet::tasks::task_84;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

fn ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-84: cannot build Ctx: {e}"))
}

// --- validation ---

/// V1: single-provider architecture — the backend holds exactly one
/// base_url and OllamaConfig has no secondary field. The task-level
/// driver then runs all four cases and reports the honest seam
/// failure: the failover-policy product decision is banked in the
/// task-level `how`, not implemented on gauntlet authority.
#[test]
fn single_provider_architecture() {
    assert_eq!(task_84::ID, "task-84");
    assert_eq!(task_84::NAME, "provider outage failover");
    assert_eq!(task_84::KIND, TaskKind::Rust);
    assert_eq!(task_84::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_84::run_case("single_provider_architecture")
        .unwrap_or_else(|e| panic!("task-84 case failed to run: {e}"));
    assert!(
        report.passed,
        "single-provider case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["provider_urls"], 1);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no secondary_url"),
        "evidence must show the config has no secondary:\n{joined}"
    );
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` banks the failover-policy product decision for Matt.
    let (where_, how) = match task_84::run(&ctx()) {
        TaskOutcome::Fail { where_, how, .. } => (where_, how),
        TaskOutcome::Pass { evidence } => panic!(
            "task-84 passed: a failover router was invented, not found\nevidence: {evidence:?}"
        ),
    };
    assert_eq!(where_, "seam", "task-84 must fail at the seam");
    assert!(
        how.contains("product decision for Matt"),
        "the 'how' must bank the product decision: {how}"
    );
    assert!(
        how.contains("no provider router"),
        "the 'how' must name the absent router: {how}"
    );
}

/// V2: no failover vocabulary in the workspace — zero `failover` /
/// `fail_over` hits over every product crate's src.
#[test]
fn no_failover_vocabulary() {
    let report = task_84::run_case("no_failover_vocabulary")
        .unwrap_or_else(|e| panic!("task-84 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-failover-vocab case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["failover_vocabulary_hits"], 0);
}

// --- adversarial ---

/// A1: an outage is not rerouted — the scripted connection-refused
/// failure surfaces after exactly 1 recorded request; is_available()
/// is false. The outage fails closed by construction (error
/// propagates), but no failover decision is logged — there is no
/// failover machinery to log.
#[test]
fn outage_is_not_rerouted() {
    let report = task_84::run_case("outage_is_not_rerouted")
        .unwrap_or_else(|e| panic!("task-84 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-reroute case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["requests_during_outage"], 1);
    assert_eq!(report.metrics["rerouted"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("fail-closed holds by construction"),
        "evidence must show fail-closed with no failover:\n{joined}"
    );
}

/// A2: the local-only invariant holds by architecture, not by policy
/// — across the outage and availability check, the recording
/// transport saw 0 cloud requests (one loopback URL, nothing to
/// discover), but the routing policy (workload marked local-only,
/// router refuses cloud failover) has no implementation: no workload
/// marking, no router, no cloud secondary to refuse.
#[test]
fn local_only_invariant_by_architecture() {
    let report = task_84::run_case("local_only_invariant_by_architecture")
        .unwrap_or_else(|e| panic!("task-84 case failed to run: {e}"));
    assert!(
        report.passed,
        "local-only case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["cloud_requests"], 0);
    assert_eq!(report.metrics["routing_policy"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("holds vacuously"),
        "evidence must show the vacuous invariant:\n{joined}"
    );
}
