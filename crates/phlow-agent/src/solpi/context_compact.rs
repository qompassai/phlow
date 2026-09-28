//! Online Context Compact: completed steps become compaction candidates.
//!
//! Re-expresses SoL-Pi's Online Context Compact ("completed plan steps
//! become candidate points for ... compaction, subject to economic and
//! window-pressure checks"). The engine evaluates a snapshot of steps
//! against an explicit cost model with named thresholds and returns a
//! [`CompactionPlan`] describing the decision. Compaction itself is the
//! caller's job; this module only decides and explains.
//!
//! Sizes are measured from step content by the engine — never trusted from
//! caller-supplied claims — so adversarial size inflation cannot force a
//! compaction.

use std::fmt;

/// Default context window in bytes for the cost model.
pub const WINDOW_BYTES_DEFAULT: usize = 256 * 1024;

/// Default window-pressure threshold: compact only when the window is at
/// least this full (fraction of [`WINDOW_BYTES_DEFAULT`]).
pub const PRESSURE_THRESHOLD_DEFAULT: f64 = 0.75;

/// Default minimum reclaimable bytes for a compaction to pay for itself.
pub const MIN_SAVINGS_BYTES_DEFAULT: usize = 4096;

/// Default minimum candidate steps for a compaction to be worthwhile.
pub const MIN_CANDIDATE_STEPS_DEFAULT: usize = 2;

/// Maximum candidates named in one plan.
pub const CANDIDATES_MAX: usize = 64;

/// Maximum characters of a plan's reason string.
pub const REASON_CHARS_MAX: usize = 512;

/// One step of agent work, as seen by the compaction engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactStep {
    /// Stable step identifier, carried into the plan.
    pub id: u64,
    /// Step content; the engine measures `content.len()` itself.
    pub content: String,
    /// Only completed steps are ever candidates.
    pub completed: bool,
    /// Pinned steps (system prompts, safety context) are never candidates,
    /// regardless of pressure.
    pub pinned: bool,
}

/// The engine's decision, fully observable.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionPlan {
    /// Whether the economic and window-pressure checks both passed.
    pub compact: bool,
    /// Candidate step ids, in first-seen order, at most
    /// [`CANDIDATES_MAX`].
    pub candidate_ids: Vec<u64>,
    /// Bytes the candidates would reclaim if compacted.
    pub savings_bytes: usize,
    /// Measured window pressure: filled bytes / window bytes.
    pub window_pressure: f64,
    /// Human-readable account of the decision, bounded to
    /// [`REASON_CHARS_MAX`] characters.
    pub reason: String,
}

/// Cost-model configuration. Disabled by default.
#[derive(Debug, Clone)]
pub struct CompactionPolicy {
    enabled: bool,
    /// Context window in bytes; must be non-zero.
    pub window_bytes: usize,
    /// Pressure threshold in (0, 1]; must be finite.
    pub pressure_threshold: f64,
    /// Minimum reclaimable bytes for compaction to pay for itself.
    pub min_savings_bytes: usize,
    /// Minimum candidate steps for compaction to be worthwhile.
    pub min_candidate_steps: usize,
    /// Maximum candidates named in one plan.
    pub candidates_max: usize,
}

impl CompactionPolicy {
    /// Disabled policy: evaluation refuses with [`PolicyError::NotEnabled`].
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            window_bytes: WINDOW_BYTES_DEFAULT,
            pressure_threshold: PRESSURE_THRESHOLD_DEFAULT,
            min_savings_bytes: MIN_SAVINGS_BYTES_DEFAULT,
            min_candidate_steps: MIN_CANDIDATE_STEPS_DEFAULT,
            candidates_max: CANDIDATES_MAX,
        }
    }

    /// Explicit opt-in with default thresholds.
    pub fn opt_in() -> Self {
        Self {
            enabled: true,
            ..Self::disabled()
        }
    }

    /// Explicit opt-in with caller-chosen thresholds.
    ///
    /// Rejects a non-finite or out-of-range pressure threshold and a zero
    /// window; bad economics must fail at construction, not at evaluation.
    pub fn with_thresholds(
        window_bytes: usize,
        pressure_threshold: f64,
        min_savings_bytes: usize,
        min_candidate_steps: usize,
    ) -> Result<Self, PolicyError> {
        if window_bytes == 0 {
            return Err(PolicyError::ZeroWindow);
        }
        if !pressure_threshold.is_finite() || pressure_threshold <= 0.0 || pressure_threshold > 1.0
        {
            return Err(PolicyError::InvalidPressureThreshold {
                value: pressure_threshold,
            });
        }
        Ok(Self {
            enabled: true,
            window_bytes,
            pressure_threshold,
            min_savings_bytes,
            min_candidate_steps,
            candidates_max: CANDIDATES_MAX,
        })
    }

    /// True only after explicit opt-in.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

impl Default for CompactionPolicy {
    /// Default is disabled, matching the missing-config rule.
    fn default() -> Self {
        Self::disabled()
    }
}

