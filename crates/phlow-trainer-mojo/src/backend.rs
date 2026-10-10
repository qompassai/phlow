//! The production backend surface: dispatch + the contract run.
//!
//! Selection model (mirrors the other trainer tracks): phlow-trainlab
//! itself stays backend-agnostic — backends are separate crates the
//! operator invokes against the shared file contract
//! (`phlow.trainlab.groups/v1` export + its trainlab receipt in, a
//! receipt out). This module is the Mojo backend's surface:
//!
//! - [`availability`] reports whether the Mojo path can run here
//!   (GPU residency policy + kernel ABI), without allocating.
//! - [`score_blocks`], [`rloo_advantages_backend`], and [`sample`]
//!   dispatch one call each: Mojo when available, the pure-Rust
//!   reference when the backend is *unavailable* and the config
//!   allows fallback. Kernel errors and shape violations are **not**
//!   fallback triggers — they fail closed.
//! - [`run_scoring`] is the production entry point: verify a groups
//!   export against its trainlab receipt (run_id + config hash tie),
//!   score the producer logits of one export group through the
//!   dispatch, recompute that group's advantages, optionally draw
//!   one sample per completion row-set, and build the
//!   [`ScoringReceipt`] hash-chained to both input files.
//!
//! The logits producer stays the PyTorch sidecar (it owns the
//! forward pass and the trainer); this backend owns the reduction
//! and the draw. Default selection remains the sidecar path — this
//! surface runs only when an operator invokes it.

use std::path::Path;
use std::time::Instant;

use phlow_trainlab::export::GroupsExport;
use serde::Deserialize;

use crate::error::ScoringError;
use crate::ffi::{self, SampleBatch};
use crate::gpu_policy::{self, GpuSnapshot};
use crate::reference::{self, SampleParams};
use crate::scoring_receipt::{
    GroupScoreRecord, SCORING_RECEIPT_SCHEMA, SamplerRecord, ScoringReceipt, read_contract_file,
    sha256_hex,
};

/// Backend identifier recorded in receipts.
pub const BACKEND_ID: &str = "mojo";
/// Kernel-implementation label recorded in receipts.
pub const KERNEL_VERSION_LABEL: &str = "mojo-1.0/max-26.5 scoring ABI v2";
/// The scoring ABI version this backend requires.
pub const KERNEL_ABI_VERSION_REQUIRED: i32 = 2;

/// Operator configuration for the backend dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendConfig {
    /// Minimum free VRAM (MiB) the residency policy requires.
    pub gpu_free_mib_min: u64,
    /// Whether an unavailable Mojo backend falls back to the
    /// reference path (recorded in the receipt) instead of erroring.
    pub allow_reference_fallback: bool,
}

impl Default for BackendConfig {
    fn default() -> BackendConfig {
        BackendConfig {
            gpu_free_mib_min: gpu_policy::GPU_FREE_MIB_MIN_DEFAULT,
            allow_reference_fallback: true,
        }
    }
}

/// Which implementation produced a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoringPath {
    /// The Mojo GPU kernels.
    Mojo,
    /// The pure-Rust reference (fallback).
    Reference,
}

impl ScoringPath {
    /// The receipt spelling of the path.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            ScoringPath::Mojo => "mojo",
            ScoringPath::Reference => "reference",
        }
    }
}

/// A dispatch outcome: the value, the path that produced it, and the
/// residency evidence (GPU snapshot when the Mojo path ran; the
/// structured refusal reason when the fallback did).
#[derive(Debug)]
pub struct Dispatch<T> {
    /// The produced value.
    pub value: T,
    /// The path that produced it.
    pub path: ScoringPath,
    /// GPU snapshot observed by the residency check (Mojo path).
    pub gpu: Option<GpuSnapshot>,
    /// Why the fallback ran (reference path only).
    pub fallback_reason: Option<String>,
}

