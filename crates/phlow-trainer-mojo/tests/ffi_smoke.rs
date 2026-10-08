//! Integration tests for the Rust→Mojo FFI boundary.
//!
//! Inventory: 3 validation (v_*) + 2 adversarial (a_*) tests. Validation
//! tests prove known answers cross the boundary intact; adversarial
//! tests prove hostile shapes are rejected with typed errors before
//! any device work. These tests need the pixi-built kernel library and
//! a GPU; they run on primo as part of the experiment's own gates.

use phlow_trainer_mojo::error::ScoringError;
use phlow_trainer_mojo::ffi;

#[test]
fn v_version_is_one() {
    assert_eq!(ffi::scoring_version(), 1);
}

#[test]
fn v_uniform_logits_logprob() {
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
    // Groups [1, 0] and [1, 1, 0]: LOO advantages [1, -1] and [.5, .5, -1].
    let rewards = vec![1.0_f32, 0.0, 1.0, 1.0, 0.0];
    let offsets = vec![0_i32, 2, 5];
    let got = ffi::rloo_advantages(&rewards, &offsets).expect("kernel call");
    let want = [1.0_f32, -1.0, 0.5, 0.5, -1.0];
    for (g, w) in got.iter().zip(want.iter()) {
        assert!((g - w).abs() < 1e-6, "got {g}, want {w}");
    }
}

#[test]
fn a_rejects_oversized_rows() {
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
    let rewards = vec![1.0_f32, 0.0, 1.0];
    let offsets = vec![0_i32, 3, 2];
    assert!(matches!(
        ffi::rloo_advantages(&rewards, &offsets),
        Err(ScoringError::InvalidShape { .. })
    ));
}
