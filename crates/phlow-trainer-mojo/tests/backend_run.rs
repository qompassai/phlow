//! Integration tests for the production backend surface
//! (`backend::run_scoring`): the trainlab groups/receipt contract,
//! the GPU-policy fallback path, and receipt write/refuse semantics.
//!
//! Inventory: 4 validation (v_*) + 2 adversarial (a_*) tests. All
//! fixtures are synthesized in a temp directory — no external files.
//! The whole file is gated on the `trainlab-contract` feature: the
//! backend surface it tests does not exist without it.
#![cfg(feature = "trainlab-contract")]

use std::fs;
use std::path::PathBuf;

use phlow_trainer_mojo::backend::{self, BackendConfig, LogitBlock, ManifestEntry};
use std::sync::Mutex;

use phlow_trainer_mojo::error::ScoringError;

/// GPU work in this process is serialized: parallel test threads
/// launching kernels through separate device contexts in one
/// process can stall on context contention (observed on primo,
/// 2026-10-09). One lock per test file; tests stay independent.
static GPU_LOCK: Mutex<()> = Mutex::new(());

fn gpu_guard() -> std::sync::MutexGuard<'static, ()> {
    GPU_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
use phlow_trainer_mojo::reference::{self, SampleParams};
use phlow_trainlab::{GroupExportRecord, GroupsExport};

const VOCAB: usize = 8;
const CONFIG_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// Two completions over a fixed 8-token vocab: completion A is
/// uniform (mean logprob -ln 8); completion B puts its mass on the
/// targets (higher mean). Manifest values come from the Rust
/// reference, so both dispatch paths must agree with them.
fn fixture() -> (Vec<ManifestEntry>, Vec<LogitBlock>) {
    let logits_a = vec![0.0_f32; 3 * VOCAB];
    let mut logits_b = vec![0.0_f32; 2 * VOCAB];
    logits_b[5] = 4.0; // row 0 target index 5
    logits_b[VOCAB + 2] = 4.0; // row 1 target index 2
    let targets_a = vec![0_i32, 3, 7];
    let targets_b = vec![5_i32, 2];
    let mut entries = Vec::new();
    let mut blocks = Vec::new();
    for (logits, rows, targets) in [
        (logits_a, 3_usize, targets_a),
        (logits_b, 2_usize, targets_b),
    ] {
        let token_logps: Vec<f64> =
            reference::logprob_token_logps(&logits, rows, VOCAB, &targets)
                .iter()
                .map(|value| f64::from(*value))
                .collect();
        let mean = token_logps.iter().sum::<f64>() / token_logps.len() as f64;
        entries.push(ManifestEntry {
            targets: targets.clone(),
            token_logps,
            mean_logprob: mean,
        });
        blocks.push(LogitBlock {
            logits,
            rows,
            vocab: VOCAB,
            targets,
        });
    }
    (entries, blocks)
}

/// Set up the export + trainlab receipt in a fresh temp dir. Rewards
/// [1, 0] give exact LOO advantages [1, -1] and spread 1.
fn setup(run_id: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "phlow-trainer-mojo-backend-{}-{}",
        std::process::id(),
        run_id
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("mkdir");
    let export = GroupsExport {
        schema: phlow_trainlab::GROUPS_SCHEMA.to_string(),
        run_id: run_id.to_string(),
        config_sha256: CONFIG_SHA.to_string(),
        split: "dev".to_string(),
        sampler_id: "fixture".to_string(),
        groups: vec![GroupExportRecord {
            group: 1,
            task_id: "fixture-task".to_string(),
            family: "fixture".to_string(),
            prompt: "fixture prompt".to_string(),
            completions: vec!["aaa".to_string(), "bb".to_string()],
            rewards: vec![1.0, 0.0],
            advantages: vec![1.0, -1.0],
            reward_spread: 1.0,
            updated: true,
        }],
    };
    let export_path = dir.join("groups.json");
    fs::write(
        &export_path,
        serde_json::to_string_pretty(&export).expect("export json"),
    )
    .expect("write export");
    let receipt_path = dir.join("trainlab-receipt.json");
    fs::write(
        &receipt_path,
        format!("{{\"run_id\": \"{run_id}\", \"config_sha256\": \"{CONFIG_SHA}\"}}"),
    )
    .expect("write receipt");
    (dir, export_path, receipt_path)
}