/// The backend's availability, probed without allocating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendAvailability {
    /// Whether the Mojo path can run under this config.
    pub available: bool,
    /// Kernel ABI version reported by the linked library.
    pub abi_version: i32,
    /// GPU snapshot when the residency check passed.
    pub gpu: Option<GpuSnapshot>,
    /// Why the backend is unavailable, when it is.
    pub reason: Option<String>,
}

/// How a run resolves its path, decided once per run so every number
/// in one receipt comes from the same implementation.
enum Resolved {
    Mojo(GpuSnapshot),
    Reference(String),
}

/// Resolve the path for one call or run under `config`.
///
/// The Mojo path requires the residency check to pass **and** the
/// linked library to report the required ABI version. Any
/// unavailability becomes the reference fallback when the config
/// allows it, and the typed error otherwise.
fn resolve(config: &BackendConfig) -> Result<Resolved, ScoringError> {
    let unavailable = |detail: String| -> Result<Resolved, ScoringError> {
        if config.allow_reference_fallback {
            Ok(Resolved::Reference(detail))
        } else {
            Err(ScoringError::Unavailable { detail })
        }
    };
    match gpu_policy::ensure_capacity(config.gpu_free_mib_min) {
        Ok(snapshot) => {
            let abi = ffi::scoring_version();
            if abi == KERNEL_ABI_VERSION_REQUIRED {
                Ok(Resolved::Mojo(snapshot))
            } else {
                unavailable(format!(
                    "kernel ABI version {abi}, backend requires {KERNEL_ABI_VERSION_REQUIRED}"
                ))
            }
        }
        Err(ScoringError::Unavailable { detail }) => unavailable(detail),
        Err(other) => Err(other),
    }
}

/// Probe availability under `config` (no device allocation).
#[must_use]
pub fn availability(config: &BackendConfig) -> BackendAvailability {
    let abi = ffi::scoring_version();
    match gpu_policy::ensure_capacity(config.gpu_free_mib_min) {
        Ok(snapshot) => {
            if abi == KERNEL_ABI_VERSION_REQUIRED {
                BackendAvailability {
                    available: true,
                    abi_version: abi,
                    gpu: Some(snapshot),
                    reason: None,
                }
            } else {
                BackendAvailability {
                    available: false,
                    abi_version: abi,
                    gpu: Some(snapshot),
                    reason: Some(format!(
                        "kernel ABI version {abi}, backend requires \
                         {KERNEL_ABI_VERSION_REQUIRED}"
                    )),
                }
            }
        }
        Err(error) => BackendAvailability {
            available: false,
            abi_version: abi,
            gpu: None,
            reason: Some(error.to_string()),
        },
    }
}

/// One block of producer logits to score (one completion's rows).
#[derive(Debug, Clone)]
pub struct LogitBlock {
    /// Row-major f32 logits, `rows * vocab` values.
    pub logits: Vec<f32>,
    /// Completion tokens scored (rows).
    pub rows: usize,
    /// Logit width (vocabulary).
    pub vocab: usize,
    /// Target token id per row.
    pub targets: Vec<i32>,
}

fn score_with(resolved: &Resolved, blocks: &[LogitBlock]) -> Result<Vec<Vec<f32>>, ScoringError> {
    let mut out = Vec::with_capacity(blocks.len());
    match resolved {
        Resolved::Mojo(_) => {
            for block in blocks {
                out.push(ffi::logprob_token_logps(
                    &block.logits,
                    block.rows,
                    block.vocab,
                    &block.targets,
                )?);
            }
        }
        Resolved::Reference(_) => {
            for block in blocks {
                out.push(reference::logprob_token_logps(
                    &block.logits,
                    block.rows,
                    block.vocab,
                    &block.targets,
                ));
            }
        }
    }
    Ok(out)
}

fn advantages_with(
    resolved: &Resolved,
    rewards: &[f32],
    offsets: &[i32],
) -> Result<Vec<f32>, ScoringError> {
    match resolved {
        Resolved::Mojo(_) => ffi::rloo_advantages(rewards, offsets),
        Resolved::Reference(_) => Ok(reference::rloo_advantages(rewards, offsets)),
    }
}

