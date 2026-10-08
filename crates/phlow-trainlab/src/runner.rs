//! The run loop and pass@k evaluation.
//!
//! The runner performs every step of the video's RL loop that phlow
//! can honestly perform — schedule prompt groups, sample completions,
//! execute them for rewards, compute leave-one-out advantages — and
//! records the result as a [`RunReceipt`]. It performs **no weight
//! update**: `updated` in a group record means the group carried
//! signal (non-zero reward spread), i.e. a trainer backend *would*
//! have stepped from it.

use std::time::Instant;

use serde::Serialize;

use crate::error::TrainlabError;
use crate::executor::Executor;
use crate::group::{leave_one_out_advantages, pass_at_k, reward_spread};
use crate::receipt::{GroupRecord, RunReceipt, config_sha256};
use crate::reward::{EvalStatus, RewardConfig};
use crate::rng::SplitMix64;
use crate::sampler::{GROUP_SIZE_MAX, Sampler};
use crate::task::{CodingTask, PER_FAMILY_MAX, Split, family_by_name, frozen_tasks};

/// Maximum prompt groups in one run.
pub const GROUPS_MAX: usize = 256;

/// Configuration for one experiment run. Serialized into the
/// receipt verbatim (and hashed), so field order is part of the
/// receipt format.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunConfig {
    /// Split to draw prompts from.
    pub split: Split,
    /// Tasks per family in the split.
    pub per_family: usize,
    /// Number of prompt groups.
    pub groups: usize,
    /// Completions per group (RLOO requires at least 2).
    pub group_size: usize,
    /// Sampling temperature, `0.0..=2.0`.
    pub temperature: f64,
    /// Base seed for scheduling and sampling.
    pub seed: u64,
    /// Reward semantics.
    pub reward: RewardConfig,
    /// Family filter; empty means all families.
    pub families: Vec<String>,
}

impl RunConfig {
    /// Validate every bound before any sampling or execution.
    pub fn validate(&self) -> Result<(), TrainlabError> {
        if self.groups == 0 || self.groups > GROUPS_MAX {
            return Err(TrainlabError::LimitExceeded(format!(
                "groups {} outside 1..={GROUPS_MAX}",
                self.groups
            )));
        }
        if self.group_size < 2 || self.group_size > GROUP_SIZE_MAX {
            return Err(TrainlabError::LimitExceeded(format!(
                "group_size {} outside 2..={GROUP_SIZE_MAX} (RLOO requires at least 2)",
                self.group_size
            )));
        }
        if !self.temperature.is_finite() || !(0.0..=2.0).contains(&self.temperature) {
            return Err(TrainlabError::InvalidConfig(format!(
                "temperature {} outside 0.0..=2.0",
                self.temperature
            )));
        }
        if self.per_family == 0 || self.per_family > PER_FAMILY_MAX {
            return Err(TrainlabError::LimitExceeded(format!(
                "per_family {} outside 1..={PER_FAMILY_MAX}",
                self.per_family
            )));
        }
        self.reward.validate()?;
        for family in &self.families {
            family_by_name(family)?;
        }
        Ok(())
    }
}

/// Select the frozen tasks for a split, applying the family filter.
/// An empty result (filter matched nothing) is an error.
pub fn select_tasks(
    split: Split,
    per_family: usize,
    families: &[String],
) -> Result<Vec<CodingTask>, TrainlabError> {
    let tasks = frozen_tasks(split, per_family)?;
    if families.is_empty() {
        return Ok(tasks);
    }
    for family in families {
        family_by_name(family)?;
    }
    let selected: Vec<CodingTask> = tasks
        .into_iter()
        .filter(|task| families.contains(&task.family))
        .collect();
    if selected.is_empty() {
        return Err(TrainlabError::InvalidConfig(
            "family filter matched no tasks".to_string(),
        ));
    }
    Ok(selected)
}

/// Build the prompt schedule: shuffled cycles of the task list,
/// truncated to `groups` — the port of the Python `schedule`.
pub fn schedule(tasks: &[CodingTask], groups: usize, seed: u64) -> Vec<CodingTask> {
    let mut rng = SplitMix64::new(seed);
    let mut result = Vec::with_capacity(groups);
    while result.len() < groups {
        let mut cycle = tasks.to_vec();
        rng.shuffle(&mut cycle);
        result.extend(cycle);
    }
    result.truncate(groups);
    result
}

