//! The live evaluator adapter: file-evidence in, [`Evaluation`] out.
//!
//! Where [`crate::evaluator::ScriptedEvaluator`] replays outcomes, this
//! adapter measures nothing itself — it *verifies* the evidence the
//! training pipeline produced and derives the loop's metric from it:
//!
//! - the **metric** is the dev-split mean pass@1 of a candidate trainlab
//!   run receipt (`phlow.trainlab.receipt/v1`, `split == "dev"`, binary
//!   reward mode), computed exactly as trainlab's `PasskReport` defines
//!   it: per task, the fraction of samples whose reward is a pass,
//!   averaged over tasks;
//! - when the change-set called for a training step, a **trainer
//!   receipt** (`phlow.trainer.receipt/v1`) must chain-hash to the
//!   trainlab receipt and groups export it consumed, the export must
//!   agree with that receipt group-for-group, the adapter must provably
//!   have moved, and the receipt's parity batch must survive the
//!   **parity guard** against a reference receipt: a divergence beyond
//!   [`PARITY_TOLERANCE_NATS_PER_TOKEN`] discards the iteration unless
//!   the receipt itself explains it (the trainer contract: a quantized
//!   configuration is measured and explained, never silently failed —
//!   and never silently passed either).
//!
//! Every check is hand-validated after parsing; derive is never trusted
//! at this boundary because the three real backends already disagree on
//! field spellings (`framework` vs `framework_versions`, `variant` vs
//! `completion_kind`, parity as a list vs an object with `pairs`).
//! All failures are [`EvalError`]s — evidence problems are evaluation
//! failures, never low scores.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::changeset::ChangeSet;
use crate::clock::Clock;
use crate::evaluator::{EvalError, Evaluation, Evaluator};
use crate::ledger::is_sha256_hex;

/// Maximum bytes in any single evidence file the adapter will read.
/// Real receipts and exports are tens of KiB; the bound exists so a
/// hostile or stray file cannot make the adapter allocate without
/// limit.
pub const EVIDENCE_FILE_BYTES_MAX: u64 = 8 * 1024 * 1024;
/// Maximum groups in one trainlab receipt or groups export.
pub const EVIDENCE_GROUPS_MAX: usize = 4096;
/// Maximum samples (rewards) in one group.
pub const EVIDENCE_SAMPLES_PER_GROUP_MAX: usize = 1024;
/// Maximum parity pairs in one trainer receipt (the contract batch
/// has four).
pub const PARITY_PAIRS_MAX: usize = 64;
/// Parity guard tolerance, from the trainer contract: same-precision
/// configurations must land within this per-pair divergence against
/// the reference, in nats per token.
pub const PARITY_TOLERANCE_NATS_PER_TOKEN: f64 = 0.05;

/// Schema string of a trainlab run receipt.
const TRAINLAB_RECEIPT_SCHEMA: &str = "phlow.trainlab.receipt/v1";
/// Schema string of a trainlab groups export.
const TRAINLAB_GROUPS_SCHEMA: &str = "phlow.trainlab.groups/v1";
/// Schema string of a trainer receipt.
const TRAINER_RECEIPT_SCHEMA: &str = "phlow.trainer.receipt/v1";
/// Backends the trainer contract defines.
const TRAINER_BACKENDS: &[&str] = &["pytorch", "candle", "burn"];

/// The trainer-side evidence for one change-set: the trainer receipt
/// plus the exact trainlab artifacts it consumed, and optionally the
/// reference trainer receipt the parity guard compares against (the
/// PyTorch track is the contract's reference).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainerEvidence {
    /// The trainer receipt (`phlow.trainer.receipt/v1`).
    pub trainer_receipt_path: PathBuf,
    /// The trainlab run receipt the trainer consumed.
    pub trainlab_receipt_path: PathBuf,
    /// The groups export the trainer consumed.
    pub groups_export_path: PathBuf,
    /// Reference trainer receipt for the parity guard, when available.
    pub parity_reference_path: Option<PathBuf>,
}

/// All evidence for one change-set, keyed by change-set id in
/// [`ReceiptEvaluator`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationBundle {
    /// Candidate trainlab run receipt on the dev split; the metric's
    /// source.
    pub dev_receipt_path: PathBuf,
    /// Trainer evidence, when the change-set called for a training
    /// step.
    pub trainer: Option<TrainerEvidence>,
}

/// An [`Evaluator`] that derives metrics from real pipeline evidence
/// files instead of running anything. The operator (or the serving
/// pipeline) assembles one [`EvaluationBundle`] per change-set id;
/// an id with no bundle is an evaluation failure, never a guess.
#[derive(Debug, Default)]
pub struct ReceiptEvaluator {
    bundles: BTreeMap<String, EvaluationBundle>,
}

