//! Metrics: the design's formulas against hand-computed values,
//! plus the adversarial shapes (unlabeled, NaN, distribution-less)
//! that must degrade to honest `None`s rather than invented scores.

mod common;

use common::{choice_tuple, noul_tuple, score_tuple};
use phlow_calibration::{
    CalibrationTuple, GateVerdict, compute_metrics, compute_split_metrics, gate_verdict,
};
use phlow_system1::Answer;

/// Assert two f64s agree to measurement precision.
fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-12,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn brier_perfect_predictions_score_zero() {
    let tuples = vec![
        noul_tuple(1, false, 1.0, true, Some(true)),
        choice_tuple(2, false, 1.0, 0, Some(0)),
    ];
    let metrics = compute_metrics(&tuples, 10);
    assert_close(metrics.brier.expect("labeled"), 0.0);
    assert_close(metrics.ece.expect("labeled"), 0.0);
}

#[test]
fn brier_binary_random_anchor_is_a_quarter() {
    // The design's anchor: binary 0.5-confidence predictions score
    // exactly 0.25 whether right or wrong.
    let tuples = vec![
        noul_tuple(1, false, 0.5, true, Some(true)),
        noul_tuple(2, false, 0.5, true, Some(false)),
        noul_tuple(3, false, 0.5, false, Some(false)),
        noul_tuple(4, false, 0.5, false, Some(true)),
    ];
    let metrics = compute_metrics(&tuples, 10);
    assert_close(metrics.brier.expect("labeled"), 0.25);
}

#[test]
fn brier_known_single_value() {
    // (0.8 - 1)^2 = 0.04.
    let tuples = vec![choice_tuple(1, false, 0.8, 1, Some(1))];
    let metrics = compute_metrics(&tuples, 10);
    assert_close(metrics.brier.expect("labeled"), 0.04);
}

#[test]
fn ece_known_value_from_design_example_shape() {
    // Ten predictions at confidence 0.9, seven correct: the only
    // occupied bin contributes |0.7 - 0.9| * 1.0 = 0.2.
    let mut tuples = Vec::new();
    for seed in 0..10 {
        let correct = seed < 7;
        tuples.push(choice_tuple(
            100 + seed,
            false,
            0.9,
            0,
            Some(if correct { 0 } else { 1 }),
        ));
    }
    let metrics = compute_metrics(&tuples, 10);
    assert_close(metrics.ece.expect("labeled"), 0.2);
}

#[test]
fn empty_scope_scores_are_none_not_zero() {
    let metrics = compute_metrics(&[], 10);
    assert_eq!(metrics.tuples, 0);
    assert!(metrics.brier.is_none());
    assert!(metrics.brier_full_distribution.is_none());
    assert!(metrics.ece.is_none());
    assert_eq!(gate_verdict(&metrics), GateVerdict::InsufficientData);
}

#[test]
fn unlabeled_tuples_are_counted_never_scored() {
    let tuples = vec![
        noul_tuple(1, false, 0.99, false, None),
        choice_tuple(2, false, 0.99, 0, None),
    ];
    let metrics = compute_metrics(&tuples, 10);
    assert_eq!(metrics.tuples, 2);
    assert_eq!(metrics.labeled, 0);
    assert!(metrics.brier.is_none());
    assert!(metrics.ece.is_none());
}

#[test]
fn full_distribution_brier_noul_at_half_is_a_half() {
    // dist [0.5, 0.5] against one-hot [0, 1]: 0.25 + 0.25 = 0.5.
    let tuples = vec![noul_tuple(1, false, 0.5, true, Some(true))];
    let metrics = compute_metrics(&tuples, 10);
    assert_eq!(metrics.distribution_labeled, 1);
    assert_close(metrics.brier_full_distribution.expect("dist labeled"), 0.5);
}

#[test]
fn full_distribution_brier_perfect_noul_is_zero() {
    let tuples = vec![noul_tuple(1, false, 1.0, true, Some(true))];
    let metrics = compute_metrics(&tuples, 10);
    assert_close(metrics.brier_full_distribution.expect("dist labeled"), 0.0);
}

#[test]
fn score_tuples_never_enter_full_distribution_brier() {
    // A scalar truth cannot be one-hot aligned over distribution
    // levels; the crate must say so with None, not invent a level.
    let tuples = vec![score_tuple(1, false, 0.05, 0.99, Some(0.05))];
    let metrics = compute_metrics(&tuples, 10);
    assert!(metrics.brier.is_some());
    assert_eq!(metrics.distribution_labeled, 0);
    assert!(metrics.brier_full_distribution.is_none());
}

