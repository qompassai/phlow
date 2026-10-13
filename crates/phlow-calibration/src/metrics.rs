//! Brier scores, expected calibration error, and calibration
//! curves, per the design's Metrics section.
//!
//! Two Brier forms are reported, and they are never conflated:
//!
//! - [`Metrics::brier`] is the design's stated formula over the
//!   (confidence, correct) pair of each labeled tuple:
//!   `mean((predicted_prob - actual_outcome)^2)`. A binary coin-flip
//!   predictor scores exactly 0.25 under it, as the design states.
//! - [`Metrics::brier_full_distribution`] is the multiclass Brier
//!   over full distributions, `mean(sum((p_i - o_i)^2))`, computed
//!   only over labeled tuples that genuinely carry a distribution.

use std::collections::BTreeMap;

use crate::tuple::{CalibrationTuple, QuestionKind};

/// The design's calibration-curve resolution: ten equal bins over
/// 0.0..=1.0. Callers measuring a thin corpus may pass fewer bins to
/// [`compute_metrics`]; the counts per bin are always reported so a
/// thin bin is visible rather than hidden.
pub const BINS_DEFAULT: usize = 10;

/// The largest bin count [`compute_metrics`] accepts.
pub const BINS_MAX: usize = 100;

/// Design Phase A gate: Brier below this counts as well-calibrated.
pub const WELL_CALIBRATED_BRIER_MAX: f64 = 0.1;

/// Design Phase A gate: ECE below this counts as well-calibrated.
pub const WELL_CALIBRATED_ECE_MAX: f64 = 0.05;

/// One bin of a calibration curve.
#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationBin {
    /// 0-based bin index.
    pub bin_index: usize,
    /// Inclusive lower confidence bound.
    pub lower: f64,
    /// Upper confidence bound (the last bin includes 1.0).
    pub upper: f64,
    /// Tuples whose confidence fell in this bin.
    pub count: usize,
    /// Mean predicted confidence in this bin, when non-empty.
    pub mean_confidence: Option<f64>,
    /// Fraction of correct predictions in this bin, when non-empty.
    pub accuracy: Option<f64>,
}

/// A calibration curve: exactly `bin_count` bins, empty bins kept
/// so the resolution is visible in the report.
#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationCurve {
    /// The bins, in ascending confidence order.
    pub bins: Vec<CalibrationBin>,
}

/// Calibration measurements for one question kind.
#[derive(Debug, Clone, PartialEq)]
pub struct KindMetrics {
    /// The question kind measured.
    pub kind: QuestionKind,
    /// All tuples of this kind in scope.
    pub tuples: usize,
    /// Tuples carrying a ground-truth label.
    pub labeled: usize,
    /// Labeled tuples that also carry a full distribution.
    pub distribution_labeled: usize,
    /// The design's Brier score, when any tuple is labeled.
    pub brier: Option<f64>,
    /// The full-distribution Brier score, when any labeled tuple
    /// carries a distribution.
    pub brier_full_distribution: Option<f64>,
    /// Expected calibration error, when any tuple is labeled.
    pub ece: Option<f64>,
    /// The calibration curve over labeled tuples.
    pub curve: CalibrationCurve,
}

/// Calibration measurements over one scope of tuples.
#[derive(Debug, Clone, PartialEq)]
pub struct Metrics {
    /// All tuples in scope.
    pub tuples: usize,
    /// Tuples carrying a ground-truth label.
    pub labeled: usize,
    /// Labeled tuples that also carry a full distribution.
    pub distribution_labeled: usize,
    /// The design's Brier score, when any tuple is labeled.
    pub brier: Option<f64>,
    /// The full-distribution Brier score, when any labeled tuple
    /// carries a distribution.
    pub brier_full_distribution: Option<f64>,
    /// Expected calibration error, when any tuple is labeled.
    pub ece: Option<f64>,
    /// The calibration curve over labeled tuples.
    pub curve: CalibrationCurve,
    /// The same measurements per question kind.
    pub per_kind: BTreeMap<QuestionKind, KindMetrics>,
}

/// Measurements split by the held-out reserve: `train` is every
/// tuple not held out, `held_out` is the validation reserve the
/// design never trains on.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitMetrics {
    /// Metrics over the non-held-out tuples.
    pub train: Metrics,
    /// Metrics over the held-out tuples.
    pub held_out: Metrics,
}

/// The design's Phase A gate outcome for a held-out scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateVerdict {
    /// Brier and ECE both under the design's thresholds.
    WellCalibrated,
    /// Labeled data exists and at least one threshold is missed.
    NotCalibrated,
    /// No labeled held-out tuples: calibration cannot be judged.
    InsufficientData,
}

/// Compute the metrics for one scope of tuples at `bin_count`
/// equal-width bins over 0.0..=1.0.
///
/// # Panics
///
/// Panics when `bin_count` is outside 1..=[`BINS_MAX`]; the bin
/// count is a caller-chosen constant, not external data.
pub fn compute_metrics(tuples: &[CalibrationTuple], bin_count: usize) -> Metrics {
    assert!(
        (1..=BINS_MAX).contains(&bin_count),
        "bin_count must be in 1..={BINS_MAX}"
    );
    let mut per_kind: BTreeMap<QuestionKind, Vec<&CalibrationTuple>> = BTreeMap::new();
    for tuple in tuples {
        per_kind.entry(tuple.kind()).or_default().push(tuple);
    }
    let kind_metrics: BTreeMap<QuestionKind, KindMetrics> = per_kind
        .iter()
        .map(|(kind, group)| {
            let measured = measure_group(group, bin_count);
            (
                *kind,
                KindMetrics {
                    kind: *kind,
                    tuples: measured.tuples,
                    labeled: measured.labeled,
                    distribution_labeled: measured.distribution_labeled,
                    brier: measured.brier,
                    brier_full_distribution: measured.brier_full_distribution,
                    ece: measured.ece,
                    curve: measured.curve,
                },
            )
        })
        .collect();
    let all: Vec<&CalibrationTuple> = tuples.iter().collect();
    let overall = measure_group(&all, bin_count);
    Metrics {
        tuples: overall.tuples,
        labeled: overall.labeled,
        distribution_labeled: overall.distribution_labeled,
        brier: overall.brier,
        brier_full_distribution: overall.brier_full_distribution,
        ece: overall.ece,
        curve: overall.curve,
        per_kind: kind_metrics,
    }
}