impl ReceiptEvaluator {
    /// An evaluator with no bundles.
    #[must_use]
    pub fn new() -> Self {
        ReceiptEvaluator {
            bundles: BTreeMap::new(),
        }
    }

    /// Register the evidence bundle for one change-set id.
    #[must_use]
    pub fn with_bundle(
        mut self,
        change_set_id: impl Into<String>,
        bundle: EvaluationBundle,
    ) -> Self {
        self.bundles.insert(change_set_id.into(), bundle);
        self
    }

    /// Bundles registered.
    #[must_use]
    pub fn bundle_count(&self) -> usize {
        self.bundles.len()
    }
}

impl Evaluator for ReceiptEvaluator {
    fn evaluate(
        &mut self,
        change_set: &ChangeSet,
        _clock: &dyn Clock,
    ) -> Result<Evaluation, EvalError> {
        let bundle = self.bundles.get(&change_set.id).ok_or_else(|| {
            EvalError::failed(format!(
                "no evidence bundle registered for change-set {}",
                change_set.id
            ))
        })?;
        let dev = read_evidence(&bundle.dev_receipt_path, "trainlab dev receipt")?;
        let metric = dev_mean_pass_at_1(&dev.json)?;
        let mut description = format!(
            "dev mean pass@1 {metric:.4} from trainlab receipt {}",
            short_hash(&dev.sha256)
        );
        let mut trainer_sha256 = None;
        if let Some(trainer) = &bundle.trainer {
            let verified = verify_trainer_evidence(trainer)?;
            description.push_str(&format!(
                "; trainer {} receipt {}",
                verified.backend,
                short_hash(&verified.sha256)
            ));
            if let Some(divergence) = verified.max_parity_divergence {
                description.push_str(&format!("; parity max |d| {divergence:.4} nats/token"));
            }
            trainer_sha256 = Some(verified.sha256);
        }
        Ok(Evaluation {
            metric,
            trainlab_receipt_sha256: Some(dev.sha256),
            trainer_receipt_sha256: trainer_sha256,
            description,
        })
    }
}

/// One evidence file, read under the byte bound, with its SHA-256.
struct Evidence {
    json: Value,
    sha256: String,
}

/// Read and parse one evidence file. The size bound is enforced while
/// reading (never after an unbounded allocation), and the digest is
/// taken over the exact bytes on disk — the hash-chain currency.
fn read_evidence(path: &Path, what: &str) -> Result<Evidence, EvalError> {
    let file = std::fs::File::open(path).map_err(|err| {
        EvalError::failed(format!("{what}: cannot open {}: {err}", path.display()))
    })?;
    let mut bytes = Vec::new();
    file.take(EVIDENCE_FILE_BYTES_MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| {
            EvalError::failed(format!("{what}: cannot read {}: {err}", path.display()))
        })?;
    if bytes.len() as u64 > EVIDENCE_FILE_BYTES_MAX {
        return Err(EvalError::failed(format!(
            "{what}: {} exceeds the {EVIDENCE_FILE_BYTES_MAX}-byte evidence bound",
            path.display()
        )));
    }
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let json: Value = serde_json::from_slice(&bytes)
        .map_err(|err| EvalError::failed(format!("{what}: malformed JSON: {err}")))?;
    if !json.is_object() {
        return Err(EvalError::failed(format!(
            "{what}: top level is not a JSON object"
        )));
    }
    Ok(Evidence { json, sha256 })
}

/// First 12 hex chars of a digest, for one-line descriptions.
fn short_hash(sha256: &str) -> &str {
    &sha256[..sha256.len().min(12)]
}

/// A required string field, non-empty.
fn req_str<'a>(json: &'a Value, field: &str, what: &str) -> Result<&'a str, EvalError> {
    json.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| EvalError::failed(format!("{what}: field {field} missing or empty")))
}

/// A required finite number field.
fn req_f64(json: &Value, field: &str, what: &str) -> Result<f64, EvalError> {
    json.get(field)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| EvalError::failed(format!("{what}: field {field} missing or non-finite")))
}

/// Assert a document's `schema` field equals `expected`.
fn check_schema(json: &Value, expected: &str, what: &str) -> Result<(), EvalError> {
    let actual = req_str(json, "schema", what)?;
    if actual != expected {
        return Err(EvalError::failed(format!(
            "{what}: schema mismatch: expected {expected}, found {actual}"
        )));
    }
    Ok(())
}

