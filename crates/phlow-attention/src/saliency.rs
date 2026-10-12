//! The saliency function: how much a workspace target matters, computed
//! identically by any agent from shared inputs.
//!
//! Plain words: this is the analog of the paper's object grounding.
//! Where MATE anchors learned attention to detector objects, this stack
//! has no learned attention to anchor — so saliency is a fixed,
//! published function of signals every participant can observe in the
//! shared environment (the SCIP index, the diagnostic aggregate, the
//! task text). Same inputs, same number, for any model from any vendor.
//! That reproducibility is what makes two strangers' foci comparable.
//!
//! Every signal contributes at most its weight, so no single signal —
//! including a poisoned one (a corrupted index inflating a target's
//! centrality, a diagnostic flood) — can dominate the ranking. The
//! per-signal caps are the poisoning bound; the snapshot hash (see
//! [`crate::snapshot`]) is the integrity check on top of it.

use serde::{Deserialize, Serialize};

/// Weight of "the task text names this target".
pub const WEIGHT_NAMED_IN_TASK: f64 = 0.35;
/// Weight of the error-diagnostic signal.
pub const WEIGHT_DIAGNOSTIC_ERRORS: f64 = 0.25;
/// Weight of the warning-diagnostic signal.
pub const WEIGHT_DIAGNOSTIC_WARNINGS: f64 = 0.10;
/// Weight of SCIP graph centrality (normalised inbound references).
pub const WEIGHT_GRAPH_CENTRALITY: f64 = 0.20;
/// Weight of change recency.
pub const WEIGHT_RECENCY: f64 = 0.10;
/// Tie-break epsilon for targets currently open in the editor. Kept
/// far below every real weight: being open breaks ties, it does not
/// manufacture importance.
pub const OPEN_IN_EDITOR_BONUS: f64 = 0.01;

/// Error count at which the error signal reaches half its weight.
/// Chosen so a single error is meaningful (≈0.05 of the total) but a
/// broken file cannot saturate the ranking alone.
pub const ERROR_SATURATION: f64 = 4.0;
/// Warning count at which the warning signal reaches half its weight.
pub const WARNING_SATURATION: f64 = 8.0;

/// Raw observable signals for one target, before scoring.
///
/// The caller owns every clock and index: `recency` arrives already
/// decayed into `0.0..=1.0`, `graph_centrality` already normalised.
/// Keeping the function free of I/O and time is what makes its output
/// reproducible across agents.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SaliencyInput {
    /// The task text names this target.
    pub named_in_task: bool,
    /// Error-severity diagnostics currently reported on the target.
    pub diagnostic_errors: u32,
    /// Warning-severity diagnostics currently reported on the target.
    pub diagnostic_warnings: u32,
    /// Inbound-reference centrality from the SCIP index, `0.0..=1.0`.
    pub graph_centrality: f64,
    /// Change recency, `0.0..=1.0` (1 = changed just now).
    pub recency: f64,
    /// The target is open in the editor right now.
    pub open_in_editor: bool,
}

impl SaliencyInput {
    /// The all-quiet input: nothing names, flags, or references the
    /// target. Scores exactly 0.
    pub fn quiet() -> SaliencyInput {
        SaliencyInput {
            named_in_task: false,
            diagnostic_errors: 0,
            diagnostic_warnings: 0,
            graph_centrality: 0.0,
            recency: 0.0,
            open_in_editor: false,
        }
    }
}