fn sample_with(
    resolved: &Resolved,
    logits: &[f32],
    rows: usize,
    vocab: usize,
    params: &SampleParams,
) -> Result<SampleBatch, ScoringError> {
    match resolved {
        Resolved::Mojo(_) => ffi::sample_tokens(logits, rows, vocab, params),
        Resolved::Reference(_) => {
            let outcomes = reference::sample_batch(logits, rows, vocab, params);
            let cand_len = rows * crate::SAMPLE_CANDIDATES_MAX as usize;
            let mut batch = SampleBatch {
                tokens: Vec::with_capacity(rows),
                candidate_counts: Vec::with_capacity(rows),
                cand_idx: vec![0_i32; cand_len],
                cand_val: vec![0.0_f32; cand_len],
            };
            for (row, outcome) in outcomes.iter().enumerate() {
                batch.tokens.push(outcome.token);
                batch.candidate_counts.push(outcome.candidate_count as i32);
                let base = row * crate::SAMPLE_CANDIDATES_MAX as usize;
                for (slot, candidate) in outcome.candidates.iter().enumerate() {
                    batch.cand_idx[base + slot] = *candidate;
                    batch.cand_val[base + slot] = logits[row * vocab + *candidate as usize];
                }
            }
            Ok(batch)
        }
    }
}

fn into_dispatch<T>(resolved: Resolved, value: T) -> Dispatch<T> {
    match resolved {
        Resolved::Mojo(snapshot) => Dispatch {
            value,
            path: ScoringPath::Mojo,
            gpu: Some(snapshot),
            fallback_reason: None,
        },
        Resolved::Reference(reason) => Dispatch {
            value,
            path: ScoringPath::Reference,
            gpu: None,
            fallback_reason: Some(reason),
        },
    }
}

/// Score every block's per-token logprobs through the dispatch.
pub fn score_blocks(
    blocks: &[LogitBlock],
    config: &BackendConfig,
) -> Result<Dispatch<Vec<Vec<f32>>>, ScoringError> {
    let resolved = resolve(config)?;
    let value = score_with(&resolved, blocks)?;
    Ok(into_dispatch(resolved, value))
}

/// RLOO advantages through the dispatch.
pub fn rloo_advantages_backend(
    rewards: &[f32],
    offsets: &[i32],
    config: &BackendConfig,
) -> Result<Dispatch<Vec<f32>>, ScoringError> {
    let resolved = resolve(config)?;
    let value = advantages_with(&resolved, rewards, offsets)?;
    Ok(into_dispatch(resolved, value))
}

/// Sample one token per row through the dispatch.
pub fn sample(
    logits: &[f32],
    rows: usize,
    vocab: usize,
    params: &SampleParams,
    config: &BackendConfig,
) -> Result<Dispatch<SampleBatch>, ScoringError> {
    let resolved = resolve(config)?;
    let value = sample_with(&resolved, logits, rows, vocab, params)?;
    Ok(into_dispatch(resolved, value))
}

/// One manifest entry: the producer-side (PyTorch control) values a
/// production run compares its scores against.
#[derive(Debug, Clone)]
pub struct ManifestEntry {
    /// Target token ids, aligned with the block's rows.
    pub targets: Vec<i32>,
    /// Producer per-token logprobs, aligned with `targets`.
    pub token_logps: Vec<f64>,
    /// Producer mean per-token logprob for the completion.
    pub mean_logprob: f64,
}

/// The minimal projection of a trainlab receipt the backend verifies
/// (the Burn track's pattern: the two binding fields, hashed whole).
#[derive(Debug, Deserialize)]
struct TrainlabReceiptRef {
    run_id: String,
    config_sha256: String,
}