/// Dev-split mean pass@1 from a trainlab run receipt: per group, the
/// fraction of rewards equal to a pass (binary reward mode only — the
/// derivation is meaningless under any other reward shape, so other
/// modes are rejected rather than approximated).
fn dev_mean_pass_at_1(receipt: &Value) -> Result<f64, EvalError> {
    let what = "trainlab dev receipt";
    check_schema(receipt, TRAINLAB_RECEIPT_SCHEMA, what)?;
    if req_str(receipt, "split", what)? != "dev" {
        return Err(EvalError::failed(format!(
            "{what}: split is not dev; the loop's metric is defined on the dev split only"
        )));
    }
    let mode = receipt
        .pointer("/config/reward/mode")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if mode != "binary" {
        return Err(EvalError::failed(format!(
            "{what}: reward mode {mode:?} is not binary; pass@1 cannot be derived"
        )));
    }
    let groups = receipt
        .get("groups")
        .and_then(Value::as_array)
        .filter(|groups| !groups.is_empty() && groups.len() <= EVIDENCE_GROUPS_MAX)
        .ok_or_else(|| EvalError::failed(format!("{what}: groups missing, empty, or oversized")))?;
    let mut total = 0.0_f64;
    for group in groups {
        total += group_pass_fraction(group, what)?;
    }
    Ok(total / groups.len() as f64)
}

/// One group's pass fraction: rewards equal to 1.0 over sample count.
fn group_pass_fraction(group: &Value, what: &str) -> Result<f64, EvalError> {
    let rewards = group
        .get("rewards")
        .and_then(Value::as_array)
        .filter(|rewards| !rewards.is_empty() && rewards.len() <= EVIDENCE_SAMPLES_PER_GROUP_MAX)
        .ok_or_else(|| EvalError::failed(format!("{what}: a group has no usable rewards")))?;
    let mut passing = 0_usize;
    for reward in rewards {
        let value = reward
            .as_f64()
            .filter(|value| value.is_finite() && *value >= -1.0 && *value <= 1.0)
            .ok_or_else(|| EvalError::failed(format!("{what}: reward out of range")))?;
        if (value - 1.0).abs() < 1e-9 {
            passing += 1;
        }
    }
    Ok(passing as f64 / rewards.len() as f64)
}

/// Parse a groups export and cross-check it against the trainlab
/// receipt it belongs to: config digest and run id must agree, and
/// every group's rewards/advantages must match the receipt exactly
/// (the trainer contract's attribution rule).
fn check_groups_export(export: &Value, receipt: &Value) -> Result<(), EvalError> {
    let what = "groups export";
    check_schema(export, TRAINLAB_GROUPS_SCHEMA, what)?;
    if req_str(export, "config_sha256", what)? != req_str(receipt, "config_sha256", "receipt")? {
        return Err(EvalError::failed(
            "groups export: config_sha256 does not match the trainlab receipt",
        ));
    }
    if req_str(export, "run_id", what)? != req_str(receipt, "run_id", "receipt")? {
        return Err(EvalError::failed(
            "groups export: run_id does not match the trainlab receipt",
        ));
    }
    let export_groups = export
        .get("groups")
        .and_then(Value::as_array)
        .filter(|groups| groups.len() <= EVIDENCE_GROUPS_MAX)
        .ok_or_else(|| EvalError::failed("groups export: groups missing or oversized"))?;
    let receipt_groups = receipt
        .get("groups")
        .and_then(Value::as_array)
        .ok_or_else(|| EvalError::failed("trainlab receipt: groups missing"))?;
    if export_groups.len() != receipt_groups.len() {
        return Err(EvalError::failed(
            "groups export: group count does not match the trainlab receipt",
        ));
    }
    for export_group in export_groups {
        let number = export_group.get("group").and_then(Value::as_u64);
        let receipt_group = receipt_groups
            .iter()
            .find(|candidate| candidate.get("group").and_then(Value::as_u64) == number);
        let same = receipt_group.is_some_and(|receipt_group| {
            export_group.get("task_id") == receipt_group.get("task_id")
                && export_group.get("rewards") == receipt_group.get("rewards")
                && export_group.get("advantages") == receipt_group.get("advantages")
        });
        if !same {
            return Err(EvalError::failed(
                "groups export: a group's task/rewards/advantages disagree with the receipt",
            ));
        }
    }
    Ok(())
}

/// Which half of the parity batch a pair belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ParityKind {
    /// The task's reference completion, whole.
    Full,
    /// The reference completion truncated to its first half.
    Half,
}