/// Narrow an untrusted unit-interval signal into `0.0..=1.0`.
/// Non-finite values narrow to 0: a NaN centrality from a corrupt index
/// must read as "no signal", never as an exception or a free pass.
fn unit(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Saturating response curve for diagnostic counts: `n / (n + k)`.
/// Bounded by construction — the count can grow without limit and the
/// contribution never exceeds the signal's weight.
fn saturating(count: u32, half_at: f64) -> f64 {
    let n = f64::from(count);
    n / (n + half_at)
}

/// Score one target. Pure and deterministic: same input, same output,
/// on every agent, every run. Output is always in `0.0..=1.0`.
pub fn saliency(input: &SaliencyInput) -> f64 {
    let mut score = 0.0;
    if input.named_in_task {
        score += WEIGHT_NAMED_IN_TASK;
    }
    score += WEIGHT_DIAGNOSTIC_ERRORS * saturating(input.diagnostic_errors, ERROR_SATURATION);
    score += WEIGHT_DIAGNOSTIC_WARNINGS * saturating(input.diagnostic_warnings, WARNING_SATURATION);
    score += WEIGHT_GRAPH_CENTRALITY * unit(input.graph_centrality);
    score += WEIGHT_RECENCY * unit(input.recency);
    if input.open_in_editor {
        score += OPEN_IN_EDITOR_BONUS;
    }
    unit(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> SaliencyInput {
        SaliencyInput::quiet()
    }

    #[test]
    fn quiet_input_scores_zero() {
        assert_eq!(saliency(&input()), 0.0);
    }

    #[test]
    fn named_in_task_is_the_strongest_single_signal() {
        let mut named = input();
        named.named_in_task = true;
        let mut central = input();
        central.graph_centrality = 1.0;
        let mut errors = input();
        errors.diagnostic_errors = 10_000;
        assert!(saliency(&named) > saliency(&central));
        assert!(saliency(&named) > saliency(&errors));
    }

    #[test]
    fn score_never_exceeds_one_even_with_everything_maxed() {
        let maxed = SaliencyInput {
            named_in_task: true,
            diagnostic_errors: u32::MAX,
            diagnostic_warnings: u32::MAX,
            graph_centrality: 1.0,
            recency: 1.0,
            open_in_editor: true,
        };
        let score = saliency(&maxed);
        assert!(score <= 1.0, "score {score} escaped the unit interval");
        assert!(
            score > 0.99,
            "maxed input should nearly saturate, got {score}"
        );
    }

    #[test]
    fn poisoned_error_count_cannot_exceed_its_weight() {
        // Adversarial: a diagnostic flood is the cheapest saliency
        // poisoning available. Its total contribution is capped by
        // WEIGHT_DIAGNOSTIC_ERRORS no matter the count.
        let mut flooded = input();
        flooded.diagnostic_errors = u32::MAX;
        let contribution = saliency(&flooded);
        assert!(
            contribution <= WEIGHT_DIAGNOSTIC_ERRORS + f64::EPSILON,
            "flood contributed {contribution}, weight is {WEIGHT_DIAGNOSTIC_ERRORS}"
        );
        // And a merely-named target still outranks the flood.
        let mut named = input();
        named.named_in_task = true;
        assert!(saliency(&named) > contribution);
    }

    #[test]
    fn non_finite_signals_narrow_to_no_signal() {
        let mut poisoned = input();
        poisoned.graph_centrality = f64::NAN;
        poisoned.recency = f64::INFINITY;
        assert_eq!(saliency(&poisoned), 0.0);
    }

    #[test]
    fn out_of_range_signals_are_clamped() {
        let mut wild = input();
        wild.graph_centrality = 7.5;
        wild.recency = -2.0;
        let mut clamped = input();
        clamped.graph_centrality = 1.0;
        assert_eq!(saliency(&wild), saliency(&clamped));
    }

    #[test]
    fn saliency_is_monotone_in_centrality_and_recency() {
        let mut low = input();
        low.graph_centrality = 0.2;
        low.recency = 0.2;
        let mut high = low;
        high.graph_centrality = 0.8;
        high.recency = 0.9;
        assert!(saliency(&high) > saliency(&low));
    }

    #[test]
    fn open_in_editor_only_breaks_ties() {
        let mut open = input();
        open.open_in_editor = true;
        assert_eq!(saliency(&open), OPEN_IN_EDITOR_BONUS);
        // The bonus must never outweigh the weakest real signal gap:
        // one error already beats it.
        let mut one_error = input();
        one_error.diagnostic_errors = 1;
        assert!(saliency(&one_error) > saliency(&open));
    }
}
