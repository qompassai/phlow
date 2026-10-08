//! The live evaluator: measures a change-set by running trainlab.
//!
//! Composition, per change-set:
//!
//! 1. The base run configuration is read from `base-config.json` in
//!    the experiment worktree (so a `file_patch` change-set — applied
//!    by [`crate::applying_evaluator::ApplyingEvaluator`] before this
//!    evaluator runs — can itself move the base configuration).
//! 2. A `trainlab_config` change-set's delta JSON is merged over the
//!    base. Only the run-shape fields the delta language names are
//!    recognized (`per_family`, `groups`, `group_size`,
//!    `temperature`, `seed`, `families`, `invalid_penalty`); an
//!    unknown key fails closed — a delta that silently did nothing
//!    would fabricate a measurement.
//! 3. The merged [`RunConfig`] runs on trainlab's frozen dev split
//!    against the incumbent served model through trainlab's own
//!    sampler + bounded reward executor, producing a real trainlab
//!    receipt, written under the evidence directory (refuse-overwrite,
//!    like every evidence writer in the pipeline).
//! 4. The receipt is scored by [`ReceiptEvaluator`] from a bundle
//!    with no trainer evidence: a config delta or file patch does not
//!    call for a training step, so none is claimed.
//!
//! The reward executor executes model-generated code; constructing
//! this evaluator is the operator's acknowledgment (the library form
//! of the trainlab CLI's `--execute-rewards`).

use std::path::PathBuf;

use phlow_trainlab::executor::{Executor, ExecutorConfig};
use phlow_trainlab::reward::RewardConfig;
use phlow_trainlab::runner::{RunConfig, run_experiment};
use phlow_trainlab::sampler::OllamaSampler;
use phlow_trainlab::task::Split;
use serde::Deserialize;

use crate::changeset::{ChangeKind, ChangeSet};
use crate::clock::Clock;
use crate::evaluator::{EvalError, Evaluation, Evaluator};
use crate::receipt_evaluator::{EvaluationBundle, ReceiptEvaluator};

/// Name of the base-configuration file inside the experiment worktree.
pub const BASE_CONFIG_FILE_NAME: &str = "base-config.json";

/// Default Ollama endpoint for the live sampler (loopback).
pub const DEFAULT_SAMPLER_BASE_URL: &str = "http://127.0.0.1:11434";

/// The base run configuration, as stored in `base-config.json`.
/// Defaults are the trainlab CLI's `run` defaults.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BaseConfig {
    /// Tasks per family in the dev split.
    pub per_family: usize,
    /// Number of prompt groups.
    pub groups: usize,
    /// Completions per group.
    pub group_size: usize,
    /// Sampling temperature.
    pub temperature: f64,
    /// Base seed.
    pub seed: u64,
    /// Reward penalty for invalid completions.
    pub invalid_penalty: f64,
    /// Family filter; empty means all families.
    pub families: Vec<String>,
}

impl Default for BaseConfig {
    fn default() -> Self {
        BaseConfig {
            per_family: 4,
            groups: 8,
            group_size: 4,
            temperature: 0.35,
            seed: 42,
            invalid_penalty: -0.1,
            families: Vec::new(),
        }
    }
}

impl BaseConfig {
    fn to_run_config(&self) -> RunConfig {
        RunConfig {
            split: Split::Dev,
            per_family: self.per_family,
            groups: self.groups,
            group_size: self.group_size,
            temperature: self.temperature,
            seed: self.seed,
            reward: RewardConfig {
                invalid_penalty: self.invalid_penalty,
                ..RewardConfig::default()
            },
            families: self.families.clone(),
        }
    }

    /// Merge one `trainlab_config` delta object over this base.
    fn merged_with_delta(&self, delta: &serde_json::Value) -> Result<BaseConfig, EvalError> {
        let object = delta.as_object().ok_or_else(|| {
            EvalError::failed("trainlab_config delta is not a JSON object".to_string())
        })?;
        let mut merged = self.clone();
        for (key, value) in object {
            match key.as_str() {
                "per_family" => merged.per_family = usize_field(key, value)?,
                "groups" => merged.groups = usize_field(key, value)?,
                "group_size" => merged.group_size = usize_field(key, value)?,
                "temperature" => merged.temperature = f64_field(key, value)?,
                "seed" => merged.seed = u64_field(key, value)?,
                "invalid_penalty" => merged.invalid_penalty = f64_field(key, value)?,
                "families" => {
                    merged.families = value
                        .as_array()
                        .ok_or_else(|| {
                            EvalError::failed("delta field families must be an array".to_string())
                        })?
                        .iter()
                        .map(|item| {
                            item.as_str().map(str::to_string).ok_or_else(|| {
                                EvalError::failed(
                                    "delta field families must hold strings".to_string(),
                                )
                            })
                        })
                        .collect::<Result<Vec<String>, EvalError>>()?;
                }
                other => {
                    return Err(EvalError::failed(format!(
                        "delta carries unrecognized field {other:?}; refusing to measure a delta that would silently do nothing"
                    )));
                }
            }
        }
        Ok(merged)
    }
}

fn usize_field(key: &str, value: &serde_json::Value) -> Result<usize, EvalError> {
    value
        .as_u64()
        .and_then(|raw| usize::try_from(raw).ok())
        .ok_or_else(|| {
            EvalError::failed(format!("delta field {key} must be a non-negative integer"))
        })
}