/// One normalized parity measurement.
#[derive(Debug, Clone, PartialEq)]
struct ParityPair {
    task_id: String,
    kind: ParityKind,
    tokens: u64,
    mean_logprob: f64,
}

/// Normalize the parity batch across the three real backend spellings:
/// a bare array (PyTorch, Burn) or an object carrying `pairs`
/// (Candle); kind under `variant` or `completion_kind`; token count
/// under `token_count` or `completion_tokens`.
fn parity_pairs(receipt: &Value, what: &str) -> Result<Vec<ParityPair>, EvalError> {
    let parity = receipt
        .get("parity")
        .ok_or_else(|| EvalError::failed(format!("{what}: parity batch missing")))?;
    let entries = match parity {
        Value::Array(entries) => entries.clone(),
        Value::Object(_) => parity
            .get("pairs")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| EvalError::failed(format!("{what}: parity object has no pairs")))?,
        _ => {
            return Err(EvalError::failed(format!(
                "{what}: parity has an unknown shape"
            )));
        }
    };
    if entries.is_empty() || entries.len() > PARITY_PAIRS_MAX {
        return Err(EvalError::failed(format!(
            "{what}: parity pair count out of bounds"
        )));
    }
    let mut pairs = Vec::with_capacity(entries.len());
    for entry in &entries {
        pairs.push(parity_pair(entry, what)?);
    }
    pairs.sort_by(|a, b| (&a.task_id, a.kind).cmp(&(&b.task_id, b.kind)));
    if pairs
        .windows(2)
        .any(|w| w[0].task_id == w[1].task_id && w[0].kind == w[1].kind)
    {
        return Err(EvalError::failed(format!("{what}: duplicate parity pair")));
    }
    Ok(pairs)
}

