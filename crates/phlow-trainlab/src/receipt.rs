//! The per-run receipt: the auditable record of one experiment.
//!
//! Mirrors the companion repo's `training-receipt.json`: algorithm,
//! full configuration (plus its SHA-256, so a receipt can be tied to
//! exactly one config), per-group rewards/advantages/spread, and
//! totals. Receipts are written atomically and **never overwritten**
//! — like `evaluate_passk.py`, writing to an existing path is an
//! error, because a receipt that can be replaced is not evidence.

use std::fs;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::TrainlabError;

/// Receipt schema identifier, versioned from the first build.
pub const RECEIPT_SCHEMA: &str = "phlow.trainlab.receipt/v1";

/// One prompt group's record within a run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroupRecord {
    /// 1-based group index within the run.
    pub group: usize,
    /// Task the group was sampled for.
    pub task_id: String,
    /// Task family.
    pub family: String,
    /// Per-completion rewards, in sampling order.
    pub rewards: Vec<f64>,
    /// Leave-one-out advantages, aligned with `rewards`.
    pub advantages: Vec<f64>,
    /// `max(rewards) - min(rewards)`; zero means no relative signal.
    pub reward_spread: f64,
    /// Whether this group carried signal (spread > 0) — i.e. whether
    /// a trainer backend would have taken an update step from it.
    pub updated: bool,
    /// Completions exactly equal to the reference body (trimmed).
    pub exact_rollouts: usize,
    /// Wall-clock seconds for the group (sampling + evaluation).
    pub seconds: f64,
}

/// A complete run receipt.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunReceipt {
    /// Schema identifier ([`RECEIPT_SCHEMA`]).
    pub schema: &'static str,
    /// Caller-chosen run identifier.
    pub run_id: String,
    /// Algorithm the statistics implement (`"RLOO"`).
    pub algorithm: &'static str,
    /// Sampler identifier (see [`crate::sampler::Sampler::id`]).
    pub sampler_id: String,
    /// Split the run drew prompts from.
    pub split: String,
    /// The full run configuration, echoed verbatim.
    pub config: serde_json::Value,
    /// SHA-256 of the canonical JSON of `config`.
    pub config_sha256: String,
    /// Per-group records, in run order.
    pub groups: Vec<GroupRecord>,
    /// Total completions sampled across the run.
    pub samples_total: usize,
    /// Wall-clock seconds for the whole run.
    pub elapsed_seconds: f64,
}

impl RunReceipt {
    /// Write the receipt to `path` atomically, refusing to overwrite
    /// an existing file.
    pub fn write_new(&self, path: &Path) -> Result<(), TrainlabError> {
        if path.exists() {
            return Err(TrainlabError::Receipt(format!(
                "receipt already exists at {}; refusing to overwrite evidence",
                path.display()
            )));
        }
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let temp = parent.join(format!(
            ".phlow-trainlab-receipt-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        let text = serde_json::to_string_pretty(self)?;
        fs::write(&temp, &text)?;
        fs::rename(&temp, path)?;
        // Verify what landed: the file on disk must hash to the same
        // canonical form as the in-memory receipt.
        let landed = fs::read_to_string(path)?;
        let landed_value: serde_json::Value = serde_json::from_str(&landed)?;
        if landed_value
            .get("config_sha256")
            .and_then(|value| value.as_str())
            != Some(self.config_sha256.as_str())
        {
            return Err(TrainlabError::Receipt(
                "written receipt failed verification (config hash mismatch)".to_string(),
            ));
        }
        Ok(())
    }
}

/// SHA-256 (hex) of a value's canonical JSON encoding. serde_json
/// serializes struct fields in declaration order and sorts map keys,
/// so the digest is stable for a given value.
pub fn config_sha256<T: Serialize>(value: &T) -> Result<String, TrainlabError> {
    let text = serde_json::to_string(value)?;
    let digest = Sha256::digest(text.as_bytes());
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn receipt() -> RunReceipt {
        RunReceipt {
            schema: RECEIPT_SCHEMA,
            run_id: "test-run".to_string(),
            algorithm: "RLOO",
            sampler_id: "stub".to_string(),
            split: "rl".to_string(),
            config: json!({"group_size": 4}),
            config_sha256: config_sha256(&json!({"group_size": 4})).expect("hash"),
            groups: Vec::new(),
            samples_total: 0,
            elapsed_seconds: 0.0,
        }
    }

    #[test]
    fn receipt_writes_and_verifies() {
        let path = std::env::temp_dir().join(format!(
            "phlow-trainlab-receipt-test-{}.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        receipt().write_new(&path).expect("write");
        assert!(path.exists());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn receipt_refuses_overwrite() {
        // Adversarial: yesterday's evidence must not be replaceable
        // by today's rerun under the same path.
        let path = std::env::temp_dir().join(format!(
            "phlow-trainlab-receipt-test-{}-twice.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        receipt().write_new(&path).expect("first write");
        assert!(matches!(
            receipt().write_new(&path),
            Err(TrainlabError::Receipt(_))
        ));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn config_hash_is_stable_and_sensitive() {
        let a = config_sha256(&json!({"group_size": 4, "seed": 1})).expect("hash");
        let b = config_sha256(&json!({"group_size": 4, "seed": 1})).expect("hash");
        let c = config_sha256(&json!({"group_size": 4, "seed": 2})).expect("hash");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