/// Run one experiment and return its receipt.
///
/// Sampling seeds follow the Python: group `g` samples with
/// `seed + g * 17`.
pub fn run_experiment(
    run_id: &str,
    config: &RunConfig,
    sampler: &dyn Sampler,
    executor: &Executor,
) -> Result<RunReceipt, TrainlabError> {
    config.validate()?;
    let tasks = select_tasks(config.split, config.per_family, &config.families)?;
    let schedule = schedule(&tasks, config.groups, config.seed);
    let started = Instant::now();
    let mut records = Vec::with_capacity(config.groups);
    let mut samples_total = 0_usize;
    for (index, task) in schedule.iter().enumerate() {
        let group_number = index + 1;
        let group_started = Instant::now();
        let completions = sampler.sample(
            &task.prompt,
            config.group_size,
            config.temperature,
            config.seed.wrapping_add((group_number as u64) * 17),
        )?;
        assert_eq!(
            completions.len(),
            config.group_size,
            "sampler contract: exactly group_size completions"
        );
        let mut rewards = Vec::with_capacity(config.group_size);
        let mut exact_rollouts = 0_usize;
        for completion in &completions {
            let evaluation = executor.evaluate(task, completion)?;
            rewards.push(crate::reward::reward_for(&evaluation, &config.reward));
            if completion.trim() == task.reference_completion.trim() {
                exact_rollouts += 1;
            }
        }
        samples_total += completions.len();
        let advantages = leave_one_out_advantages(&rewards)?;
        let spread = reward_spread(&rewards);
        records.push(GroupRecord {
            group: group_number,
            task_id: task.task_id.clone(),
            family: task.family.clone(),
            rewards,
            advantages,
            reward_spread: spread,
            updated: spread > 0.0,
            exact_rollouts,
            seconds: group_started.elapsed().as_secs_f64(),
        });
    }
    Ok(RunReceipt {
        schema: crate::receipt::RECEIPT_SCHEMA,
        run_id: run_id.to_string(),
        algorithm: "RLOO",
        sampler_id: sampler.id(),
        split: config.split.name().to_string(),
        config: serde_json::to_value(config)?,
        config_sha256: config_sha256(config)?,
        groups: records,
        samples_total,
        elapsed_seconds: started.elapsed().as_secs_f64(),
    })
}

/// Per-task pass@k row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskPassk {
    /// Task id.
    pub task_id: String,
    /// Family.
    pub family: String,
    /// Samples drawn (`n`).
    pub samples: usize,
    /// Samples passing all hidden cases (`c`).
    pub passing: usize,
    /// Unbiased pass@1 estimate.
    pub pass_at_1: f64,
    /// Unbiased pass@k estimate.
    pub pass_at_k: f64,
}

/// A pass@k evaluation report over a set of tasks.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PasskReport {
    /// Split the tasks came from.
    pub split: String,
    /// The `k` of pass@k.
    pub k: usize,
    /// Per-task rows, in task order.
    pub tasks: Vec<TaskPassk>,
    /// Mean of per-task pass@1 estimates.
    pub mean_pass_at_1: f64,
    /// Mean of per-task pass@k estimates.
    pub mean_pass_at_k: f64,
}

