//! Best-of-N trajectory verifier: pick the candidate a contrastive head
//! endorses, with exact tie handling.
//!
//! Plain words: when the agent rolls out several candidate trajectories, a
//! cheap scalar per step (here: the head's state/action cosine) turns into
//! one trajectory score, and best-of-N keeps the best. CLM's evaluator does
//! this in `/tmp/CLM/evaluation/bon_eval.py`:
//!
//! - `aggregate`: one benchmark-independent trajectory score — the mean over
//!   the final `--window` steps (`bon_eval.py:56-61`).
//! - `best_of_n`: the *exact* expected reward of a uniform N-subset with
//!   uniform breaking of top-score ties, plus random and oracle baselines
//!   (`bon_eval.py:64-80`). Short candidate groups are retained: the budget
//!   is `min(n, group size)` (`bon_eval.py:88-96`).
//!
//! This module ports that selection math to Rust. Step scores are `f64`;
//! rewards are binary (`passed`), exactly as `best_of_n` requires
//! (`bon_eval.py:71`, "finite scores and binary outcomes are required").
//!
//! # Exactness note
//!
//! `bon_eval.py` computes binomial coefficients as Python big integers and
//! divides. Here `C(a, n) / C(b, n)` is evaluated as a product of `n`
//! ratios in `f64` ([`comb_ratio`]): the *formula* is identical, so tie
//! handling is exact in the combinatorial sense; only floating-point
//! rounding differs, and it is deterministic. Inputs are bounded so the
//! product cannot overflow to infinity.
//!
//! # Bounds
//!
//! - [`CANDIDATES_MAX`] candidates per selection; [`STEP_SCORES_MAX`] step
//!   scores per trajectory; [`WINDOW_MAX`] window size; [`NAME_CHARS_MAX`]
//!   chars per trajectory name.
//!
//! # Unsafe policy
//!
//! This module forbids unsafe code (crate-level `#![forbid(unsafe_code)]`).

use std::fmt;

/// Maximum candidates in one best-of-N selection.
pub const CANDIDATES_MAX: usize = 100_000;

/// Maximum step scores in one trajectory.
pub const STEP_SCORES_MAX: usize = 100_000;

/// Maximum final-window size.
pub const WINDOW_MAX: usize = 10_000;

/// Maximum chars of a trajectory name.
pub const NAME_CHARS_MAX: usize = 256;

/// Failures of trajectory scoring and selection. All are caller errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionError {
    /// No candidates were offered.
    EmptyCandidates,
    /// More than [`CANDIDATES_MAX`] candidates.
    TooManyCandidates {
        /// Candidates offered.
        count: usize,
    },
    /// Budget was zero or exceeded the candidate count.
    BudgetOutOfRange {
        /// Requested budget.
        budget: usize,
        /// Candidates available.
        count: usize,
    },
    /// Window was zero or exceeded [`WINDOW_MAX`].
    InvalidWindow {
        /// Requested window.
        window: usize,
    },
    /// A trajectory had no step scores.
    EmptyTrajectory,
    /// A trajectory had more than [`STEP_SCORES_MAX`] step scores.
    TooManySteps {
        /// Step scores offered.
        count: usize,
    },
    /// A step score was non-finite.
    NonFiniteScore,
    /// A trajectory name was empty or over [`NAME_CHARS_MAX`] chars.
    BadName,
}

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelectionError::EmptyCandidates => write!(f, "no candidates to select from"),
            SelectionError::TooManyCandidates { count } => {
                write!(
                    f,
                    "{count} candidates exceeds CANDIDATES_MAX={CANDIDATES_MAX}"
                )
            }
            SelectionError::BudgetOutOfRange { budget, count } => {
                write!(f, "budget {budget} out of range for {count} candidates")
            }
            SelectionError::InvalidWindow { window } => {
                write!(f, "window must be in 1..=WINDOW_MAX, got {window}")
            }
            SelectionError::EmptyTrajectory => {
                write!(f, "cannot aggregate an empty trajectory")
            }
            SelectionError::TooManySteps { count } => {
                write!(
                    f,
                    "{count} step scores exceeds STEP_SCORES_MAX={STEP_SCORES_MAX}"
                )
            }
            SelectionError::NonFiniteScore => write!(f, "step scores must be finite"),
            SelectionError::BadName => {
                write!(f, "trajectory name must be 1..=NAME_CHARS_MAX chars")
            }
        }
    }
}

