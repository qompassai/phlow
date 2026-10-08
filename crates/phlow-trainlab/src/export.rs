//! The per-group export: the trainer-backend input surface of a run.
//!
//! A [`crate::receipt::RunReceipt`] deliberately keeps only
//! rewards/advantages per group — enough to audit the statistics,
//! not enough to train from. A trainer backend needs the text: the
//! exact sampling prompt and every completion, aligned with the
//! rewards and advantages the run assigned them. This module is that
//! surface. An export is written atomically and **never overwritten**
//! (same discipline as the receipt), and it carries the run's
//! `config_sha256`, computed with [`crate::receipt::config_sha256`],
//! so an export verifiably ties to exactly one run receipt:
//! [`GroupsExport::validate_against_receipt`] checks the tie field by
//! field, including every reward and advantage.
//!
//! Loading is a constructor, not a bypass: [`GroupsExport::load`]
//! re-validates schema, shapes (completions/rewards/advantages aligned
//! and at least 2 per group — RLOO's minimum), finiteness, and the
//! internal consistency of `reward_spread` / `updated`. A truncated
//! or tampered export is a typed error, never a partial training set.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::TrainlabError;
use crate::receipt::RunReceipt;
use crate::runner::GROUPS_MAX;

/// Export schema identifier, versioned from the first build.
pub const GROUPS_SCHEMA: &str = "phlow.trainlab.groups/v1";

/// Tolerance for the `reward_spread` consistency check: the spread is
/// recomputed from the rewards and must agree within this absolute
/// tolerance (the values are small-magnitude f64 rewards; 1e-9 is far
/// below any reward quantum the crate produces).
const SPREAD_TOLERANCE: f64 = 1e-9;

/// One prompt group's export record: everything a trainer backend
/// needs to take the step the runner only recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupExportRecord {
    /// 1-based group index within the run (matches the receipt).
    pub group: usize,
    /// Task the group was sampled for.
    pub task_id: String,
    /// Task family.
    pub family: String,
    /// The exact prompt the sampler was given
    /// ([`crate::task::CodingTask::prompt`]).
    pub prompt: String,
    /// Sampled completions, in sampling order.
    pub completions: Vec<String>,
    /// Per-completion rewards, aligned with `completions`.
    pub rewards: Vec<f64>,
    /// Leave-one-out advantages, aligned with `completions`.
    pub advantages: Vec<f64>,
    /// `max(rewards) - min(rewards)`; zero means no relative signal.
    pub reward_spread: f64,
    /// Whether this group carried signal (spread > 0).
    pub updated: bool,
}

// Layout pin (2026-10-08 struct-size audit): 168 bytes is the floor for
// these fields under rustc's default repr, and an export holds at most
// GROUPS_MAX (256) records. Field order is serde-visible (it is the
// export format); this pin makes a growth in the record fail loudly.
const _: () = assert!(std::mem::size_of::<GroupExportRecord>() == 168);

/// A complete group export for one run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupsExport {
    /// Schema identifier ([`GROUPS_SCHEMA`]).
    pub schema: String,
    /// Run identifier, matching the receipt's `run_id`.
    pub run_id: String,
    /// SHA-256 of the canonical JSON of the run config — the same
    /// value the receipt carries, computed by the same helper.
    pub config_sha256: String,
    /// Split the run drew prompts from.
    pub split: String,
    /// Sampler identifier, matching the receipt's `sampler_id`.
    pub sampler_id: String,
    /// Per-group records, in run order.
    pub groups: Vec<GroupExportRecord>,
}