/// Evaluate pass@k for `tasks`: sample `samples` completions per
/// task and count those whose evaluation status is `Passed`.
///
/// Mirrors `evaluate_passk.py`: pass means *all* hidden cases pass,
/// regardless of the reward mode used for training runs.
pub fn evaluate_passk(
    tasks: &[CodingTask],
    sampler: &dyn Sampler,
    executor: &Executor,
    samples: usize,
    k: usize,
    temperature: f64,
    seed: u64,
) -> Result<PasskReport, TrainlabError> {
    if tasks.is_empty() {
        return Err(TrainlabError::InvalidConfig(
            "pass@k evaluation needs at least one task".to_string(),
        ));
    }
    let mut rows = Vec::with_capacity(tasks.len());
    for (index, task) in tasks.iter().enumerate() {
        let completions = sampler.sample(
            &task.prompt,
            samples,
            temperature,
            seed.wrapping_add((index as u64) * 31),
        )?;
        let mut passing = 0_usize;
        for completion in &completions {
            let evaluation = executor.evaluate(task, completion)?;
            if evaluation.status == EvalStatus::Passed {
                passing += 1;
            }
        }
        rows.push(TaskPassk {
            task_id: task.task_id.clone(),
            family: task.family.clone(),
            samples,
            passing,
            pass_at_1: pass_at_k(samples, passing, 1)?,
            pass_at_k: pass_at_k(samples, passing, k)?,
        });
    }
    let count = rows.len() as f64;
    Ok(PasskReport {
        split: tasks[0].split.clone(),
        k,
        mean_pass_at_1: rows.iter().map(|row| row.pass_at_1).sum::<f64>() / count,
        mean_pass_at_k: rows.iter().map(|row| row.pass_at_k).sum::<f64>() / count,
        tasks: rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{Executor, ExecutorConfig};
    use crate::reward::RewardConfig;
    use crate::sampler::ScriptedSampler;
    use std::process::{Command, Stdio};

    fn python3_available() -> bool {
        Command::new("python3")
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn executor() -> Executor {
        Executor::new(ExecutorConfig {
            acknowledge_code_execution: true,
            ..ExecutorConfig::default()
        })
        .expect("executor")
    }

    fn base_config() -> RunConfig {
        RunConfig {
            split: Split::Rl,
            per_family: 1,
            groups: 2,
            group_size: 4,
            temperature: 0.35,
            seed: 99,
            reward: RewardConfig::default(),
            families: vec!["increment".to_string()],
        }
    }

    #[test]
    fn config_bounds_are_validated() {
        let mut config = base_config();
        assert!(config.validate().is_ok());
        config.group_size = 1;
        assert!(config.validate().is_err(), "RLOO needs group >= 2");
        config = base_config();
        config.groups = GROUPS_MAX + 1;
        assert!(config.validate().is_err());
        config = base_config();
        config.families = vec!["nope".to_string()];
        assert!(config.validate().is_err());
    }

    #[test]
    fn schedule_is_deterministic_and_complete() {
        let tasks = select_tasks(Split::Dev, 1, &[]).expect("tasks");
        let a = schedule(&tasks, 20, 5);
        let b = schedule(&tasks, 20, 5);
        assert_eq!(a.len(), 20);
        assert_eq!(
            a.iter().map(|task| &task.task_id).collect::<Vec<_>>(),
            b.iter().map(|task| &task.task_id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn run_records_rewards_and_advantages() {
        if !python3_available() {
            eprintln!("SKIP: python3 not available for runner integration test");
            return;
        }
        // Scripted sampler alternates the reference body (reward 1.0)
        // with a valid-but-wrong body (reward 0.0): every group must
        // show both, positive spread, and an `updated` flag.
        let sampler = ScriptedSampler::new(
            "scripted",
            vec![
                "\n    return x + 1\n".to_string(),
                "\n    return x\n".to_string(),
            ],
        )
        .expect("sampler");
        let receipt =
            run_experiment("test-run", &base_config(), &sampler, &executor()).expect("run");
        assert_eq!(receipt.groups.len(), 2);
        assert_eq!(receipt.samples_total, 8);
        for group in &receipt.groups {
            assert_eq!(group.rewards, vec![1.0, 0.0, 1.0, 0.0]);
            assert!(group.updated);
            assert_eq!(group.exact_rollouts, 2);
            let sum: f64 = group.advantages.iter().sum();
            assert!(sum.abs() < 1e-9);
        }
        assert_eq!(receipt.algorithm, "RLOO");
        assert_eq!(receipt.config_sha256.len(), 64);
    }

    #[test]
    fn uniform_group_records_no_update() {
        if !python3_available() {
            eprintln!("SKIP: python3 not available for runner integration test");
            return;
        }
        // Adversarial in the video's sense: when every completion in
        // a group earns the same reward there is no relative signal,
        // and the receipt must say no update would have happened.
        let sampler = ScriptedSampler::new("scripted", vec!["\n    return x\n".to_string()])
            .expect("sampler");
        let receipt =
            run_experiment("test-run", &base_config(), &sampler, &executor()).expect("run");
        for group in &receipt.groups {
            assert!(!group.updated);
            assert_eq!(group.reward_spread, 0.0);
        }
    }

    #[test]
    fn passk_report_counts_passes() {
        if !python3_available() {
            eprintln!("SKIP: python3 not available for runner integration test");
            return;
        }
        let tasks = select_tasks(Split::Dev, 1, &["increment".to_string()]).expect("tasks");
        let sampler = ScriptedSampler::new(
            "scripted",
            vec![
                "\n    return x + 1\n".to_string(),
                "\n    return x\n".to_string(),
            ],
        )
        .expect("sampler");
        let report = evaluate_passk(&tasks, &sampler, &executor(), 4, 2, 0.35, 7).expect("report");
        assert_eq!(report.tasks.len(), 1);
        assert_eq!(report.tasks[0].passing, 2);
        assert!((report.tasks[0].pass_at_1 - 0.5).abs() < 1e-12);
        // pass@2 from n=4, c=2: 1 - C(2,2)/C(4,2) = 1 - 1/6.
        assert!((report.tasks[0].pass_at_k - (1.0 - 1.0 / 6.0)).abs() < 1e-12);
    }
}