/// A production run's outcome: the receipt plus the agreement
/// evidence the CLI reports (the receipt itself stores our numbers;
/// the diffs against the producer are run evidence).
#[derive(Debug)]
pub struct ProductionRun {
    /// The scoring receipt (not yet written; the caller writes it).
    pub receipt: ScoringReceipt,
    /// Worst per-token |Δ| vs the producer manifest.
    pub worst_token_diff: f64,
    /// Worst per-completion mean |Δ| vs the producer manifest.
    pub worst_mean_diff: f64,
    /// Worst |Δ| between our advantages and the export's recorded
    /// f64 advantages for the scored group.
    pub worst_advantage_diff: f64,
    /// Sampled tokens, when the run sampled (one per scored row of
    /// the first block).
    pub sampled_tokens: Option<Vec<i32>>,
}

/// Run the production scoring path over one export group.
///
/// Verifies, in order: the export parses and self-validates; the
/// trainlab receipt's `run_id`/`config_sha256` tie to the export;
/// the named group exists and its completion count matches the
/// manifest/blocks. Then scores through one resolved path and
/// builds the receipt. Any violation is a typed error and no
/// receipt is produced.
///
/// `sampler`, when present, additionally draws one sample per row of
/// the first block under those params — the draw that the receipt's
/// sampler record makes reproducible.
#[allow(clippy::too_many_arguments)]
pub fn run_scoring(
    export_path: &Path,
    trainlab_receipt_path: &Path,
    group_number: usize,
    entries: &[ManifestEntry],
    blocks: &[LogitBlock],
    sampler: Option<SampleParams>,
    config: &BackendConfig,
) -> Result<ProductionRun, ScoringError> {
    let started = Instant::now();
    let export_bytes = read_contract_file(export_path)?;
    let receipt_bytes = read_contract_file(trainlab_receipt_path)?;
    let export: GroupsExport =
        serde_json::from_slice(&export_bytes).map_err(|error| ScoringError::Contract {
            detail: format!("groups export does not parse: {error}"),
        })?;
    export.validate().map_err(|error| ScoringError::Contract {
        detail: format!("groups export invalid: {error}"),
    })?;
    let receipt_ref: TrainlabReceiptRef =
        serde_json::from_slice(&receipt_bytes).map_err(|error| ScoringError::Contract {
            detail: format!("trainlab receipt does not parse: {error}"),
        })?;
    if receipt_ref.run_id != export.run_id || receipt_ref.config_sha256 != export.config_sha256 {
        return Err(ScoringError::Contract {
            detail: format!(
                "export/receipt tie broken: export run {:?} config {} vs receipt run {:?} \
                 config {}",
                export.run_id, export.config_sha256, receipt_ref.run_id, receipt_ref.config_sha256
            ),
        });
    }
    let group = export
        .groups
        .iter()
        .find(|record| record.group == group_number)
        .ok_or_else(|| ScoringError::Contract {
            detail: format!("export has no group {group_number}"),
        })?;
    if entries.len() != group.completions.len() || blocks.len() != entries.len() {
        return Err(ScoringError::Contract {
            detail: format!(
                "group {group_number} has {} completions, manifest has {} entries, \
                 {} blocks given",
                group.completions.len(),
                entries.len(),
                blocks.len()
            ),
        });
    }
    for (index, (entry, block)) in entries.iter().zip(blocks.iter()).enumerate() {
        if entry.targets != block.targets || entry.token_logps.len() != block.rows {
            return Err(ScoringError::Contract {
                detail: format!(
                    "manifest entry {index} targets/token_logps do not align with its block"
                ),
            });
        }
    }

    let resolved = resolve(config)?;
    let token_logps = score_with(&resolved, blocks)?;
    let mut mean_logprobs: Vec<f64> = Vec::with_capacity(blocks.len());
    let mut worst_token_diff = 0.0_f64;
    let mut worst_mean_diff = 0.0_f64;
    for (index, (logps, entry)) in token_logps.iter().zip(entries.iter()).enumerate() {
        for (got, want) in logps.iter().zip(entry.token_logps.iter()) {
            worst_token_diff = worst_token_diff.max((f64::from(*got) - want).abs());
        }
        let mean = if logps.is_empty() {
            0.0
        } else {
            logps.iter().map(|v| f64::from(*v)).sum::<f64>() / logps.len() as f64
        };
        worst_mean_diff = worst_mean_diff.max((mean - entry.mean_logprob).abs());
        let _ = index;
        mean_logprobs.push(mean);
    }

    let rewards: Vec<f32> = group.rewards.iter().map(|r| *r as f32).collect();
    let offsets = vec![0_i32, rewards.len() as i32];
    let advantages = advantages_with(&resolved, &rewards, &offsets)?;
    let mut worst_advantage_diff = 0.0_f64;
    for (got, want) in advantages.iter().zip(group.advantages.iter()) {
        worst_advantage_diff = worst_advantage_diff.max((f64::from(*got) - want).abs());
    }

    let mut sampled_tokens = None;
    if let Some(params) = sampler {
        let first = &blocks[0];
        let batch = sample_with(&resolved, &first.logits, first.rows, first.vocab, &params)?;
        sampled_tokens = Some(batch.tokens);
    }

    let (path, gpu, fallback_reason) = match &resolved {
        Resolved::Mojo(snapshot) => (ScoringPath::Mojo, Some(*snapshot), None),
        Resolved::Reference(reason) => (ScoringPath::Reference, None, Some(reason.clone())),
    };
    let receipt = ScoringReceipt {
        schema: SCORING_RECEIPT_SCHEMA,
        backend: BACKEND_ID.to_string(),
        kernel_abi_version: ffi::scoring_version(),
        kernel_version: KERNEL_VERSION_LABEL.to_string(),
        run_id: export.run_id.clone(),
        config_sha256: export.config_sha256.clone(),
        groups_export_sha256: sha256_hex(&export_bytes),
        trainlab_receipt_sha256: sha256_hex(&receipt_bytes),
        scoring_path: path.as_str().to_string(),
        fallback_reason,
        gpu_free_mib_observed: gpu.map(|snapshot| snapshot.free_mib),
        gpu_free_mib_min: config.gpu_free_mib_min,
        sampler: sampler.map(SamplerRecord::from),
        groups: vec![GroupScoreRecord {
            group: group.group,
            task_id: group.task_id.clone(),
            mean_logprobs,
            advantages: advantages.iter().map(|v| f64::from(*v)).collect(),
        }],
        completions_total: blocks.len(),
        wall_clock_seconds: started.elapsed().as_secs_f64(),
    };
    Ok(ProductionRun {
        receipt,
        worst_token_diff,
        worst_mean_diff,
        worst_advantage_diff,
        sampled_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_policy_default() {
        let config = BackendConfig::default();
        assert_eq!(
            config.gpu_free_mib_min,
            gpu_policy::GPU_FREE_MIB_MIN_DEFAULT
        );
        assert!(config.allow_reference_fallback);
    }

    #[test]
    fn impossible_threshold_without_fallback_is_unavailable() {
        // Adversarial: an unsatisfiable residency demand with the
        // fallback disabled must surface the typed error — a run
        // that proceeds anyway would be exactly the silent-corruption
        // scenario the policy exists to prevent.
        let config = BackendConfig {
            gpu_free_mib_min: u64::MAX,
            allow_reference_fallback: false,
        };
        let blocks = vec![LogitBlock {
            logits: vec![0.0_f32; 8],
            rows: 1,
            vocab: 8,
            targets: vec![0],
        }];
        assert!(matches!(
            score_blocks(&blocks, &config),
            Err(ScoringError::Unavailable { .. })
        ));
    }

    #[test]
    fn impossible_threshold_with_fallback_uses_reference() {
        let config = BackendConfig {
            gpu_free_mib_min: u64::MAX,
            allow_reference_fallback: true,
        };
        let blocks = vec![LogitBlock {
            logits: vec![0.0_f32; 8],
            rows: 1,
            vocab: 8,
            targets: vec![3],
        }];
        let dispatch = score_blocks(&blocks, &config).expect("fallback scores");
        assert_eq!(dispatch.path, ScoringPath::Reference);
        assert!(dispatch.fallback_reason.is_some());
        let want = -f32::ln(8.0);
        assert!((dispatch.value[0][0] - want).abs() < 1e-6);
    }
}