impl std::error::Error for SelectionError {}

/// One benchmark-independent trajectory score: the mean over the final
/// `window` steps. Mirrors `aggregate` (`/tmp/CLM/evaluation/bon_eval.py:56-61`).
///
/// When `window` exceeds the trajectory length, the whole trajectory is
/// averaged — Python's `scores[-window:]` clamps the same way.
pub fn final_window_mean(scores: &[f64], window: usize) -> Result<f64, SelectionError> {
    if window == 0 || window > WINDOW_MAX {
        return Err(SelectionError::InvalidWindow { window });
    }
    if scores.is_empty() {
        return Err(SelectionError::EmptyTrajectory);
    }
    if scores.len() > STEP_SCORES_MAX {
        return Err(SelectionError::TooManySteps {
            count: scores.len(),
        });
    }
    if scores.iter().any(|s| !s.is_finite()) {
        return Err(SelectionError::NonFiniteScore);
    }
    let tail = &scores[scores.len().saturating_sub(window.min(scores.len()))..];
    Ok(tail.iter().sum::<f64>() / tail.len() as f64)
}

/// `C(a, n) / C(b, n)` for `0 <= n <= a <= b`, as a product of `n` ratios in
/// `f64`. Returns `0.0` when `a < n` (Python's `choose` returns 0 then).
/// Deterministic; only floating-point rounding differs from the big-int
/// original.
fn comb_ratio(a: u64, b: u64, n: u64) -> f64 {
    debug_assert!(n <= b, "budget cannot exceed the population");
    if a < n {
        return 0.0;
    }
    let mut ratio = 1.0f64;
    for i in 0..n {
        ratio *= (a - i) as f64 / (b - i) as f64;
    }
    ratio
}

/// One scored candidate: a trajectory score and its binary outcome.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoredCandidate {
    /// Trajectory score (e.g. a final-window mean).
    pub score: f64,
    /// Whether the trajectory passed.
    pub passed: bool,
}

/// The best-of-N report. Mirrors the dict `best_of_n` returns
/// (`/tmp/CLM/evaluation/bon_eval.py:64-80`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionReport {
    /// Expected reward of the uniform-N-subset selection with uniform
    /// tie-breaking.
    pub expected_selected: f64,
    /// Mean reward over all candidates (random pick).
    pub random_baseline: f64,
    /// Probability a uniform N-subset contains a passing candidate.
    pub oracle: f64,
}

/// Exact expectation for a uniform N-subset and uniform breaking of
/// top-score ties. Mirrors `best_of_n`
/// (`/tmp/CLM/evaluation/bon_eval.py:64-80`): candidates are grouped into
/// blocks of equal score (ascending); the chance the selected subset's best
/// block is block `k` equals `(C(lower+|block|, n) - C(lower, n)) / C(total, n)`.
pub fn best_of_n(
    candidates: &[ScoredCandidate],
    budget: usize,
) -> Result<SelectionReport, SelectionError> {
    let total = candidates.len();
    if total == 0 {
        return Err(SelectionError::EmptyCandidates);
    }
    if total > CANDIDATES_MAX {
        return Err(SelectionError::TooManyCandidates { count: total });
    }
    if budget == 0 || budget > total {
        return Err(SelectionError::BudgetOutOfRange {
            budget,
            count: total,
        });
    }
    if candidates.iter().any(|c| !c.score.is_finite()) {
        return Err(SelectionError::NonFiniteScore);
    }
    // Ascending by score; f64::total_cmp is a total order, no unwrap needed.
    let mut order: Vec<usize> = (0..total).collect();
    order.sort_by(|&a, &b| candidates[a].score.total_cmp(&candidates[b].score));

    let total_u = total as u64;
    let budget_u = budget as u64;
    let mut expected = 0.0f64;
    let mut lower = 0u64;
    let mut start = 0;
    while start < total {
        let mut end = start + 1;
        while end < total && candidates[order[end]].score == candidates[order[start]].score {
            end += 1;
        }
        let block = &order[start..end];
        let block_len = block.len() as u64;
        let probability =
            comb_ratio(lower + block_len, total_u, budget_u) - comb_ratio(lower, total_u, budget_u);
        let mean_reward = block
            .iter()
            .map(|&i| f64::from(candidates[i].passed as u8))
            .sum::<f64>()
            / block_len as f64;
        expected += probability * mean_reward;
        lower += block_len;
        start = end;
    }
    let passed = candidates.iter().filter(|c| c.passed).count();
    Ok(SelectionReport {
        expected_selected: expected,
        random_baseline: passed as f64 / total as f64,
        oracle: 1.0 - comb_ratio((total - passed) as u64, total_u, budget_u),
    })
}

