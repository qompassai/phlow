//! The scoring receipt: the auditable record of one Mojo-backend
//! scoring run, hash-chained to its inputs.
//!
//! Shape mirrors the trainer tracks' receipts
//! (`phlow.trainer.receipt/v1`): schema + backend + the run's
//! identifiers and SHA-256 ties to the exact groups export and
//! trainlab receipt that were verified before scoring, the
//! hyperparameters that matter (here: the sampler spec, when the run
//! sampled), per-group results, and wall-clock time. Receipts are
//! written atomically and **never overwritten** — a receipt that can
//! be replaced is not evidence (the trainlab rule, kept here).

use std::fs;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::ScoringError;
use crate::reference::SampleParams;

/// Scoring receipt schema identifier.
pub const SCORING_RECEIPT_SCHEMA: &str = "phlow.trainer-mojo.scoring-receipt/v1";

/// Maximum bytes for a contract JSON file (exports/receipts).
pub const CONTRACT_BYTES_MAX: u64 = 64 * 1024 * 1024;

/// The sampler parameters a run used, recorded so the draw is
/// reproducible from the receipt alone.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SamplerRecord {
    /// Temperature (`0.0` = greedy).
    pub temperature: f64,
    /// Top-k limit (0 = disabled).
    pub top_k: u32,
    /// Nucleus mass (1.0 = disabled).
    pub top_p: f64,
    /// Base seed.
    pub seed: u64,
    /// Draw index.
    pub draw_index: u32,
}

impl From<SampleParams> for SamplerRecord {
    fn from(params: SampleParams) -> SamplerRecord {
        SamplerRecord {
            temperature: f64::from(params.temperature),
            top_k: params.top_k,
            top_p: f64::from(params.top_p),
            seed: params.seed,
            draw_index: params.draw_index,
        }
    }
}

/// One group's scoring results within a run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroupScoreRecord {
    /// 1-based group index within the export (matches the receipt).
    pub group: usize,
    /// Task the group was sampled for.
    pub task_id: String,
    /// Mean per-token logprob per completion, aligned with the
    /// export's `completions`.
    pub mean_logprobs: Vec<f64>,
    /// RLOO advantages recomputed from the export's rewards by this
    /// backend, aligned with `completions`.
    pub advantages: Vec<f64>,
}

/// A complete scoring receipt for one production run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScoringReceipt {
    /// Schema identifier ([`SCORING_RECEIPT_SCHEMA`]).
    pub schema: &'static str,
    /// Backend identifier: `mojo` when the Mojo kernels produced the
    /// scores (see `scoring_path` for the fallback case).
    pub backend: String,
    /// Mojo scoring ABI version reported by the kernel library.
    pub kernel_abi_version: i32,
    /// Kernel-implementation label for human readers.
    pub kernel_version: String,
    /// Run identifier, matching the export and trainlab receipt.
    pub run_id: String,
    /// Config hash shared by export and trainlab receipt.
    pub config_sha256: String,
    /// SHA-256 of the groups-export file bytes consumed.
    pub groups_export_sha256: String,
    /// SHA-256 of the trainlab-receipt file bytes verified against.
    pub trainlab_receipt_sha256: String,
    /// Which path produced the numbers: `mojo` or `reference`.
    pub scoring_path: String,
    /// Why the reference fallback ran, when it did.
    pub fallback_reason: Option<String>,
    /// Free VRAM (MiB) observed by the residency check, when the
    /// Mojo path was attempted.
    pub gpu_free_mib_observed: Option<u64>,
    /// The residency minimum (MiB) the run enforced.
    pub gpu_free_mib_min: u64,
    /// Sampler parameters, when the run sampled.
    pub sampler: Option<SamplerRecord>,
    /// Per-group results, in export order (the scored group only:
    /// producer logits exist per group, not per run).
    pub groups: Vec<GroupScoreRecord>,
    /// Total completions scored across `groups`.
    pub completions_total: usize,
    /// Wall-clock seconds for the run.
    pub wall_clock_seconds: f64,
}