#[test]
fn v_run_scores_and_writes_receipt() {
    let _gpu = gpu_guard();
    let (entries, blocks) = fixture();
    let (dir, export_path, receipt_path) = setup("run-ok");
    let config = BackendConfig::default();
    let outcome = backend::run_scoring(
        &export_path,
        &receipt_path,
        1,
        &entries,
        &blocks,
        None,
        &config,
    )
    .expect("run_scoring");
    assert!(outcome.worst_token_diff <= 5e-4, "token diff");
    assert!(outcome.worst_mean_diff <= 5e-4, "mean diff");
    assert!(outcome.worst_advantage_diff <= 1e-5, "advantage diff");
    assert_eq!(outcome.receipt.backend, "mojo");
    assert_eq!(outcome.receipt.groups.len(), 1);
    assert_eq!(outcome.receipt.groups[0].mean_logprobs.len(), 2);
    assert_eq!(outcome.receipt.groups[0].advantages, vec![1.0, -1.0]);
    let receipt_out = dir.join("scoring-receipt.json");
    outcome.receipt.write_new(&receipt_out).expect("write receipt");
    let text = fs::read_to_string(&receipt_out).expect("read receipt");
    assert!(text.contains("phlow.trainer-mojo.scoring-receipt/v1"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn v_run_forced_fallback_records_reference_path() {
    let _gpu = gpu_guard();
    let (entries, blocks) = fixture();
    let (dir, export_path, receipt_path) = setup("run-fallback");
    let config = BackendConfig {
        gpu_free_mib_min: u64::MAX,
        allow_reference_fallback: true,
    };
    let outcome = backend::run_scoring(
        &export_path,
        &receipt_path,
        1,
        &entries,
        &blocks,
        None,
        &config,
    )
    .expect("run_scoring fallback");
    assert_eq!(outcome.receipt.scoring_path, "reference");
    assert!(outcome.receipt.fallback_reason.is_some());
    assert_eq!(outcome.receipt.gpu_free_mib_min, u64::MAX);
    assert!(outcome.worst_mean_diff <= 1e-9, "reference is exact here");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn v_run_records_sampler_and_tokens() {
    let _gpu = gpu_guard();
    let (entries, blocks) = fixture();
    let (dir, export_path, receipt_path) = setup("run-sampler");
    let params = SampleParams {
        temperature: 0.8,
        top_k: 4,
        top_p: 0.95,
        seed: 77,
        draw_index: 2,
    };
    let outcome = backend::run_scoring(
        &export_path,
        &receipt_path,
        1,
        &entries,
        &blocks,
        Some(params),
        &BackendConfig::default(),
    )
    .expect("run_scoring sampler");
    let record = outcome.receipt.sampler.expect("sampler record");
    assert_eq!(record.seed, 77);
    assert_eq!(record.top_k, 4);
    let tokens = outcome.sampled_tokens.expect("sampled tokens");
    assert_eq!(tokens.len(), 3, "first block has 3 rows");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_run_refuses_receipt_overwrite() {
    let _gpu = gpu_guard();
    let (entries, blocks) = fixture();
    let (dir, export_path, receipt_path) = setup("run-overwrite");
    let outcome = backend::run_scoring(
        &export_path,
        &receipt_path,
        1,
        &entries,
        &blocks,
        None,
        &BackendConfig::default(),
    )
    .expect("run_scoring");
    let receipt_out = dir.join("scoring-receipt.json");
    outcome.receipt.write_new(&receipt_out).expect("first write");
    assert!(matches!(
        outcome.receipt.write_new(&receipt_out),
        Err(ScoringError::Contract { .. })
    ));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_run_rejects_tampered_receipt_tie() {
    let _gpu = gpu_guard();
    let (entries, blocks) = fixture();
    let (dir, export_path, receipt_path) = setup("run-tampered");
    // Rewrite the trainlab receipt with a different run id.
    fs::write(
        &receipt_path,
        format!("{{\"run_id\": \"someone-else\", \"config_sha256\": \"{CONFIG_SHA}\"}}"),
    )
    .expect("rewrite receipt");
    assert!(matches!(
        backend::run_scoring(
            &export_path,
            &receipt_path,
            1,
            &entries,
            &blocks,
            None,
            &BackendConfig::default(),
        ),
        Err(ScoringError::Contract { .. })
    ));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_run_rejects_unknown_group() {
    let _gpu = gpu_guard();
    let (entries, blocks) = fixture();
    let (dir, export_path, receipt_path) = setup("run-bad-group");
    assert!(matches!(
        backend::run_scoring(
            &export_path,
            &receipt_path,
            9,
            &entries,
            &blocks,
            None,
            &BackendConfig::default(),
        ),
        Err(ScoringError::Contract { .. })
    ));
    let _ = fs::remove_dir_all(&dir);
}
