//! Integration tests for the Rust→Mojo FFI boundary.
//!
//! Inventory: 8 validation (v_*) + 2 adversarial (a_*) tests.
//! Validation tests prove known answers cross the boundary intact —
//! including the sampling kernel's exact agreement with the pure-Rust
//! reference on fixed logits; adversarial tests prove hostile shapes
//! are rejected with typed errors before any device work. These
//! tests need the built kernel library and a GPU; they run on primo
//! as part of the crate's own gates.

use std::sync::Mutex;

use phlow_trainer_mojo::error::ScoringError;

/// GPU work in this process is serialized: parallel test threads
/// launching kernels through separate device contexts in one
/// process can stall on context contention (observed on primo,
/// 2026-10-09). One lock per test file; tests stay independent.
static GPU_LOCK: Mutex<()> = Mutex::new(());

fn gpu_guard() -> std::sync::MutexGuard<'static, ()> {
    GPU_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
use phlow_trainer_mojo::ffi;
use phlow_trainer_mojo::reference::{self, SampleParams};

#[test]
fn v_version_is_two() {
    let _gpu = gpu_guard();
    assert_eq!(ffi::scoring_version(), 2);
}

#[test]
fn v_uniform_logits_logprob() {
    let _gpu = gpu_guard();
    // Uniform logits over an 8-wide vocab: every logprob is -ln(8).
    let logits = vec![0.0_f32; 16];
    let targets = vec![0_i32, 7];
    let got = ffi::logprob_token_logps(&logits, 2, 8, &targets).expect("kernel call");
    let want = -f32::ln(8.0);
    for value in got {
        assert!((value - want).abs() < 1e-5, "got {value}, want {want}");
    }
}

#[test]
fn v_advantages_known_groups() {
    let _gpu = gpu_guard();
    // Groups [1, 0] and [1, 1, 0]: LOO advantages [1, -1] and [.5, .5, -1].
    let rewards = vec![1.0_f32, 0.0, 1.0, 1.0, 0.0];
    let offsets = vec![0_i32, 2, 5];
    let got = ffi::rloo_advantages(&rewards, &offsets).expect("kernel call");
    let want = [1.0_f32, -1.0, 0.5, 0.5, -1.0];
    for (g, w) in got.iter().zip(want.iter()) {
        assert!((g - w).abs() < 1e-6, "got {g}, want {w}");
    }
}

/// Deterministic pseudo-logits (a small LCG; no external fixtures).
fn pseudo_logits(rows: usize, vocab: usize, seed: u32) -> Vec<f32> {
    let mut state = seed;
    let mut out = Vec::with_capacity(rows * vocab);
    for _ in 0..rows * vocab {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let unit = (state >> 8) as f32 / (1_u32 << 24) as f32;
        out.push(unit * 6.0 - 3.0);
    }
    out
}

fn sample_params(temperature: f32, top_k: u32, top_p: f32) -> SampleParams {
    SampleParams {
        temperature,
        top_k,
        top_p,
        seed: 1234,
        draw_index: 3,
    }
}

#[test]
fn v_sample_greedy_matches_argmax() {
    let _gpu = gpu_guard();
    // Row 0 has an exact tie at the max: the lowest index must win.
    let mut logits = vec![1.0_f32, 3.0, 3.0, 2.0];
    logits.extend([0.5_f32, -1.0, 4.0, 0.0]);
    let batch = ffi::sample_tokens(&logits, 2, 4, &sample_params(0.0, 0, 1.0)).expect("sample");
    assert_eq!(batch.tokens, vec![1, 2]);
    assert_eq!(batch.candidate_counts, vec![1, 1]);
}

