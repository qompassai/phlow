//! Task 114 integration tests: 2 validation + 2 adversarial.
//!
//! Validation: protected-region attacks are 100% typed-rejected; the
//! gated slow update costs ≤ 2.0 points on clean data and blocks
//! poisoned writes. Adversarial: the poisoned slow update's D_test
//! damage is measured (verdict follows the bar); the meta flip is
//! caught by the checksum (fail closed) and shown to skew proposals
//! without it.

use phlow_gauntlet::tasks::task_114::{self, GATE_COST_PTS_MAX, POISON_DAMAGE_PTS};

/// V1: protected-region attacks are 100% typed-rejected.
#[test]
fn v_protected_region_rejection() {
    let r = task_114::run_case("protected_region_rejection").unwrap();
    assert!(r.passed, "failures: {:?}", r.failures);
    assert_eq!(r.metrics["n_rejected"], r.metrics["n_attacks"]);
    assert_eq!(r.metrics["rejection_rate_pct"], 100.0);
}

/// V2: gated slow update is cheap on clean data and blocks poison.
#[test]
fn v_gated_slow_update_cost() {
    let r = task_114::run_case("gated_slow_update_cost").unwrap();
    assert!(r.passed, "failures: {:?}", r.failures);
    let cost = r.metrics["gate_cost_pts"].as_f64().unwrap();
    assert!(
        cost <= GATE_COST_PTS_MAX,
        "gate cost {cost} > {GATE_COST_PTS_MAX}"
    );
    // No false positives on clean data.
    assert_eq!(r.metrics["gate_blocks"], 0);
    // Blocks the poison.
    assert!(
        r.metrics["poisoned_gate_blocks"].as_u64().unwrap() > 0,
        "gate blocked zero poisoned writes"
    );
}

/// A1: poisoned slow update damage is measured; the verdict follows the
/// preregistered bar (negative iff damage ≥ 3.0).
#[test]
fn a_poisoned_slow_update_impact() {
    let r = task_114::run_case("poisoned_slow_update_impact").unwrap();
    assert!(r.passed, "failures: {:?}", r.failures);
    let damage = r.metrics["damage_pts"].as_f64().unwrap();
    let verdict = r.metrics["verdict"].as_str().unwrap();
    if damage >= POISON_DAMAGE_PTS {
        assert_eq!(verdict, "Negative");
    } else {
        assert_eq!(verdict, "Null");
    }
    // The verdict is reported honestly, never forced.
    assert!(
        r.evidence.iter().any(|e| e.contains("verdict:")),
        "no verdict line in evidence"
    );
}

/// A2: meta flip is caught by the checksum and skews proposals.
#[test]
fn a_meta_flip_and_tamper() {
    let r = task_114::run_case("meta_flip_and_tamper").unwrap();
    assert!(r.passed, "failures: {:?}", r.failures);
    assert_eq!(r.metrics["tamper_verdict"], "MetaTampered");
    let skew = r.metrics["max_category_skew"].as_f64().unwrap();
    assert!(skew >= 0.10, "proposal skew {skew} < 0.10: flip is vacuous");
}
