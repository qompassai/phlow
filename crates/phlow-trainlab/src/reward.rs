//! Evaluation classification and reward semantics.
//!
//! Mirrors `reward_for` in the companion repo's `scripts/train_rl.py`:
//! a completion is first classified by execution (passed / failed /
//! invalid), and only then mapped to a scalar reward.

use serde::Serialize;

use crate::error::TrainlabError;

/// Default invalid-code penalty, as in the Python (`-0.1`).
pub const INVALID_PENALTY_DEFAULT: f64 = -0.1;

/// Outcome of executing one completion against a task's hidden cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EvalStatus {
    /// Every hidden case passed.
    Passed,
    /// The code ran but at least one case failed.
    Failed,
    /// The code could not be evaluated: syntax error, missing entry
    /// point, executor deadline, or no harness result. Invalid is a
    /// property of the *completion*, never of the harness run itself.
    Invalid,
}

/// One completion's evaluation record.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Evaluation {
    /// Classification.
    pub status: EvalStatus,
    /// Hidden cases passed.
    pub tests_passed: usize,
    /// Hidden cases total.
    pub tests_total: usize,
    /// Human-readable detail (bounded by the executor).
    pub message: String,
}

impl Evaluation {
    /// An `Invalid` evaluation with no executed cases.
    pub fn invalid(message: impl Into<String>) -> Evaluation {
        Evaluation {
            status: EvalStatus::Invalid,
            tests_passed: 0,
            tests_total: 0,
            message: message.into(),
        }
    }

    /// Fraction of hidden cases passed, in `0.0..=1.0`. A completion
    /// with no cases at all is defined as fraction 0 (fail closed).
    pub fn pass_fraction(&self) -> f64 {
        if self.tests_total == 0 {
            return 0.0;
        }
        self.tests_passed as f64 / self.tests_total as f64
    }
}

/// How evaluations map to scalar rewards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RewardMode {
    /// `1.0` if all cases pass, `0.0` if valid but wrong, the
    /// invalid penalty if invalid.
    Binary,
    /// The fraction of cases passed, plus the invalid penalty when
    /// invalid (so invalid code scores below valid-but-wrong code).
    CaseFraction,
}

/// Reward configuration. `invalid_penalty` must be finite and in
/// `-1.0..=0.0` (the Python screens `0`, `-0.1`, `-1`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct RewardConfig {
    /// Mapping mode.
    pub mode: RewardMode,
    /// Penalty applied to invalid completions.
    pub invalid_penalty: f64,
}

impl Default for RewardConfig {
    fn default() -> Self {
        RewardConfig {
            mode: RewardMode::Binary,
            invalid_penalty: INVALID_PENALTY_DEFAULT,
        }
    }
}

impl RewardConfig {
    /// Validate the penalty range; out-of-range values are config
    /// errors, never silently clamped.
    pub fn validate(&self) -> Result<(), TrainlabError> {
        if !self.invalid_penalty.is_finite() || !(-1.0..=0.0).contains(&self.invalid_penalty) {
            return Err(TrainlabError::InvalidConfig(format!(
                "invalid_penalty {} outside -1.0..=0.0",
                self.invalid_penalty
            )));
        }
        Ok(())
    }
}

/// Map an evaluation to its scalar reward under `config`.
///
/// Pure and total: every evaluation has exactly one reward.
pub fn reward_for(evaluation: &Evaluation, config: &RewardConfig) -> f64 {
    match config.mode {
        RewardMode::Binary => match evaluation.status {
            EvalStatus::Passed => 1.0,
            EvalStatus::Failed => 0.0,
            EvalStatus::Invalid => config.invalid_penalty,
        },
        RewardMode::CaseFraction => {
            let mut reward = evaluation.pass_fraction();
            if evaluation.status == EvalStatus::Invalid {
                reward += config.invalid_penalty;
            }
            reward
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evaluation(status: EvalStatus, passed: usize, total: usize) -> Evaluation {
        Evaluation {
            status,
            tests_passed: passed,
            tests_total: total,
            message: String::new(),
        }
    }

    #[test]
    fn binary_reward_matches_the_python_scheme() {
        let config = RewardConfig::default();
        assert_eq!(
            reward_for(&evaluation(EvalStatus::Passed, 3, 3), &config),
            1.0
        );
        assert_eq!(
            reward_for(&evaluation(EvalStatus::Failed, 2, 3), &config),
            0.0
        );
        assert_eq!(
            reward_for(&evaluation(EvalStatus::Invalid, 0, 0), &config),
            -0.1
        );
    }

    #[test]
    fn case_fraction_reward_gives_partial_credit() {
        let config = RewardConfig {
            mode: RewardMode::CaseFraction,
            invalid_penalty: -0.1,
        };
        let reward = reward_for(&evaluation(EvalStatus::Failed, 2, 3), &config);
        assert!((reward - 2.0 / 3.0).abs() < 1e-12);
        // Invalid code scores strictly below valid-but-wrong code.
        let invalid = reward_for(&evaluation(EvalStatus::Invalid, 0, 0), &config);
        assert!(invalid < 0.0);
    }

    #[test]
    fn penalty_range_is_validated() {
        let bad = RewardConfig {
            mode: RewardMode::Binary,
            invalid_penalty: -2.0,
        };
        assert!(bad.validate().is_err());
        let nan = RewardConfig {
            mode: RewardMode::Binary,
            invalid_penalty: f64::NAN,
        };
        assert!(nan.validate().is_err());
        assert!(RewardConfig::default().validate().is_ok());
    }

    #[test]
    fn empty_evaluation_has_zero_fraction() {
        // Adversarial: a harness that reported no cases must not be
        // readable as a pass.
        assert_eq!(evaluation(EvalStatus::Failed, 0, 0).pass_fraction(), 0.0);
    }
}