/// One candidate trajectory: its per-step head scores and its outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct Trajectory {
    /// Trajectory name (for the winners list).
    pub name: String,
    /// Per-step scores, in step order.
    pub step_scores: Vec<f64>,
    /// Whether the trajectory passed.
    pub passed: bool,
}

/// The verifier's decision.
#[derive(Debug, Clone, PartialEq)]
pub struct Verification {
    /// Expected reward of the best-of-N selection.
    pub expected_reward: f64,
    /// Mean reward over all trajectories.
    pub random_baseline: f64,
    /// Probability a uniform budget-subset contains a passing trajectory.
    pub oracle: f64,
    /// Names tied at the maximum trajectory score, in input order
    /// (mirrors `selection_rate`'s picks, `bon_eval.py:88-96`).
    pub winners: Vec<String>,
}

/// Verify a set of candidate trajectories: score each by its final-window
/// mean, then run best-of-N. The effective budget is `min(budget, len)` —
/// short candidate groups are retained, like `selection_rate`
/// (`/tmp/CLM/evaluation/bon_eval.py:92`).
pub fn verify(
    trajectories: &[Trajectory],
    budget: usize,
    window: usize,
) -> Result<Verification, SelectionError> {
    if trajectories.is_empty() {
        return Err(SelectionError::EmptyCandidates);
    }
    if trajectories.len() > CANDIDATES_MAX {
        return Err(SelectionError::TooManyCandidates {
            count: trajectories.len(),
        });
    }
    if budget == 0 {
        return Err(SelectionError::BudgetOutOfRange {
            budget,
            count: trajectories.len(),
        });
    }
    for trajectory in trajectories {
        if trajectory.name.is_empty() || trajectory.name.chars().count() > NAME_CHARS_MAX {
            return Err(SelectionError::BadName);
        }
    }
    let mut scored = Vec::with_capacity(trajectories.len());
    for trajectory in trajectories {
        scored.push(ScoredCandidate {
            score: final_window_mean(&trajectory.step_scores, window)?,
            passed: trajectory.passed,
        });
    }
    let effective = budget.min(trajectories.len());
    let report = best_of_n(&scored, effective)?;
    let best = scored
        .iter()
        .map(|c| c.score)
        .fold(f64::NEG_INFINITY, f64::max);
    let winners: Vec<String> = trajectories
        .iter()
        .zip(scored.iter())
        .filter(|(_, scored)| scored.score == best)
        .map(|(trajectory, _)| trajectory.name.clone())
        .collect();
    Ok(Verification {
        expected_reward: report.expected_selected,
        random_baseline: report.random_baseline,
        oracle: report.oracle,
        winners,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(score: f64, passed: bool) -> ScoredCandidate {
        ScoredCandidate { score, passed }
    }

    // ---- validation: the selection math ----

    #[test]
    fn final_window_mean_full_window() {
        assert_eq!(final_window_mean(&[1.0, 2.0, 3.0, 4.0], 2), Ok(3.5));
    }

    #[test]
    fn final_window_mean_clamps_long_window() {
        // Python's scores[-window:] with window > len averages everything.
        assert_eq!(final_window_mean(&[2.0, 4.0], 12), Ok(3.0));
    }

    #[test]
    fn best_of_n_single_candidate() {
        let report = best_of_n(&[candidate(0.9, true)], 1).unwrap();
        assert_eq!(report.expected_selected, 1.0);
        assert_eq!(report.random_baseline, 1.0);
        assert_eq!(report.oracle, 1.0);
    }

    #[test]
    fn best_of_n_tie_blocks_use_uniform_expectation() {
        // Scores [1.0 pass, 1.0 fail, 0.0 pass], budget 1: a 1-subset is a
        // uniform single pick, so the expectation is the mean reward, 2/3.
        // Block-wise: the {1.0} block is hit with probability
        // (C(2,1) - C(0,1))/C(3,1) = 2/3 at mean reward 1/2, and the {0.0}
        // block with probability 1/3 at mean reward 1: 2/3*1/2 + 1/3 = 2/3.
        // This is bon_eval's exact tie handling, hand-computed.
        let report = best_of_n(
            &[
                candidate(1.0, true),
                candidate(1.0, false),
                candidate(0.0, true),
            ],
            1,
        )
        .unwrap();
        assert!(
            (report.expected_selected - 2.0 / 3.0).abs() < 1e-12,
            "{report:?}"
        );
        assert!((report.random_baseline - 2.0 / 3.0).abs() < 1e-12);
        // Oracle: a 1-subset contains a passing candidate with prob 2/3.
        assert!((report.oracle - 2.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn best_of_n_matches_brute_force_enumeration() {
        // 4 candidates, budget 2: enumerate all 6 subsets; within a subset
        // the winner is the max score, ties split uniformly.
        let group = [
            candidate(0.8, true),
            candidate(0.8, false),
            candidate(0.3, true),
            candidate(0.1, false),
        ];
        let report = best_of_n(&group, 2).unwrap();
        let mut brute = 0.0f64;
        let mut subsets = 0u32;
        for mask in 0..(1u32 << group.len()) {
            if mask.count_ones() != 2 {
                continue;
            }
            subsets += 1;
            let best = (0..group.len())
                .filter(|&i| mask & (1 << i) != 0)
                .map(|i| group[i].score)
                .fold(f64::NEG_INFINITY, f64::max);
            let tied: Vec<_> = (0..group.len())
                .filter(|&i| mask & (1 << i) != 0 && group[i].score == best)
                .collect();
            brute += tied
                .iter()
                .map(|&i| f64::from(group[i].passed as u8))
                .sum::<f64>()
                / tied.len() as f64;
        }
        brute /= f64::from(subsets);
        assert!(
            (report.expected_selected - brute).abs() < 1e-9,
            "{report:?} vs {brute}"
        );
    }

    #[test]
    fn verify_reports_winners_and_baselines() {
        let trajectories = vec![
            Trajectory {
                name: "a".to_string(),
                step_scores: vec![0.1, 0.9],
                passed: true,
            },
            Trajectory {
                name: "b".to_string(),
                step_scores: vec![0.1, 0.9],
                passed: false,
            },
            Trajectory {
                name: "c".to_string(),
                step_scores: vec![0.0, 0.2],
                passed: true,
            },
        ];
        let verification = verify(&trajectories, 1, 2).unwrap();
        assert_eq!(verification.winners, vec!["a".to_string(), "b".to_string()]);
        // Budget 1 is a uniform single pick: expected reward is the mean, 2/3.
        assert!((verification.expected_reward - 2.0 / 3.0).abs() < 1e-12);
        assert!((verification.random_baseline - 2.0 / 3.0).abs() < 1e-12);
    }

    // ---- adversarial: invalid selection inputs ----

    #[test]
    fn empty_trajectory_rejected() {
        assert_eq!(
            final_window_mean(&[], 4),
            Err(SelectionError::EmptyTrajectory)
        );
    }

    #[test]
    fn window_zero_rejected() {
        assert_eq!(
            final_window_mean(&[1.0], 0),
            Err(SelectionError::InvalidWindow { window: 0 })
        );
    }

    #[test]
    fn nonfinite_score_rejected() {
        assert_eq!(
            final_window_mean(&[1.0, f64::NAN], 2),
            Err(SelectionError::NonFiniteScore)
        );
        assert_eq!(
            best_of_n(&[candidate(f64::INFINITY, true)], 1),
            Err(SelectionError::NonFiniteScore)
        );
    }

    #[test]
    fn budget_out_of_range_rejected() {
        let group = [candidate(0.5, true)];
        assert_eq!(
            best_of_n(&group, 0),
            Err(SelectionError::BudgetOutOfRange {
                budget: 0,
                count: 1
            })
        );
        assert_eq!(
            best_of_n(&group, 2),
            Err(SelectionError::BudgetOutOfRange {
                budget: 2,
                count: 1
            })
        );
    }

    #[test]
    fn verify_empty_candidates_rejected() {
        assert_eq!(verify(&[], 1, 4), Err(SelectionError::EmptyCandidates));
    }

    #[test]
    fn too_many_candidates_rejected() {
        let group = vec![candidate(0.5, true); CANDIDATES_MAX + 1];
        assert_eq!(
            best_of_n(&group, 1),
            Err(SelectionError::TooManyCandidates {
                count: CANDIDATES_MAX + 1
            })
        );
    }
}
