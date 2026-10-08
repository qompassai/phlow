//! Group statistics: RLOO leave-one-out advantages and pass@k.
//!
//! The advantage computation mirrors `leave_one_out` in the companion
//! repo's `scripts/train_rl.py`. The pass@k estimator is the standard
//! unbiased estimator from Chen et al. (2021, the Codex paper):
//! `pass@k = 1 - C(n - c, k) / C(n, k)` for `n` samples with `c`
//! passing, computed as a product to avoid factorials.

use crate::error::TrainlabError;

/// Maximum samples per task the estimator accepts (matches the
/// runner's sampling bound).
pub const SAMPLES_MAX: usize = 1024;

/// Leave-one-out advantages for one prompt group:
/// `advantage_i = reward_i - mean(rewards without i)`.
///
/// This is the baseline that makes the update *group-relative*: a
/// completion is reinforced only relative to its siblings. Requires
/// at least two rewards (the Python raises the same way: "RLOO
/// requires group size at least two"). The advantages always sum to
/// zero, which the tests assert.
pub fn leave_one_out_advantages(rewards: &[f64]) -> Result<Vec<f64>, TrainlabError> {
    if rewards.len() < 2 {
        return Err(TrainlabError::InvalidConfig(format!(
            "leave-one-out requires at least 2 rewards, got {}",
            rewards.len()
        )));
    }
    if rewards.iter().any(|reward| !reward.is_finite()) {
        return Err(TrainlabError::InvalidConfig(
            "rewards must be finite".to_string(),
        ));
    }
    let total: f64 = rewards.iter().sum();
    let others = (rewards.len() - 1) as f64;
    Ok(rewards
        .iter()
        .map(|reward| reward - (total - reward) / others)
        .collect())
}

/// Reward spread of a group: `max - min`. A zero spread means the
/// group carries no relative signal (the Python performs no update).
pub fn reward_spread(rewards: &[f64]) -> f64 {
    if rewards.is_empty() {
        return 0.0;
    }
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for reward in rewards {
        min = min.min(*reward);
        max = max.max(*reward);
    }
    max - min
}

/// Unbiased pass@k estimate from `n` samples of which `c` pass.
///
/// Errors (never guesses) when `k == 0`, `k > n`, `c > n`, or `n`
/// exceeds [`SAMPLES_MAX`].
pub fn pass_at_k(n: usize, c: usize, k: usize) -> Result<f64, TrainlabError> {
    if n == 0 || n > SAMPLES_MAX {
        return Err(TrainlabError::LimitExceeded(format!(
            "sample count {n} outside 1..={SAMPLES_MAX}"
        )));
    }
    if k == 0 || k > n {
        return Err(TrainlabError::InvalidConfig(format!(
            "k {k} outside 1..={n}"
        )));
    }
    if c > n {
        return Err(TrainlabError::InvalidConfig(format!(
            "passing count {c} exceeds sample count {n}"
        )));
    }
    if c == 0 {
        return Ok(0.0);
    }
    if n - c < k {
        return Ok(1.0);
    }
    // 1 - prod_{i=0}^{k-1} (n - c - i) / (n - i); every factor is in
    // (0, 1], so the product is stable and bounded.
    let mut ratio = 1.0_f64;
    for i in 0..k {
        ratio *= (n - c - i) as f64 / (n - i) as f64;
    }
    Ok(1.0 - ratio)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advantages_sum_to_zero() {
        let rewards = [1.0, 0.0, -0.1, 1.0, 0.0];
        let advantages = leave_one_out_advantages(&rewards).expect("advantages");
        let sum: f64 = advantages.iter().sum();
        assert!(sum.abs() < 1e-9, "advantages must sum to ~0, got {sum}");
        // The passing completions get positive advantage, the
        // invalid one negative — the group-relative sign is the point.
        assert!(advantages[0] > 0.0);
        assert!(advantages[2] < 0.0);
    }

    #[test]
    fn advantages_require_two_rewards() {
        assert!(leave_one_out_advantages(&[1.0]).is_err());
        assert!(leave_one_out_advantages(&[]).is_err());
    }

    #[test]
    fn non_finite_rewards_are_rejected() {
        // Adversarial: a NaN reward must not poison a group's update.
        assert!(leave_one_out_advantages(&[1.0, f64::NAN]).is_err());
        assert!(leave_one_out_advantages(&[1.0, f64::INFINITY]).is_err());
    }

    #[test]
    fn spread_of_uniform_group_is_zero() {
        assert_eq!(reward_spread(&[0.5, 0.5, 0.5]), 0.0);
        assert_eq!(reward_spread(&[]), 0.0);
        assert!((reward_spread(&[1.0, -0.1]) - 1.1).abs() < 1e-12);
    }

    #[test]
    fn pass_at_k_known_values() {
        // All pass -> 1, none pass -> 0, for any k.
        assert_eq!(pass_at_k(8, 8, 4).expect("k"), 1.0);
        assert_eq!(pass_at_k(8, 0, 4).expect("k"), 0.0);
        // n=8, c=4, k=4: 1 - C(4,4)/C(8,4) = 1 - 1/70.
        let value = pass_at_k(8, 4, 4).expect("k");
        assert!((value - (1.0 - 1.0 / 70.0)).abs() < 1e-12);
        // pass@1 is the plain pass rate.
        let value = pass_at_k(8, 3, 1).expect("k");
        assert!((value - 3.0 / 8.0).abs() < 1e-12);
    }

    #[test]
    fn pass_at_k_rejects_bad_shapes() {
        assert!(pass_at_k(8, 4, 0).is_err());
        assert!(pass_at_k(8, 4, 9).is_err());
        assert!(pass_at_k(8, 9, 1).is_err());
        assert!(pass_at_k(0, 0, 1).is_err());
        assert!(pass_at_k(SAMPLES_MAX + 1, 0, 1).is_err());
    }
}