/// The numeric bundle [`measure_group`] produces, before the caller
/// attaches a kind (per-kind scopes) or not (the overall scope).
struct GroupMeasure {
    tuples: usize,
    labeled: usize,
    distribution_labeled: usize,
    brier: Option<f64>,
    brier_full_distribution: Option<f64>,
    ece: Option<f64>,
    curve: CalibrationCurve,
}

/// Compute the metrics for the train and held-out scopes.
pub fn compute_split_metrics(tuples: &[CalibrationTuple], bin_count: usize) -> SplitMetrics {
    let train: Vec<CalibrationTuple> = tuples.iter().filter(|t| !t.held_out).cloned().collect();
    let held_out: Vec<CalibrationTuple> = tuples.iter().filter(|t| t.held_out).cloned().collect();
    SplitMetrics {
        train: compute_metrics(&train, bin_count),
        held_out: compute_metrics(&held_out, bin_count),
    }
}

/// Judge a held-out scope against the design's Phase A thresholds:
/// Brier < [`WELL_CALIBRATED_BRIER_MAX`] and ECE <
/// [`WELL_CALIBRATED_ECE_MAX`] means well-calibrated and no
/// training is needed. With no labeled tuples the honest verdict
/// is [`GateVerdict::InsufficientData`], never a pass.
pub fn gate_verdict(held_out: &Metrics) -> GateVerdict {
    match (held_out.brier, held_out.ece) {
        (Some(brier), Some(ece)) => {
            if brier < WELL_CALIBRATED_BRIER_MAX && ece < WELL_CALIBRATED_ECE_MAX {
                GateVerdict::WellCalibrated
            } else {
                GateVerdict::NotCalibrated
            }
        }
        _ => GateVerdict::InsufficientData,
    }
}

/// Measure one group of tuples (one kind, or the overall scope).
fn measure_group(tuples: &[&CalibrationTuple], bin_count: usize) -> GroupMeasure {
    let mut pairs: Vec<(f64, bool)> = Vec::new();
    let mut full_brier_sum = 0.0_f64;
    let mut distribution_labeled = 0_usize;
    for tuple in tuples {
        if let Some(pair) = tuple.scored_outcome() {
            pairs.push(pair);
        }
        if let (Some(dist), Some(truth)) = (tuple.full_distribution(), tuple.truth_distribution())
            && dist.len() == truth.len()
        {
            full_brier_sum += dist
                .iter()
                .zip(truth.iter())
                .map(|(p, o)| (p - o) * (p - o))
                .sum::<f64>();
            distribution_labeled += 1;
        }
    }
    let labeled = pairs.len();
    let brier = if labeled == 0 {
        None
    } else {
        Some(
            pairs
                .iter()
                .map(|(confidence, correct)| {
                    let outcome = if *correct { 1.0 } else { 0.0 };
                    (confidence - outcome) * (confidence - outcome)
                })
                .sum::<f64>()
                / labeled as f64,
        )
    };
    let brier_full_distribution = if distribution_labeled == 0 {
        None
    } else {
        Some(full_brier_sum / distribution_labeled as f64)
    };
    let curve = build_curve(&pairs, bin_count);
    let ece = if labeled == 0 {
        None
    } else {
        Some(
            curve
                .bins
                .iter()
                .filter(|bin| bin.count > 0)
                .map(|bin| {
                    let accuracy = bin.accuracy.unwrap_or(0.0);
                    let confidence = bin.mean_confidence.unwrap_or(0.0);
                    (accuracy - confidence).abs() * (bin.count as f64 / labeled as f64)
                })
                .sum(),
        )
    };
    GroupMeasure {
        tuples: tuples.len(),
        labeled,
        distribution_labeled,
        brier,
        brier_full_distribution,
        ece,
        curve,
    }
}

/// Bin (confidence, correct) pairs into a calibration curve.
fn build_curve(pairs: &[(f64, bool)], bin_count: usize) -> CalibrationCurve {
    let width = 1.0 / bin_count as f64;
    let mut counts = vec![0_usize; bin_count];
    let mut confidence_sums = vec![0.0_f64; bin_count];
    let mut correct_counts = vec![0_usize; bin_count];
    for (confidence, correct) in pairs {
        let mut index = (confidence / width) as usize;
        if index >= bin_count {
            index = bin_count - 1;
        }
        counts[index] += 1;
        confidence_sums[index] += confidence;
        if *correct {
            correct_counts[index] += 1;
        }
    }
    let bins = (0..bin_count)
        .map(|index| CalibrationBin {
            bin_index: index,
            lower: index as f64 * width,
            upper: (index + 1) as f64 * width,
            count: counts[index],
            mean_confidence: if counts[index] == 0 {
                None
            } else {
                Some(confidence_sums[index] / counts[index] as f64)
            },
            accuracy: if counts[index] == 0 {
                None
            } else {
                Some(correct_counts[index] as f64 / counts[index] as f64)
            },
        })
        .collect();
    CalibrationCurve { bins }
}
