// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-195 (skill supply-chain refusal).
//!
//! Three adversarial cases against hostile canonical-dir fixtures:
//! a `../../evil.sh` entry is refused with `SyncError::Traversal`
//! with zero writes and nothing outside the target root; a symlink
//! pointing outside the target root is never followed (refused per
//! the declared contract); a file with the executable bit set syncs
//! with the bit stripped (mode 0o644, zero exec bits).

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_195;

fn check_case(case: &str) -> CaseReport {
    let report = task_195::run_case(case)
        .unwrap_or_else(|e| panic!("task-195 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-195 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- adversarial ---

/// A1: `../../evil.sh` → SyncError::Traversal, zero fs mutations,
/// outside-check dir clean, no backup dir created.
#[test]
fn traversal_refused() {
    assert_eq!(task_195::ID, "task-195");
    let report = check_case("traversal_refused");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("SyncError::Traversal"),
        "evidence must name the typed refusal:\n{joined}"
    );
    assert!(
        joined.contains("(bar: 0)"),
        "evidence must state the zero-write bar:\n{joined}"
    );
}

/// A2: symlink to a file outside the target root → SyncError::SymlinkRefused;
/// the link is not followed, not copied, and the secret bytes never
/// enter the target tree.
#[test]
fn symlink_not_followed() {
    let report = check_case("symlink_not_followed");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("SyncError::SymlinkRefused"),
        "evidence must name the typed refusal:\n{joined}"
    );
    assert!(
        joined.contains("not followed"),
        "evidence must state the scan did not follow the link:\n{joined}"
    );
}

/// A3: 0o755 canonical file → target lands at exactly 0o644 with
/// identical content.
#[test]
fn exec_bit_stripped() {
    let report = check_case("exec_bit_stripped");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("mode: 644"),
        "evidence must show the stripped mode:\n{joined}"
    );
}