/// Policy and evaluation failures.
#[derive(Debug, Clone, PartialEq)]
pub enum PolicyError {
    /// The policy is not opted in. No plan is produced.
    NotEnabled,
    /// The window size was zero.
    ZeroWindow,
    /// The pressure threshold was NaN, infinite, or outside (0, 1].
    InvalidPressureThreshold {
        /// The rejected value.
        value: f64,
    },
    /// Byte accounting overflowed; inputs are not sane.
    ArithmeticOverflow,
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PolicyError::NotEnabled => write!(f, "online context compact is not enabled"),
            PolicyError::ZeroWindow => write!(f, "compaction window must be non-zero"),
            PolicyError::InvalidPressureThreshold { value } => write!(
                f,
                "pressure threshold {value} is not a finite fraction in (0, 1]"
            ),
            PolicyError::ArithmeticOverflow => {
                write!(f, "step byte accounting overflowed")
            }
        }
    }
}

impl std::error::Error for PolicyError {}

/// Bound a reason string to [`REASON_CHARS_MAX`] characters.
fn bound_reason(reason: String) -> String {
    reason.chars().take(REASON_CHARS_MAX).collect()
}

/// Evaluate steps against the cost model and return the decision.
///
/// Contract:
/// - Refuses with [`PolicyError::NotEnabled`] before measuring anything
///   when the policy is not opted in.
/// - Measures every size from step content; caller claims are not an
///   input to this function at all.
/// - Candidates are completed, unpinned steps, first-seen order, capped at
///   `candidates_max`.
/// - Compacts only when window pressure meets the threshold AND
///   reclaimable bytes meet the minimum AND the candidate count meets the
///   minimum. The reason names which check decided.
pub fn evaluate_compaction(
    policy: &CompactionPolicy,
    steps: &[CompactStep],
) -> Result<CompactionPlan, PolicyError> {
    if !policy.is_enabled() {
        return Err(PolicyError::NotEnabled);
    }

    let mut filled_bytes: usize = 0;
    let mut candidate_ids: Vec<u64> = Vec::new();
    let mut savings_bytes: usize = 0;
    for step in steps {
        filled_bytes = filled_bytes
            .checked_add(step.content.len())
            .ok_or(PolicyError::ArithmeticOverflow)?;
        if step.completed && !step.pinned && candidate_ids.len() < policy.candidates_max {
            candidate_ids.push(step.id);
            savings_bytes = savings_bytes
                .checked_add(step.content.len())
                .ok_or(PolicyError::ArithmeticOverflow)?;
        }
    }

    // window_bytes > 0 is enforced by construction.
    let window_pressure = filled_bytes as f64 / policy.window_bytes as f64;
    let candidate_count = candidate_ids.len();
    let pressure_ok = window_pressure >= policy.pressure_threshold;
    let savings_ok = savings_bytes >= policy.min_savings_bytes;
    let count_ok = candidate_count >= policy.min_candidate_steps;

    let (compact, reason) = if !pressure_ok {
        (
            false,
            format!(
                "window pressure {window_pressure:.3} below threshold {:.3}; no compaction",
                policy.pressure_threshold,
            ),
        )
    } else if !savings_ok {
        (
            false,
            format!(
                "reclaimable {savings_bytes} bytes below minimum {}; no compaction",
                policy.min_savings_bytes,
            ),
        )
    } else if !count_ok {
        (
            false,
            format!(
                "{candidate_count} candidates below minimum {}; no compaction",
                policy.min_candidate_steps,
            ),
        )
    } else {
        (
            true,
            format!(
                "pressure {window_pressure:.3} >= {:.3}, savings {savings_bytes} bytes, \
             {candidate_count} candidates: compacting",
                policy.pressure_threshold,
            ),
        )
    };

    Ok(CompactionPlan {
        compact,
        candidate_ids,
        savings_bytes,
        window_pressure,
        reason: bound_reason(reason),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CANDIDATES_MAX, CompactStep, CompactionPolicy, MIN_SAVINGS_BYTES_DEFAULT,
        PRESSURE_THRESHOLD_DEFAULT, PolicyError, WINDOW_BYTES_DEFAULT, evaluate_compaction,
    };

    fn step(id: u64, bytes: usize, completed: bool, pinned: bool) -> CompactStep {
        CompactStep {
            id,
            content: "s".repeat(bytes),
            completed,
            pinned,
        }
    }

    // ---------------- validation tests ----------------

    #[test]
    fn compact_evaluates_true_when_all_checks_pass() {
        let policy = CompactionPolicy::opt_in();
        let fill = (WINDOW_BYTES_DEFAULT as f64 * PRESSURE_THRESHOLD_DEFAULT) as usize + 100;
        let steps = vec![
            step(1, fill / 2, true, false),
            step(2, fill / 2 + 200, true, false),
            step(3, 64, false, false),
        ];
        let plan = evaluate_compaction(&policy, &steps).expect("opted-in evaluates");
        assert!(plan.compact);
        assert_eq!(plan.candidate_ids, vec![1, 2]);
        assert!(plan.savings_bytes >= MIN_SAVINGS_BYTES_DEFAULT);
        assert!(plan.window_pressure >= PRESSURE_THRESHOLD_DEFAULT);
        assert!(!plan.reason.is_empty());
    }

    #[test]
    fn compact_refuses_when_nothing_completed() {
        let policy = CompactionPolicy::opt_in();
        let steps = vec![step(1, WINDOW_BYTES_DEFAULT, false, false)];
        let plan = evaluate_compaction(&policy, &steps).expect("opted-in evaluates");
        assert!(!plan.compact);
        assert!(plan.candidate_ids.is_empty());
        assert_eq!(plan.savings_bytes, 0);
    }

    #[test]
    fn compact_accepts_pressure_at_exact_threshold() {
        let policy =
            CompactionPolicy::with_thresholds(1000, 0.5, 100, 1).expect("valid thresholds");
        let steps = vec![step(1, 500, true, false)];
        let plan = evaluate_compaction(&policy, &steps).expect("opted-in evaluates");
        assert!(plan.compact, "boundary pressure meets the threshold");
    }

    #[test]
    fn compact_caps_candidate_list() {
        let policy = CompactionPolicy::opt_in();
        let steps: Vec<CompactStep> = (0..(CANDIDATES_MAX + 10) as u64)
            .map(|id| step(id, 1024, true, false))
            .collect();
        let plan = evaluate_compaction(&policy, &steps).expect("opted-in evaluates");
        assert_eq!(plan.candidate_ids.len(), CANDIDATES_MAX);
        assert_eq!(plan.candidate_ids[0], 0);
    }

    #[test]
    fn compact_handles_empty_steps() {
        let policy = CompactionPolicy::opt_in();
        let plan = evaluate_compaction(&policy, &[]).expect("opted-in evaluates");
        assert!(!plan.compact);
        assert_eq!(plan.window_pressure, 0.0);
        assert!(plan.candidate_ids.is_empty());
    }

    // ---------------- adversarial tests ----------------

    #[test]
    fn compact_refuses_without_opt_in() {
        let policy = CompactionPolicy::disabled();
        assert!(!policy.is_enabled());
        let result = evaluate_compaction(&policy, &[step(1, 1024, true, false)]);
        assert_eq!(result, Err(PolicyError::NotEnabled));
    }

    #[test]
    fn compact_ignores_full_window_of_incomplete_steps() {
        // Adversarial pressure signal: the window is full, but nothing is
        // completed, so there is nothing safe to compact.
        let policy = CompactionPolicy::opt_in();
        let steps = vec![step(1, WINDOW_BYTES_DEFAULT, false, false)];
        let plan = evaluate_compaction(&policy, &steps).expect("opted-in evaluates");
        assert!(plan.window_pressure >= PRESSURE_THRESHOLD_DEFAULT);
        assert!(!plan.compact);
        assert!(plan.candidate_ids.is_empty());
        assert!(plan.reason.contains("below minimum"));
    }

    #[test]
    fn compact_never_candidates_pinned_steps() {
        // Pinned safety context must survive any pressure.
        let policy = CompactionPolicy::with_thresholds(1000, 0.1, 10, 1).expect("valid thresholds");
        let steps = vec![step(1, 900, true, true), step(2, 900, true, false)];
        let plan = evaluate_compaction(&policy, &steps).expect("opted-in evaluates");
        assert!(!plan.candidate_ids.contains(&1));
        assert_eq!(plan.candidate_ids, vec![2]);
    }

    #[test]
    fn compact_refuses_when_savings_below_minimum() {
        let policy = CompactionPolicy::with_thresholds(1000, 0.5, MIN_SAVINGS_BYTES_DEFAULT, 1)
            .expect("valid thresholds");
        let steps = vec![step(1, 600, true, false)];
        let plan = evaluate_compaction(&policy, &steps).expect("opted-in evaluates");
        assert!(plan.window_pressure >= 0.5);
        assert!(!plan.compact, "uneconomic compaction is refused");
        assert!(plan.reason.contains("below minimum"));
    }

    #[test]
    fn compact_rejects_nonsense_thresholds_at_construction() {
        assert!(matches!(
            CompactionPolicy::with_thresholds(0, 0.5, 1, 1),
            Err(PolicyError::ZeroWindow)
        ));
        for bad in [f64::NAN, f64::INFINITY, 0.0, -0.25, 1.5] {
            assert!(
                matches!(
                    CompactionPolicy::with_thresholds(1000, bad, 1, 1),
                    Err(PolicyError::InvalidPressureThreshold { .. })
                ),
                "threshold {bad} must be rejected"
            );
        }
        // A default-constructed policy is disabled even with sane fields.
        let default = CompactionPolicy::default();
        assert!(!default.is_enabled());
        assert_eq!(
            evaluate_compaction(&default, &[]),
            Err(PolicyError::NotEnabled)
        );
    }
}