impl GroupsExport {
    /// Validate the export's internal consistency. Every boundary
    /// value is checked with ordinary control flow; a failure leaves
    /// no partial state (the value is simply rejected).
    pub fn validate(&self) -> Result<(), TrainlabError> {
        if self.schema != GROUPS_SCHEMA {
            return Err(TrainlabError::Export(format!(
                "unknown groups schema {:?}, expected {GROUPS_SCHEMA:?}",
                self.schema
            )));
        }
        if self.run_id.is_empty() || self.split.is_empty() || self.sampler_id.is_empty() {
            return Err(TrainlabError::Export(
                "export run_id, split, and sampler_id must be non-empty".to_string(),
            ));
        }
        if self.config_sha256.len() != 64
            || !self
                .config_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(TrainlabError::Export(
                "export config_sha256 must be 64 lowercase hex characters".to_string(),
            ));
        }
        if self.groups.is_empty() || self.groups.len() > GROUPS_MAX {
            return Err(TrainlabError::Export(format!(
                "export group count {} outside 1..={GROUPS_MAX}",
                self.groups.len()
            )));
        }
        for (index, record) in self.groups.iter().enumerate() {
            record.validate(index + 1)?;
        }
        Ok(())
    }

    /// Check the export against the receipt of the same run: header
    /// fields must be identical and every group's rewards, advantages,
    /// spread, and `updated` flag must match the receipt exactly.
    /// This is the tie that makes an export evidence for *that* run
    /// and no other.
    pub fn validate_against_receipt(&self, receipt: &RunReceipt) -> Result<(), TrainlabError> {
        self.validate()?;
        if self.run_id != receipt.run_id
            || self.config_sha256 != receipt.config_sha256
            || self.split != receipt.split
            || self.sampler_id != receipt.sampler_id
        {
            return Err(TrainlabError::Export(
                "export header does not match receipt (run_id/config_sha256/split/sampler_id)"
                    .to_string(),
            ));
        }
        if self.groups.len() != receipt.groups.len() {
            return Err(TrainlabError::Export(format!(
                "export has {} groups, receipt has {}",
                self.groups.len(),
                receipt.groups.len()
            )));
        }
        for (exported, recorded) in self.groups.iter().zip(receipt.groups.iter()) {
            if exported.group != recorded.group
                || exported.task_id != recorded.task_id
                || exported.family != recorded.family
                || exported.rewards != recorded.rewards
                || exported.advantages != recorded.advantages
                || exported.reward_spread != recorded.reward_spread
                || exported.updated != recorded.updated
            {
                return Err(TrainlabError::Export(format!(
                    "export group {} does not match its receipt record",
                    exported.group
                )));
            }
        }
        Ok(())
    }

    /// Write the export to `path` atomically, refusing to overwrite
    /// an existing file (the receipt's discipline: an export that can
    /// be replaced is not evidence, and silently retraining from a
    /// different export under the same path is exactly the failure
    /// this prevents).
    pub fn write_new(&self, path: &Path) -> Result<(), TrainlabError> {
        self.validate()?;
        if path.exists() {
            return Err(TrainlabError::Export(format!(
                "groups export already exists at {}; refusing to overwrite evidence",
                path.display()
            )));
        }
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let temp = parent.join(format!(
            ".phlow-trainlab-export-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        let text = serde_json::to_string_pretty(self)?;
        fs::write(&temp, &text)?;
        fs::rename(&temp, path)?;
        // Verify what landed: the file on disk must be a valid export
        // whose header hashes match the in-memory value.
        let landed = GroupsExport::load(path)?;
        if landed.config_sha256 != self.config_sha256 || landed.run_id != self.run_id {
            return Err(TrainlabError::Export(
                "written export failed verification (header mismatch)".to_string(),
            ));
        }
        Ok(())
    }

    /// Load and validate an export from `path`. Truncated files,
    /// mismatched per-group shapes, unknown schemas, and internally
    /// inconsistent records are typed errors.
    pub fn load(path: &Path) -> Result<GroupsExport, TrainlabError> {
        let text = fs::read_to_string(path)?;
        let export: GroupsExport = serde_json::from_str(&text)?;
        export.validate()?;
        Ok(export)
    }
}

impl GroupExportRecord {
    /// Validate one record against its expected 1-based position.
    fn validate(&self, expected_group: usize) -> Result<(), TrainlabError> {
        if self.group != expected_group {
            return Err(TrainlabError::Export(format!(
                "export group numbers must be sequential from 1: position {expected_group} \
                 carries group {}",
                self.group
            )));
        }
        if self.task_id.is_empty() || self.family.is_empty() || self.prompt.is_empty() {
            return Err(TrainlabError::Export(format!(
                "export group {} has an empty task_id, family, or prompt",
                self.group
            )));
        }
        if self.completions.len() < 2
            || self.completions.len() != self.rewards.len()
            || self.completions.len() != self.advantages.len()
        {
            return Err(TrainlabError::Export(format!(
                "export group {} shapes mismatch: {} completions, {} rewards, {} advantages \
                 (all must align and be at least 2)",
                self.group,
                self.completions.len(),
                self.rewards.len(),
                self.advantages.len()
            )));
        }
        if self
            .rewards
            .iter()
            .chain(self.advantages.iter())
            .any(|value| !value.is_finite())
            || !self.reward_spread.is_finite()
        {
            return Err(TrainlabError::Export(format!(
                "export group {} carries a non-finite reward, advantage, or spread",
                self.group
            )));
        }
        let spread = crate::group::reward_spread(&self.rewards);
        if (spread - self.reward_spread).abs() > SPREAD_TOLERANCE {
            return Err(TrainlabError::Export(format!(
                "export group {} reward_spread {} disagrees with its rewards (spread {spread})",
                self.group, self.reward_spread
            )));
        }
        if self.updated != (spread > 0.0) {
            return Err(TrainlabError::Export(format!(
                "export group {} updated flag {} disagrees with its reward spread {spread}",
                self.group, self.updated
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receipt::{RECEIPT_SCHEMA, config_sha256};
    use serde_json::json;

    fn record(group: usize) -> GroupExportRecord {
        GroupExportRecord {
            group,
            task_id: format!("dev-increment-{group:03}"),
            family: "increment".to_string(),
            prompt: "# Complete this Python function.\ndef f(x):".to_string(),
            completions: vec![
                "\n    return x + 1\n".to_string(),
                "\n    return x\n".to_string(),
            ],
            rewards: vec![1.0, 0.0],
            advantages: vec![0.5, -0.5],
            reward_spread: 1.0,
            updated: true,
        }
    }

    fn export() -> GroupsExport {
        GroupsExport {
            schema: GROUPS_SCHEMA.to_string(),
            run_id: "test-run".to_string(),
            config_sha256: config_sha256(&json!({"group_size": 2})).expect("hash"),
            split: "dev".to_string(),
            sampler_id: "stub".to_string(),
            groups: vec![record(1), record(2)],
        }
    }

    fn receipt_for(export: &GroupsExport) -> RunReceipt {
        RunReceipt {
            schema: RECEIPT_SCHEMA,
            run_id: export.run_id.clone(),
            algorithm: "RLOO",
            sampler_id: export.sampler_id.clone(),
            split: export.split.clone(),
            config: json!({"group_size": 2}),
            config_sha256: export.config_sha256.clone(),
            groups: export
                .groups
                .iter()
                .map(|group| crate::receipt::GroupRecord {
                    group: group.group,
                    task_id: group.task_id.clone(),
                    family: group.family.clone(),
                    rewards: group.rewards.clone(),
                    advantages: group.advantages.clone(),
                    reward_spread: group.reward_spread,
                    updated: group.updated,
                    exact_rollouts: 0,
                    seconds: 0.0,
                })
                .collect(),
            samples_total: 4,
            elapsed_seconds: 0.0,
        }
    }

    fn temp_path(tag: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "phlow-trainlab-export-test-{}-{tag}.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        path
    }

    #[test]
    fn export_writes_loads_and_matches_receipt() {
        let exported = export();
        let path = temp_path("roundtrip");
        exported.write_new(&path).expect("write");
        let loaded = GroupsExport::load(&path).expect("load");
        assert_eq!(loaded, exported);
        loaded
            .validate_against_receipt(&receipt_for(&exported))
            .expect("export must tie to its receipt");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn export_refuses_overwrite() {
        // Adversarial: an export under a path is evidence for one run;
        // a rerun must not silently replace the training data.
        let path = temp_path("twice");
        export().write_new(&path).expect("first write");
        assert!(matches!(
            export().write_new(&path),
            Err(TrainlabError::Export(_))
        ));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn load_rejects_unknown_schema() {
        // Adversarial: a future/foreign format must fail closed, not
        // be partially interpreted as v1 training data.
        let mut exported = export();
        exported.schema = "phlow.trainlab.groups/v99".to_string();
        let path = temp_path("schema");
        fs::write(
            &path,
            serde_json::to_string_pretty(&exported).expect("json"),
        )
        .expect("write raw");
        assert!(matches!(
            GroupsExport::load(&path),
            Err(TrainlabError::Export(_))
        ));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn load_rejects_truncated_shapes() {
        // Adversarial: dropping one completion (a truncated export)
        // misaligns rewards/advantages — the loader must reject the
        // whole file rather than hand a trainer a shifted group.
        let mut exported = export();
        exported.groups[0].completions.pop();
        let path = temp_path("truncated");
        fs::write(
            &path,
            serde_json::to_string_pretty(&exported).expect("json"),
        )
        .expect("write raw");
        assert!(matches!(
            GroupsExport::load(&path),
            Err(TrainlabError::Export(_))
        ));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn load_rejects_inconsistent_spread_and_updated() {
        // Adversarial: a hand-edited spread/updated pair that
        // disagrees with the rewards would make a trainer step on a
        // group the run recorded as signalless (or skip a live one).
        let mut exported = export();
        exported.groups[0].reward_spread = 0.0;
        exported.groups[0].updated = false;
        let path = temp_path("spread");
        fs::write(
            &path,
            serde_json::to_string_pretty(&exported).expect("json"),
        )
        .expect("write raw");
        assert!(matches!(
            GroupsExport::load(&path),
            Err(TrainlabError::Export(_))
        ));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn receipt_tie_rejects_tampered_reward() {
        // Adversarial: changing one exported reward after the run
        // must break the tie to the receipt — the export may no
        // longer claim to be that run's data.
        let mut exported = export();
        exported.groups[1].rewards[0] = 0.0;
        exported.groups[1].advantages = vec![0.0, 0.0];
        exported.groups[1].reward_spread = 0.0;
        exported.groups[1].updated = false;
        let receipt = receipt_for(&export());
        assert!(matches!(
            exported.validate_against_receipt(&receipt),
            Err(TrainlabError::Export(_))
        ));
    }

    #[test]
    fn receipt_tie_rejects_wrong_config_hash() {
        // Adversarial: an export from a different configuration must
        // not validate against this run's receipt.
        let mut exported = export();
        exported.config_sha256 = config_sha256(&json!({"group_size": 4})).expect("hash");
        let receipt = receipt_for(&export());
        assert!(matches!(
            exported.validate_against_receipt(&receipt),
            Err(TrainlabError::Export(_))
        ));
    }
}
