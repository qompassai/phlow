//! Integration tests for task-111 (selection-split overfitting).
//!
//! Four driver cases — 2 validation, 2 adversarial — exercised against
//! the clearly labeled scripted double (deterministic, fast). D_test is
//! sealed during training on every arm; the seal is verified by the
//! live `d_test_reads` counter (positive control: an unsealed arm
//! counts exactly one training-time read).

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_111;

fn check_case(case: &str) -> CaseReport {
    let report = task_111::run_case(case)
        .unwrap_or_else(|e| panic!("task-111 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-111 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// The headline case carries the task verdict; it must be one of the
/// preregistered classes.
fn check_verdict(m: &serde_json::Value) {
    let verdict = m["verdict"].as_str().unwrap_or("<missing>");
    assert!(
        ["replicates", "null", "negative", "indeterminate"].contains(&verdict),
        "verdict must be a preregistered class, got {verdict}"
    );
}

// --- validation ---

/// V1: small vs large D_sel — the overfitting gap is measured with
/// D_test sealed, and the verdict classifies per the preregistered bars.
#[test]
fn small_vs_large_gap() {
    assert_eq!(task_111::ID, "task-111");
    let report = check_case("small_vs_large_gap");
    let m = &report.metrics;
    check_verdict(m);
    for arm in ["small", "large", "three_split"] {
        assert_eq!(
            m[arm]["n"].as_u64().unwrap(),
            5,
            "{arm}: at least 5 seeds required"
        );
        assert_eq!(
            m[arm]["d_test_reads"].as_u64().unwrap(),
            0,
            "{arm}: D_test seal broken"
        );
        assert!(
            m[arm]["gap"].as_f64().is_some(),
            "{arm}: gap must be reported"
        );
    }
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("task-111 verdict:"),
        "evidence must carry the verdict line"
    );
}

/// V2: three-split confirmation — every step scores D_selB and every
/// accepted step strictly improves both splits.
#[test]
fn three_split_containment() {
    let m = &check_case("three_split_containment").metrics;
    assert!(
        m["both_strict_holds"].as_bool().unwrap(),
        "every accepted step must strictly improve D_selB"
    );
    assert!(
        m["steps_with_b"].as_u64().unwrap() > 0,
        "D_selB must actually be scored"
    );
    assert_eq!(
        m["steps_with_b"].as_u64().unwrap(),
        m["steps_total"].as_u64().unwrap(),
        "every step must carry D_selB scores"
    );
}

// --- adversarial ---

/// A1: the D_test seal is a live tripwire — the sealed arms count zero
/// training-time reads and the unsealed positive control counts one.
#[test]
fn seal_holds() {
    let m = &check_case("seal_holds").metrics;
    assert_eq!(m["sealed_reads"].as_u64().unwrap(), 0);
    let unsealed = m["unsealed_reads"].as_array().cloned().unwrap_or_default();
    assert_eq!(unsealed.len(), 5, "positive control runs 5 seeds");
    for r in &unsealed {
        assert_eq!(
            r.as_u64().unwrap(),
            1,
            "unsealed arm must count its seed-setup D_test read"
        );
    }
}

/// A2: acceptance/D_test correlation — the per-edit ablation hunts for
/// accepted edits that are D_test-neutral or D_test-harmful (the
/// overfitting mechanism caught red-handed), seal-safe and post-hoc.
#[test]
fn acceptance_d_test_correlation() {
    let m = &check_case("acceptance_d_test_correlation").metrics;
    for arm in ["small", "large", "three_split"] {
        let n_edits = m[arm]["n_edits"].as_u64().unwrap();
        assert!(
            n_edits > 0,
            "{arm}: ablation must cover at least one accepted edit"
        );
        assert!(
            m[arm]["frac_nonpositive"].as_f64().is_some(),
            "{arm}: nonpositive fraction must be reported"
        );
    }
}