fn u64_field(key: &str, value: &serde_json::Value) -> Result<u64, EvalError> {
    value.as_u64().ok_or_else(|| {
        EvalError::failed(format!("delta field {key} must be a non-negative integer"))
    })
}

fn f64_field(key: &str, value: &serde_json::Value) -> Result<f64, EvalError> {
    value
        .as_f64()
        .ok_or_else(|| EvalError::failed(format!("delta field {key} must be a number")))
}

/// An [`Evaluator`] that runs trainlab for real. See the module docs
/// for the composition and the code-execution acknowledgment.
#[derive(Debug)]
pub struct LiveEvaluator {
    worktree_root: PathBuf,
    evidence_dir: PathBuf,
    sampler_base_url: String,
    model: String,
    runs: u64,
}

impl LiveEvaluator {
    /// A live evaluator measuring against `model` served at
    /// `sampler_base_url`, with the experiment worktree at
    /// `worktree_root` and receipts written under `evidence_dir`.
    #[must_use]
    pub fn new(
        worktree_root: PathBuf,
        evidence_dir: PathBuf,
        sampler_base_url: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        LiveEvaluator {
            worktree_root,
            evidence_dir,
            sampler_base_url: sampler_base_url.into(),
            model: model.into(),
            runs: 0,
        }
    }

    fn base_config(&self) -> Result<BaseConfig, EvalError> {
        let path = self.worktree_root.join(BASE_CONFIG_FILE_NAME);
        let text = std::fs::read_to_string(&path).map_err(|err| {
            EvalError::failed(format!("cannot read base config {}: {err}", path.display()))
        })?;
        serde_json::from_str(&text).map_err(|err| {
            EvalError::failed(format!(
                "base config {} is not valid: {err}",
                path.display()
            ))
        })
    }
}

impl Evaluator for LiveEvaluator {
    fn evaluate(
        &mut self,
        change_set: &ChangeSet,
        clock: &dyn Clock,
    ) -> Result<Evaluation, EvalError> {
        let base = self.base_config()?;
        let effective = match change_set.kind {
            ChangeKind::TrainlabConfig => {
                let delta: serde_json::Value =
                    serde_json::from_str(&change_set.payload).map_err(|err| {
                        EvalError::failed(format!("config delta is not valid JSON: {err}"))
                    })?;
                base.merged_with_delta(&delta)?
            }
            ChangeKind::FilePatch => base,
        };
        let config = effective.to_run_config();
        config
            .validate()
            .map_err(|err| EvalError::failed(format!("effective run config invalid: {err}")))?;
        let sampler = OllamaSampler::new(self.sampler_base_url.clone(), self.model.clone(), false)
            .map_err(|err| EvalError::failed(format!("sampler misconfigured: {err}")))?;
        let executor = Executor::new(ExecutorConfig {
            acknowledge_code_execution: true,
            ..ExecutorConfig::default()
        })
        .map_err(|err| EvalError::failed(format!("reward executor misconfigured: {err}")))?;
        self.runs += 1;
        let run_id = format!("live-{}-{}", change_set.id, self.runs);
        let receipt = run_experiment(&run_id, &config, &sampler, &executor)
            .map_err(|err| EvalError::failed(format!("trainlab run failed: {err}")))?;
        let receipt_path = self.evidence_dir.join(&run_id).join("dev-receipt.json");
        // write_new is the pipeline's evidence writer: atomic,
        // refuse-overwrite, verified after landing — the same call
        // the trainlab CLI makes.
        receipt
            .write_new(&receipt_path)
            .map_err(|err| EvalError::failed(format!("cannot write dev receipt: {err}")))?;
        let bundle = EvaluationBundle {
            dev_receipt_path: receipt_path,
            trainer: None,
        };
        let mut scorer = ReceiptEvaluator::new().with_bundle(change_set.id.clone(), bundle);
        scorer.evaluate(change_set, clock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_merges_recognized_fields() {
        let base = BaseConfig::default();
        let delta = serde_json::json!({"temperature": 0.7, "seed": 7});
        let merged = base.merged_with_delta(&delta).expect("merges");
        assert_eq!(merged.temperature, 0.7);
        assert_eq!(merged.seed, 7);
        assert_eq!(merged.groups, base.groups);
    }

    #[test]
    fn delta_rejects_unrecognized_fields() {
        let base = BaseConfig::default();
        let delta = serde_json::json!({"learning_rate": 0.001});
        let err = base
            .merged_with_delta(&delta)
            .expect_err("must fail closed");
        assert!(err.message.contains("unrecognized"), "{}", err.message);
    }

    #[test]
    fn delta_rejects_wrong_types() {
        let base = BaseConfig::default();
        let delta = serde_json::json!({"groups": "eight"});
        assert!(base.merged_with_delta(&delta).is_err());
        let delta = serde_json::json!({"families": "all"});
        assert!(base.merged_with_delta(&delta).is_err());
    }

    #[test]
    fn base_config_defaults_match_trainlab_cli() {
        let base = BaseConfig::default();
        assert_eq!(base.per_family, 4);
        assert_eq!(base.groups, 8);
        assert_eq!(base.group_size, 4);
        assert_eq!(base.temperature, 0.35);
        let config = base.to_run_config();
        assert_eq!(config.split, Split::Dev);
        config.validate().expect("defaults are a valid run config");
    }
}
