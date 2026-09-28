//! Integration tests for task-199 (verify-after-act).
//!
//! Both driver cases against the doctrine's small explicit state
//! machine (`execute_verified`): the honest file-backed backend (V)
//! and the fault-injected no-write backend that claims success (A).
//! Honest scope: phlow-cli has no mutating command with an
//! injectable backend, so the mechanism is proven at toy scale
//! rather than pretended against the real CLI.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_199;

fn check_case(case: &str) -> CaseReport {
    let report = task_199::run_case(case)
        .unwrap_or_else(|e| panic!("task-199 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-199 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V: the honest backend applies, reports changed fields, and the
/// verification line comes from a fresh re-read; a separate
/// script-level re-read confirms reported state equals actual state.
#[test]
fn honest_backend_verified() {
    assert_eq!(task_199::ID, "task-199");
    let report = check_case("honest_backend_verified");
    let m = &report.metrics;
    assert_eq!(m["exit_code"].as_i64().unwrap(), 0);
    assert_eq!(m["fields"].as_u64().unwrap(), 2);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("changed brightness=80"),
        "evidence must list the changed fields:\n{joined}"
    );
    assert!(
        joined.contains("verified: 2 fields match fresh re-read"),
        "evidence must show the verification line:\n{joined}"
    );
    assert!(
        joined.contains("script re-read of state file"),
        "evidence must show the independent script check:\n{joined}"
    );
}

/// A: the lying backend reports success but writes nothing.
/// Verification catches it: the typed `CommandError::VerifyFailed`
/// (naming the field and both values), a non-zero exit code, and no
/// success text is ever produced — a false success is never printed.
#[test]
fn lying_backend_caught() {
    let report = check_case("lying_backend_caught");
    let m = &report.metrics;
    assert_ne!(
        m["exit_code"].as_i64().unwrap(),
        0,
        "a caught lie must exit non-zero"
    );
    assert_eq!(m["error"].as_str().unwrap(), "VerifyFailed");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("reported '80' but fresh re-read found '20'"),
        "evidence must name the field and both values:\n{joined}"
    );
}