#[test]
fn v_sample_deterministic_across_calls() {
    let _gpu = gpu_guard();
    // Same (logits, params) twice through the FFI: identical tokens,
    // counts, and candidate prefixes — the receipt-reproducibility
    // property at the boundary.
    let logits = pseudo_logits(4, 257, 7);
    let params = sample_params(0.8, 50, 0.95);
    let first = ffi::sample_tokens(&logits, 4, 257, &params).expect("sample 1");
    let second = ffi::sample_tokens(&logits, 4, 257, &params).expect("sample 2");
    assert_eq!(first, second);
}

#[test]
fn v_sample_topk_candidate_set() {
    let _gpu = gpu_guard();
    let logits: Vec<f32> = (0..16).map(|i| i as f32 * 0.5).collect();
    let batch = ffi::sample_tokens(&logits, 1, 16, &sample_params(1.0, 4, 1.0)).expect("sample");
    assert_eq!(batch.candidate_counts[0], 4);
    let max = phlow_trainer_mojo::SAMPLE_CANDIDATES_MAX as usize;
    assert_eq!(&batch.cand_idx[0..4], &[15, 14, 13, 12]);
    assert!(batch.cand_val[0] > batch.cand_val[3]);
    assert!((12..=15).contains(&batch.tokens[0]));
    let _ = max;
}

#[test]
fn v_sample_nucleus_dominant_token() {
    let _gpu = gpu_guard();
    let mut logits = vec![0.0_f32; 64];
    logits[9] = 20.0;
    let batch = ffi::sample_tokens(&logits, 1, 64, &sample_params(1.0, 0, 0.9)).expect("sample");
    assert_eq!(batch.candidate_counts[0], 1);
    assert_eq!(batch.tokens[0], 9);
}

#[test]
fn v_sample_matches_reference_batch() {
    let _gpu = gpu_guard();
    // Kernel vs the independent Rust reference on fixed logits:
    // tokens and candidate prefixes must agree exactly. (The walk's
    // float sums differ in order between the implementations, so the
    // fixed seeds here are the evidence; the Python harness covers
    // the statistical agreement over many draws.)
    let logits = pseudo_logits(4, 257, 42);
    for params in [
        sample_params(1.0, 0, 1.0),
        sample_params(0.7, 0, 1.0),
        sample_params(1.0, 10, 1.0),
        sample_params(1.0, 0, 0.9),
        sample_params(0.8, 50, 0.95),
        sample_params(1.3, 5, 0.5),
    ] {
        let batch = ffi::sample_tokens(&logits, 4, 257, &params).expect("sample");
        let want = reference::sample_batch(&logits, 4, 257, &params);
        let max = phlow_trainer_mojo::SAMPLE_CANDIDATES_MAX as usize;
        for (row, outcome) in want.iter().enumerate() {
            assert_eq!(
                batch.tokens[row], outcome.token,
                "token mismatch params {params:?} row {row}"
            );
            assert_eq!(
                batch.candidate_counts[row] as usize, outcome.candidate_count,
                "count mismatch params {params:?} row {row}"
            );
            if !outcome.candidates.is_empty() {
                assert_eq!(
                    &batch.cand_idx[row * max..row * max + outcome.candidates.len()],
                    &outcome.candidates[..],
                    "candidate prefix mismatch params {params:?} row {row}"
                );
            }
        }
    }
}

#[test]
fn a_rejects_oversized_rows() {
    let _gpu = gpu_guard();
    let rows = phlow_trainer_mojo::TOKENS_PER_LAUNCH_MAX as usize + 1;
    let logits = vec![0.0_f32; rows];
    let targets = vec![0_i32; rows];
    assert!(matches!(
        ffi::logprob_token_logps(&logits, rows, 1, &targets),
        Err(ScoringError::InvalidShape { .. })
    ));
}

#[test]
fn a_rejects_decreasing_offsets() {
    let _gpu = gpu_guard();
    let rewards = vec![1.0_f32, 0.0, 1.0];
    let offsets = vec![0_i32, 3, 2];
    assert!(matches!(
        ffi::rloo_advantages(&rewards, &offsets),
        Err(ScoringError::InvalidShape { .. })
    ));
}