/// Normalize one parity entry.
fn parity_pair(entry: &Value, what: &str) -> Result<ParityPair, EvalError> {
    let kind_name = entry
        .get("variant")
        .or_else(|| entry.get("completion_kind"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let kind = match kind_name {
        "reference" | "reference_full" => ParityKind::Full,
        "reference_half" | "reference_half_chars" => ParityKind::Half,
        other => {
            return Err(EvalError::failed(format!(
                "{what}: unknown parity kind {other:?}"
            )));
        }
    };
    let tokens = entry
        .get("completion_tokens")
        .or_else(|| entry.get("token_count"))
        .and_then(Value::as_u64)
        .filter(|tokens| *tokens >= 1)
        .ok_or_else(|| EvalError::failed(format!("{what}: parity token count unusable")))?;
    Ok(ParityPair {
        task_id: req_str(entry, "task_id", what)?.to_string(),
        kind,
        tokens,
        mean_logprob: req_f64(entry, "mean_logprob", what)?,
    })
}

/// The receipt's own explanation of a parity divergence, when it
/// carries one: a top-level `parity_status` (Burn), or the `status`
/// of an object-shaped parity batch (Candle).
fn parity_explanation(receipt: &Value) -> Option<String> {
    if let Some(status) = receipt.get("parity_status").and_then(Value::as_str)
        && !status.trim().is_empty()
    {
        return Some(status.to_string());
    }
    receipt
        .pointer("/parity/status")
        .and_then(Value::as_str)
        .filter(|status| !status.trim().is_empty())
        .map(str::to_string)
}

/// Proof the adapter actually moved: a before/after adapter digest
/// pair that differs, or LoRA-B norms that left zero. A receipt with
/// no movement evidence — or evidence of no movement — fails: per
/// the contract, a receipt asserts weights moved.
fn check_adapter_moved(receipt: &Value) -> Result<(), EvalError> {
    let what = "trainer receipt";
    let after = req_str(receipt, "adapter_sha256", what)?;
    if !is_sha256_hex(after) {
        return Err(EvalError::failed(format!(
            "{what}: adapter_sha256 malformed"
        )));
    }
    let before_hash = receipt
        .get("adapter_sha256_before")
        .and_then(Value::as_str)
        .or_else(|| {
            receipt
                .pointer("/adapter_stats_before/adapter_sha256")
                .and_then(Value::as_str)
        });
    if let Some(before) = before_hash {
        if before == after {
            return Err(EvalError::failed(format!(
                "{what}: adapter digest unchanged; weights did not move"
            )));
        }
        return Ok(());
    }
    let norm_pairs = [
        (
            "/adapter_stats_before/lora_b_l2_norm",
            "/adapter_stats_after/lora_b_l2_norm",
        ),
        ("/adapter_norm_sq_before", "/adapter_norm_sq_after"),
    ];
    for (before_path, after_path) in norm_pairs {
        if let (Some(before), Some(after_norm)) = (
            receipt.pointer(before_path).and_then(Value::as_f64),
            receipt.pointer(after_path).and_then(Value::as_f64),
        ) {
            if after_norm > before {
                return Ok(());
            }
            return Err(EvalError::failed(format!(
                "{what}: adapter norms did not grow ({before} -> {after_norm})"
            )));
        }
    }
    Err(EvalError::failed(format!(
        "{what}: no adapter-movement evidence (no before digest, no norms)"
    )))
}

/// The parity guard: compare the candidate's parity batch against the
/// reference receipt's, pair by pair. Returns the maximum divergence.
/// A divergence past tolerance with no explanation in the candidate
/// receipt is a guard failure — the iteration is discarded, per the
/// design's guard-metrics rule.
fn parity_guard(candidate: &Value, reference: &Value) -> Result<f64, EvalError> {
    let candidate_pairs = parity_pairs(candidate, "trainer receipt")?;
    let reference_pairs = parity_pairs(reference, "parity reference receipt")?;
    let mut max_divergence = 0.0_f64;
    for pair in &candidate_pairs {
        let reference_pair = reference_pairs
            .iter()
            .find(|other| other.task_id == pair.task_id && other.kind == pair.kind)
            .ok_or_else(|| {
                EvalError::failed(format!(
                    "parity guard: reference has no pair for {} {:?}",
                    pair.task_id, pair.kind
                ))
            })?;
        if reference_pair.tokens != pair.tokens {
            return Err(EvalError::failed(format!(
                "parity guard: token count mismatch on {} {:?} ({} vs {})",
                pair.task_id, pair.kind, pair.tokens, reference_pair.tokens
            )));
        }
        max_divergence =
            max_divergence.max((pair.mean_logprob - reference_pair.mean_logprob).abs());
    }
    if max_divergence > PARITY_TOLERANCE_NATS_PER_TOKEN && parity_explanation(candidate).is_none() {
        return Err(EvalError::failed(format!(
            "parity guard: max divergence {max_divergence:.4} nats/token exceeds \
             {PARITY_TOLERANCE_NATS_PER_TOKEN} with no explanation in the receipt"
        )));
    }
    Ok(max_divergence)
}

/// A trainer receipt that survived every check.
struct VerifiedTrainer {
    backend: String,
    sha256: String,
    max_parity_divergence: Option<f64>,
}

/// Verify the whole trainer evidence chain, in dependency order:
/// trainer receipt shape → the trainlab receipt it names (hash) →
/// the groups export against that receipt → the export's hash →
/// adapter movement → the parity guard.
fn verify_trainer_evidence(evidence: &TrainerEvidence) -> Result<VerifiedTrainer, EvalError> {
    let trainer = read_evidence(&evidence.trainer_receipt_path, "trainer receipt")?;
    let what = "trainer receipt";
    check_schema(&trainer.json, TRAINER_RECEIPT_SCHEMA, what)?;
    let backend = req_str(&trainer.json, "backend", what)?.to_string();
    if !TRAINER_BACKENDS.contains(&backend.as_str()) {
        return Err(EvalError::failed(format!(
            "{what}: unknown backend {backend:?}"
        )));
    }
    for field in [
        "trainlab_receipt_sha256",
        "groups_export_sha256",
        "base_model_sha256",
    ] {
        let value = req_str(&trainer.json, field, what)?;
        if !is_sha256_hex(value) {
            return Err(EvalError::failed(format!(
                "{what}: field {field} is not a SHA-256 digest"
            )));
        }
    }
    req_str(&trainer.json, "base_model", what)?;
    if trainer.json.get("seed").and_then(Value::as_i64).is_none() {
        return Err(EvalError::failed(format!("{what}: seed missing")));
    }
    if req_f64(&trainer.json, "wall_clock_seconds", what)? <= 0.0 {
        return Err(EvalError::failed(format!(
            "{what}: wall_clock_seconds not positive"
        )));
    }
    let versions = trainer
        .json
        .get("framework_versions")
        .or_else(|| trainer.json.get("framework"))
        .filter(|value| value.is_object());
    if versions.is_none() {
        return Err(EvalError::failed(format!(
            "{what}: framework versions missing"
        )));
    }
    if !trainer
        .json
        .get("hyperparameters")
        .is_some_and(Value::is_object)
    {
        return Err(EvalError::failed(format!(
            "{what}: hyperparameters missing"
        )));
    }
    // The hash chain: the trainer receipt names the exact trainlab
    // artifacts it consumed; the files on disk must be those artifacts.
    let trainlab = read_evidence(&evidence.trainlab_receipt_path, "trainlab receipt")?;
    check_schema(&trainlab.json, TRAINLAB_RECEIPT_SCHEMA, "trainlab receipt")?;
    if trainlab.sha256 != req_str(&trainer.json, "trainlab_receipt_sha256", what)? {
        return Err(EvalError::failed(
            "hash chain break: trainlab receipt bytes do not match the trainer receipt's digest",
        ));
    }
    let export = read_evidence(&evidence.groups_export_path, "groups export")?;
    check_groups_export(&export.json, &trainlab.json)?;
    if export.sha256 != req_str(&trainer.json, "groups_export_sha256", what)? {
        return Err(EvalError::failed(
            "hash chain break: groups export bytes do not match the trainer receipt's digest",
        ));
    }
    if let Some(config) = trainer.json.get("config_sha256").and_then(Value::as_str)
        && config != req_str(&export.json, "config_sha256", "groups export")?
    {
        return Err(EvalError::failed(
            "trainer receipt: config_sha256 does not match the groups export",
        ));
    }
    check_adapter_moved(&trainer.json)?;
    // The parity batch must at least be well-formed even when no
    // reference is registered; with one, the guard compares.
    let mut max_parity_divergence = None;
    if let Some(reference_path) = &evidence.parity_reference_path {
        let reference = read_evidence(reference_path, "parity reference receipt")?;
        check_schema(
            &reference.json,
            TRAINER_RECEIPT_SCHEMA,
            "parity reference receipt",
        )?;
        max_parity_divergence = Some(parity_guard(&trainer.json, &reference.json)?);
    } else {
        parity_pairs(&trainer.json, what)?;
    }
    Ok(VerifiedTrainer {
        backend,
        sha256: trainer.sha256,
        max_parity_divergence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::ChangeKind;
    use crate::clock::ManualClock;
    use crate::testsupport::test_dir;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/");

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(format!("{FIXTURES}{name}"))
    }

    fn change_set(id: &str) -> ChangeSet {
        ChangeSet {
            id: id.to_string(),
            kind: ChangeKind::TrainlabConfig,
            paths: Vec::new(),
            payload: "{}".to_string(),
            rationale: "test".to_string(),
        }
    }

    fn file_sha256(path: &Path) -> String {
        format!(
            "{:x}",
            Sha256::digest(std::fs::read(path).expect("read fixture"))
        )
    }

    /// The real PyTorch RLOO bundle: dev receipt for the metric, the
    /// RL receipt + export the trainer consumed, itself as reference.
    fn pytorch_bundle() -> EvaluationBundle {
        EvaluationBundle {
            dev_receipt_path: fixture("dev-qwen25-coder-7b-phase0-20261008-receipt.json"),
            trainer: Some(TrainerEvidence {
                trainer_receipt_path: fixture("trainer-receipt-pytorch.json"),
                trainlab_receipt_path: fixture("rl-qwen25-coder-7b-t10-20261008-receipt.json"),
                groups_export_path: fixture("rl-qwen25-coder-7b-t10-20261008-groups.json"),
                parity_reference_path: Some(fixture("trainer-receipt-pytorch.json")),
            }),
        }
    }

    #[test]
    fn real_pytorch_artifacts_produce_metric_and_hashes() {
        let clock = ManualClock::new();
        let mut evaluator = ReceiptEvaluator::new().with_bundle("rloo", pytorch_bundle());
        let evaluation = evaluator
            .evaluate(&change_set("rloo"), &clock)
            .expect("real artifacts verify");
        assert_eq!(evaluation.metric, 1.0);
        assert_eq!(
            evaluation.trainlab_receipt_sha256.as_deref(),
            Some(
                file_sha256(&fixture("dev-qwen25-coder-7b-phase0-20261008-receipt.json")).as_str()
            )
        );
        assert_eq!(
            evaluation.trainer_receipt_sha256.as_deref(),
            Some(file_sha256(&fixture("trainer-receipt-pytorch.json")).as_str())
        );
        assert!(
            evaluation.description.contains("parity max |d| 0.0000"),
            "{}",
            evaluation.description
        );
    }

    #[test]
    fn synthetic_mixed_dev_receipt_yields_exact_mean() {
        let clock = ManualClock::new();
        let bundle = EvaluationBundle {
            dev_receipt_path: fixture("dev-receipt-synthetic-mixed.json"),
            trainer: None,
        };
        let mut evaluator = ReceiptEvaluator::new().with_bundle("mixed", bundle);
        let evaluation = evaluator
            .evaluate(&change_set("mixed"), &clock)
            .expect("synthetic receipt verifies");
        assert!((evaluation.metric - 0.4375).abs() < 1e-12);
        assert_eq!(evaluation.trainer_receipt_sha256, None);
    }

    #[test]
    fn burn_candidate_passes_guard_with_its_quantization_explanation() {
        // Burn's f32 parity diverges from the NF4 PyTorch reference by
        // ~0.075 nats/token — past tolerance — but its receipt carries
        // the contract-required explanation, so the guard records the
        // divergence instead of discarding the iteration.
        let clock = ManualClock::new();
        let bundle = EvaluationBundle {
            dev_receipt_path: fixture("dev-qwen25-coder-7b-phase0-20261008-receipt.json"),
            trainer: Some(TrainerEvidence {
                trainer_receipt_path: fixture("trainer-receipt-burn.json"),
                trainlab_receipt_path: fixture("rl-qwen25-coder-7b-t10-20261008-receipt.json"),
                groups_export_path: fixture("rl-qwen25-coder-7b-t10-20261008-groups.json"),
                parity_reference_path: Some(fixture("trainer-receipt-pytorch.json")),
            }),
        };
        let mut evaluator = ReceiptEvaluator::new().with_bundle("burn", bundle);
        let evaluation = evaluator
            .evaluate(&change_set("burn"), &clock)
            .expect("explained divergence passes the guard");
        assert!(
            evaluation.description.contains("parity max |d| 0.0886"),
            "{}",
            evaluation.description
        );
    }

    #[test]
    fn candle_receipt_with_unmoved_adapter_is_rejected() {
        // The real Candle gate-C receipt: adapter norms 0.0 -> 0.0 (its
        // 7B step never landed). The hash chain verifies; movement
        // does not, and a receipt asserts weights moved.
        let clock = ManualClock::new();
        let bundle = EvaluationBundle {
            dev_receipt_path: fixture("dev-qwen25-coder-7b-phase0-20261008-receipt.json"),
            trainer: Some(TrainerEvidence {
                trainer_receipt_path: fixture("trainer-receipt-candle.json"),
                trainlab_receipt_path: fixture("dev-qwen25-coder-7b-phase0-20261008-receipt.json"),
                groups_export_path: fixture("dev-qwen25-coder-7b-phase0-20261008-groups.json"),
                parity_reference_path: None,
            }),
        };
        let mut evaluator = ReceiptEvaluator::new().with_bundle("candle", bundle);
        let err = evaluator
            .evaluate(&change_set("candle"), &clock)
            .expect_err("unmoved adapter must fail");
        assert!(err.message.contains("did not grow"), "{}", err.message);
    }

    #[test]
    fn schema_mismatch_is_rejected() {
        let dir = test_dir("schema");
        let path = dir.path().join("dev.json");
        let mut json: Value = serde_json::from_slice(
            &std::fs::read(fixture("dev-receipt-synthetic-mixed.json")).expect("read"),
        )
        .expect("parse");
        json["schema"] = Value::from("phlow.trainlab.receipt/v2");
        std::fs::write(&path, serde_json::to_vec(&json).expect("encode")).expect("write");
        let bundle = EvaluationBundle {
            dev_receipt_path: path,
            trainer: None,
        };
        let mut evaluator = ReceiptEvaluator::new().with_bundle("x", bundle);
        let err = evaluator
            .evaluate(&change_set("x"), &ManualClock::new())
            .expect_err("schema mismatch must fail");
        assert!(err.message.contains("schema mismatch"), "{}", err.message);
    }

    #[test]
    fn hash_chain_break_is_rejected() {
        // Point the PyTorch trainer receipt at the dev receipt instead
        // of the RL receipt it actually consumed: bytes no longer
        // match the digest the trainer recorded.
        let clock = ManualClock::new();
        let mut bundle = pytorch_bundle();
        bundle
            .trainer
            .as_mut()
            .expect("trainer")
            .trainlab_receipt_path = fixture("dev-qwen25-coder-7b-phase0-20261008-receipt.json");
        let mut evaluator = ReceiptEvaluator::new().with_bundle("rloo", bundle);
        let err = evaluator
            .evaluate(&change_set("rloo"), &clock)
            .expect_err("broken chain must fail");
        assert!(err.message.contains("hash chain break"), "{}", err.message);
    }

    #[test]
    fn tampered_groups_export_is_rejected() {
        let dir = test_dir("tamper");
        let path = dir.path().join("groups.json");
        let mut json: Value = serde_json::from_slice(
            &std::fs::read(fixture("rl-qwen25-coder-7b-t10-20261008-groups.json")).expect("read"),
        )
        .expect("parse");
        let current = json["groups"][0]["rewards"][0]
            .as_f64()
            .expect("reward is a number");
        json["groups"][0]["rewards"][0] = Value::from(if current == 1.0 { 0.0 } else { 1.0 });
        std::fs::write(&path, serde_json::to_vec(&json).expect("encode")).expect("write");
        let clock = ManualClock::new();
        let mut bundle = pytorch_bundle();
        bundle.trainer.as_mut().expect("trainer").groups_export_path = path;
        let mut evaluator = ReceiptEvaluator::new().with_bundle("rloo", bundle);
        let err = evaluator
            .evaluate(&change_set("rloo"), &clock)
            .expect_err("tampered export must fail");
        assert!(
            err.message.contains("disagree with the receipt"),
            "{}",
            err.message
        );
    }

    #[test]
    fn oversized_evidence_file_is_rejected() {
        let dir = test_dir("oversized");
        let path = dir.path().join("big.json");
        let bytes = vec![b' '; (EVIDENCE_FILE_BYTES_MAX + 1) as usize];
        std::fs::write(&path, bytes).expect("write");
        let bundle = EvaluationBundle {
            dev_receipt_path: path,
            trainer: None,
        };
        let mut evaluator = ReceiptEvaluator::new().with_bundle("x", bundle);
        let err = evaluator
            .evaluate(&change_set("x"), &ManualClock::new())
            .expect_err("oversized file must fail");
        assert!(err.message.contains("evidence bound"), "{}", err.message);
    }

    #[test]
    fn malformed_json_and_missing_bundle_are_rejected() {
        let dir = test_dir("malformed");
        let path = dir.path().join("dev.json");
        std::fs::write(&path, b"{not json").expect("write");
        let bundle = EvaluationBundle {
            dev_receipt_path: path,
            trainer: None,
        };
        let mut evaluator = ReceiptEvaluator::new().with_bundle("x", bundle);
        let err = evaluator
            .evaluate(&change_set("x"), &ManualClock::new())
            .expect_err("malformed JSON must fail");
        assert!(err.message.contains("malformed JSON"), "{}", err.message);
        let err = evaluator
            .evaluate(&change_set("unregistered"), &ManualClock::new())
            .expect_err("missing bundle must fail");
        assert!(
            err.message.contains("no evidence bundle"),
            "{}",
            err.message
        );
    }

    #[test]
    fn non_dev_split_receipt_is_rejected_as_metric_source() {
        let bundle = EvaluationBundle {
            dev_receipt_path: fixture("rl-qwen25-coder-7b-t10-20261008-receipt.json"),
            trainer: None,
        };
        let mut evaluator = ReceiptEvaluator::new().with_bundle("x", bundle);
        let err = evaluator
            .evaluate(&change_set("x"), &ManualClock::new())
            .expect_err("rl split must fail as a dev metric source");
        assert!(err.message.contains("split is not dev"), "{}", err.message);
    }

    #[test]
    fn unexplained_parity_divergence_trips_the_guard() {
        // The Burn receipt minus its parity_status explanation: the
        // same ~0.075 divergence must now discard the iteration.
        let dir = test_dir("guard");
        let path = dir.path().join("trainer.json");
        let mut json: Value = serde_json::from_slice(
            &std::fs::read(fixture("trainer-receipt-burn.json")).expect("read"),
        )
        .expect("parse");
        json.as_object_mut()
            .expect("object")
            .remove("parity_status");
        std::fs::write(&path, serde_json::to_vec(&json).expect("encode")).expect("write");
        let bundle = EvaluationBundle {
            dev_receipt_path: fixture("dev-qwen25-coder-7b-phase0-20261008-receipt.json"),
            trainer: Some(TrainerEvidence {
                trainer_receipt_path: path,
                trainlab_receipt_path: fixture("rl-qwen25-coder-7b-t10-20261008-receipt.json"),
                groups_export_path: fixture("rl-qwen25-coder-7b-t10-20261008-groups.json"),
                parity_reference_path: Some(fixture("trainer-receipt-pytorch.json")),
            }),
        };
        let mut evaluator = ReceiptEvaluator::new().with_bundle("burn", bundle);
        let err = evaluator
            .evaluate(&change_set("burn"), &ManualClock::new())
            .expect_err("unexplained divergence must fail the guard");
        assert!(err.message.contains("parity guard"), "{}", err.message);
    }
}