impl ScoringReceipt {
    /// Write the receipt to `path` atomically, refusing to overwrite
    /// an existing file.
    pub fn write_new(&self, path: &Path) -> Result<(), ScoringError> {
        if path.exists() {
            return Err(ScoringError::Contract {
                detail: format!(
                    "scoring receipt already exists at {}; refusing to overwrite evidence",
                    path.display()
                ),
            });
        }
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|error| ScoringError::InputFile {
            source: path.display().to_string(),
            detail: error.to_string(),
        })?;
        let temp = parent.join(format!(
            ".phlow-trainer-mojo-receipt-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        let text = serde_json::to_string_pretty(self).map_err(|error| ScoringError::Contract {
            detail: format!("receipt serialization failed: {error}"),
        })?;
        fs::write(&temp, &text).map_err(|error| ScoringError::InputFile {
            source: temp.display().to_string(),
            detail: error.to_string(),
        })?;
        fs::rename(&temp, path).map_err(|error| ScoringError::InputFile {
            source: path.display().to_string(),
            detail: error.to_string(),
        })?;
        // Verify what landed: the file on disk must carry the same
        // run identity and input hashes as the in-memory receipt.
        let landed = fs::read_to_string(path).map_err(|error| ScoringError::InputFile {
            source: path.display().to_string(),
            detail: error.to_string(),
        })?;
        let value: serde_json::Value =
            serde_json::from_str(&landed).map_err(|error| ScoringError::Contract {
                detail: format!("written receipt does not parse: {error}"),
            })?;
        let matches = value.get("run_id").and_then(|v| v.as_str()) == Some(self.run_id.as_str())
            && value.get("config_sha256").and_then(|v| v.as_str())
                == Some(self.config_sha256.as_str())
            && value.get("groups_export_sha256").and_then(|v| v.as_str())
                == Some(self.groups_export_sha256.as_str());
        if !matches {
            return Err(ScoringError::Contract {
                detail: "written receipt failed verification (header mismatch)".to_string(),
            });
        }
        Ok(())
    }
}

/// SHA-256 (hex) of raw bytes.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Read a contract file under the byte bound, returning its bytes.
pub fn read_contract_file(path: &Path) -> Result<Vec<u8>, ScoringError> {
    let metadata = fs::metadata(path).map_err(|error| ScoringError::InputFile {
        source: path.display().to_string(),
        detail: error.to_string(),
    })?;
    if metadata.len() > CONTRACT_BYTES_MAX {
        return Err(ScoringError::Contract {
            detail: format!(
                "{} is {} bytes, over the {CONTRACT_BYTES_MAX}-byte bound",
                path.display(),
                metadata.len()
            ),
        });
    }
    fs::read(path).map_err(|error| ScoringError::InputFile {
        source: path.display().to_string(),
        detail: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt() -> ScoringReceipt {
        ScoringReceipt {
            schema: SCORING_RECEIPT_SCHEMA,
            backend: "mojo".to_string(),
            kernel_abi_version: 2,
            kernel_version: "mojo-1.0/max-26.5 scoring ABI v2".to_string(),
            run_id: "test-run".to_string(),
            config_sha256: "a".repeat(64),
            groups_export_sha256: "b".repeat(64),
            trainlab_receipt_sha256: "c".repeat(64),
            scoring_path: "mojo".to_string(),
            fallback_reason: None,
            gpu_free_mib_observed: Some(4096),
            gpu_free_mib_min: 1024,
            sampler: None,
            groups: vec![GroupScoreRecord {
                group: 1,
                task_id: "dev-increment-000".to_string(),
                mean_logprobs: vec![-0.5, -1.5],
                advantages: vec![0.5, -0.5],
            }],
            completions_total: 2,
            wall_clock_seconds: 0.25,
        }
    }

    #[test]
    fn receipt_writes_and_verifies() {
        let path = std::env::temp_dir().join(format!(
            "phlow-trainer-mojo-receipt-test-{}.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        receipt().write_new(&path).expect("write");
        assert!(path.exists());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn receipt_refuses_overwrite() {
        let path = std::env::temp_dir().join(format!(
            "phlow-trainer-mojo-receipt-test-{}-twice.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        receipt().write_new(&path).expect("first write");
        assert!(matches!(
            receipt().write_new(&path),
            Err(ScoringError::Contract { .. })
        ));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        // SHA-256 of the empty string (standard test vector).
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