#[test]
fn scalar_choice_has_no_full_distribution_brier() {
    let tuples = vec![choice_tuple(1, false, 0.9, 0, Some(0))];
    let metrics = compute_metrics(&tuples, 10);
    assert_eq!(metrics.distribution_labeled, 0);
    assert!(metrics.brier_full_distribution.is_none());
}

#[test]
fn confidence_edges_land_in_first_and_last_bins() {
    let tuples = vec![
        noul_tuple(1, false, 0.0, true, Some(true)),
        noul_tuple(2, false, 1.0, true, Some(true)),
    ];
    let metrics = compute_metrics(&tuples, 10);
    assert_eq!(metrics.curve.bins.len(), 10);
    assert_eq!(metrics.curve.bins[0].count, 1);
    assert_eq!(metrics.curve.bins[9].count, 1);
    assert_eq!(metrics.curve.bins[9].upper, 1.0);
}

#[test]
fn bin_count_is_respected() {
    let tuples = vec![noul_tuple(1, false, 0.5, true, Some(true))];
    let metrics = compute_metrics(&tuples, 4);
    assert_eq!(metrics.curve.bins.len(), 4);
    assert_eq!(metrics.curve.bins[2].count, 1);
}

#[test]
fn gate_well_calibrated_under_both_thresholds() {
    let tuples: Vec<CalibrationTuple> = (0..20)
        .map(|seed| noul_tuple(200 + seed, true, 0.99, true, Some(true)))
        .collect();
    let split = compute_split_metrics(&tuples, 10);
    assert_eq!(split.held_out.labeled, 20);
    assert_eq!(gate_verdict(&split.held_out), GateVerdict::WellCalibrated);
}

#[test]
fn gate_not_calibrated_when_confident_and_wrong() {
    let tuples: Vec<CalibrationTuple> = (0..10)
        .map(|seed| noul_tuple(300 + seed, true, 0.99, true, Some(false)))
        .collect();
    let split = compute_split_metrics(&tuples, 10);
    assert_eq!(gate_verdict(&split.held_out), GateVerdict::NotCalibrated);
}

#[test]
fn gate_threshold_is_strict_at_the_boundary() {
    // Brier exactly 0.1 (mean of 0.04 and 0.16) is NOT < 0.1.
    let tuples = vec![
        choice_tuple(1, true, 0.8, 0, Some(0)),
        choice_tuple(2, true, 0.6, 0, Some(0)),
    ];
    let split = compute_split_metrics(&tuples, 10);
    assert_close(split.held_out.brier.expect("labeled"), 0.1);
    assert_eq!(gate_verdict(&split.held_out), GateVerdict::NotCalibrated);
}

#[test]
fn per_kind_metrics_are_reported_separately() {
    let tuples = vec![
        noul_tuple(1, false, 0.9, true, Some(true)),
        choice_tuple(2, false, 0.9, 0, Some(0)),
        score_tuple(3, false, 0.5, 0.9, Some(0.5)),
    ];
    let metrics = compute_metrics(&tuples, 10);
    assert_eq!(metrics.per_kind.len(), 3);
    for kind_metrics in metrics.per_kind.values() {
        assert_eq!(kind_metrics.tuples, 1);
        assert_eq!(kind_metrics.labeled, 1);
    }
}

#[test]
fn split_metrics_separate_the_reserve() {
    let tuples = vec![
        noul_tuple(1, false, 0.9, true, Some(true)),
        noul_tuple(2, true, 0.9, true, Some(true)),
        noul_tuple(3, true, 0.9, true, Some(true)),
    ];
    let split = compute_split_metrics(&tuples, 10);
    assert_eq!(split.train.tuples, 1);
    assert_eq!(split.held_out.tuples, 2);
}

#[test]
fn nan_confidence_is_never_scored() {
    // Adversarial: a tuple built by hand with a NaN probability
    // (validate() would reject it; metrics must still not score it).
    let mut tuple = noul_tuple(1, false, 0.9, true, Some(true));
    tuple.predicted = Answer::Noul {
        probability: f64::NAN,
        yes: true,
    };
    let metrics = compute_metrics(&[tuple], 10);
    assert_eq!(metrics.labeled, 0);
    assert!(metrics.brier.is_none());
}

#[test]
fn score_exact_equality_defines_correctness() {
    let right = score_tuple(1, false, 0.5, 0.9, Some(0.5));
    let near = score_tuple(2, false, 0.5, 0.9, Some(0.500_001));
    let metrics = compute_metrics(&[right, near], 10);
    // One correct, one incorrect at confidence 0.9:
    // brier = ((0.9-1)^2 + (0.9-0)^2) / 2 = (0.01 + 0.81) / 2 = 0.41.
    assert_close(metrics.brier.expect("labeled"), 0.41);
}
