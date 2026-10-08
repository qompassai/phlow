//! Distribution statistics for the trigger and calibration probes.
//!
//! All functions are pure and total on validated inputs: callers pass
//! distributions that already passed
//! [`crate::probe::check_rich_answer`], so lengths match and values
//! are finite in `0.0..=1.0`. Assertions here guard internal invariants
//! only; malformed backend data is rejected before it reaches them.

/// Mean of a non-empty slice. The caller owns the non-empty contract.
pub fn mean(values: &[f64]) -> f64 {
    assert!(!values.is_empty(), "mean of an empty slice");
    values.iter().sum::<f64>() / values.len() as f64
}

/// Population variance of a non-empty slice.
pub fn variance(values: &[f64]) -> f64 {
    assert!(!values.is_empty(), "variance of an empty slice");
    let center = mean(values);
    values
        .iter()
        .map(|v| (v - center) * (v - center))
        .sum::<f64>()
        / values.len() as f64
}

/// Element-wise mean of equally sized distributions.
pub fn mean_distribution(distributions: &[&[f64]]) -> Vec<f64> {
    assert!(!distributions.is_empty(), "mean of no distributions");
    let width = distributions[0].len();
    assert!(
        distributions.iter().all(|d| d.len() == width),
        "distribution widths differ"
    );
    let mut out = vec![0.0_f64; width];
    for dist in distributions {
        for (slot, value) in out.iter_mut().zip(dist.iter()) {
            *slot += value;
        }
    }
    let count = distributions.len() as f64;
    for slot in &mut out {
        *slot /= count;
    }
    out
}

/// L1 distance between two equally sized distributions, in `0.0..=2.0`.
/// This is the confidence-shift statistic the trigger probes judge.
pub fn l1_shift(a: &[f64], b: &[f64]) -> f64 {
    assert_eq!(a.len(), b.len(), "shift needs equal widths");
    a.iter().zip(b.iter()).map(|(x, y)| (x - y).abs()).sum()
}

/// The model's confidence in its top option under one distribution.
pub fn top_confidence(distribution: &[f64]) -> f64 {
    assert!(!distribution.is_empty(), "confidence of no options");
    distribution.iter().copied().fold(0.0_f64, f64::max)
}

/// Bimodality score for a set of confidences, in `-1.0..=0.5`.
///
/// `min(high_fraction, low_fraction) - mid_fraction`, where high is
/// `>= high_band`, low is `<= low_band`, and mid is everything between.
/// A clean model's confidences cluster in one band (score <= 0); a
/// sharp two-humped split with an empty middle scores towards 0.5.
pub fn bimodality_score(confidences: &[f64], high_band: f64, low_band: f64) -> f64 {
    assert!(!confidences.is_empty(), "bimodality of no samples");
    assert!(low_band < high_band, "bimodality bands inverted");
    let total = confidences.len() as f64;
    let mut high = 0_usize;
    let mut low = 0_usize;
    for value in confidences {
        if *value >= high_band {
            high += 1;
        } else if *value <= low_band {
            low += 1;
        }
    }
    let mid = confidences.len() - high - low;
    (high.min(low) as f64) / total - (mid as f64) / total
}
