// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-158 (editor-socket parity and license
//! audit).
//!
//! One validation case and one adversarial case plus the wave license
//! audit: a malformed editor message is a typed error, the socket
//! stays up, the nvim peer is unaffected, and the next good message
//! reads cleanly; a 10,000-deep nesting bomb over the editor socket is
//! rejected at the same depth bound as the daemon link with the
//! socket up; every `.rs` file added in tasks 151–157 starts with the
//! maddada attribution and the source commit.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_158;

fn check_case(case: &str) -> CaseReport {
    let report = task_158::run_case(case)
        .unwrap_or_else(|e| panic!("task-158 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-158 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: malformed editor message → typed error; socket up, peer
/// unaffected, next message reads.
#[test]
fn malformed_socket_stays_up() {
    let report = check_case("malformed_socket_stays_up");
    let m = &report.metrics;
    assert_eq!(m["typed"].as_str().unwrap(), "Malformed");
    assert!(m["socket_up"].as_bool().unwrap());
    assert_eq!(m["rejected"].as_u64().unwrap(), 1);
    assert_eq!(m["received"].as_u64().unwrap(), 1);
}

/// A1: nesting bomb over the editor socket → DepthExceeded at the
/// same bound as the daemon link; socket up.
#[test]
fn nesting_bomb_socket_bounded() {
    let report = check_case("nesting_bomb_socket_bounded");
    let m = &report.metrics;
    assert_eq!(m["typed"].as_str().unwrap(), "DepthExceeded");
    assert_eq!(m["bound"].as_u64().unwrap(), 128);
    assert!(m["socket_up"].as_bool().unwrap());
    assert!(m["parity_with_daemon_link"].as_bool().unwrap());
}

/// The license audit: all 15 wave-25 files carry the attribution
/// header, and task-158's own driver + test carry it too.
#[test]
fn license_audit_all_files() {
    let report = check_case("license_audit_all_files");
    assert_eq!(report.metrics["files_checked"].as_u64().unwrap(), 15);
}
