//! Integration tests for task-76 (tokenizer boundary mismatch).
//!
//! The seam is ABSENT: no tokenizer binding exists in any phlow crate
//! (exact-token source scan for `tokenizer` over the product crates:
//! zero hits; no tokenizer dependency in any Cargo.toml). The
//! runtime's context ruler is CHARACTERS — `role_turn` ends the role
//! with "Context budget exhausted" when the serialized char count
//! exceeds `max_context_chars`; truncation is char-boundary
//! (`truncate_chars`) and can split the serving model's tokens with no
//! detection or repair; `max_tokens_for(context_length) =
//! min(8192, context_length / 2)` is an operator-configured number
//! with no tokenizer behind it, presented in token units with no
//! estimate flag; the serving model's tokenizer lives inside Ollama
//! and is invisible to phlow.
//!
//! Whether phlow should gain a tokenizer binding (budgeting with the
//! serving model's tokenizer, re-encoding truncation through it,
//! warning on X/Y divergence) is a product decision for Matt —
//! banked, not implemented on gauntlet authority.
//!
//! Four cases — 2 validation, 2 adversarial — each self-checking:
//! cases probe the seam and record measured mechanism evidence; the
//! driver then reports the honest seam failure.

use phlow_gauntlet::tasks::task_76;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

fn ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-76: cannot build Ctx: {e}"))
}

// --- validation ---

/// V1: the char budget is the ruler — the budget expression
/// `must_dumps(messages).chars().count()` is measured on the fixture
/// (the same expression `role_turn` compares against
/// `max_context_chars`), and the same text counted by the mock
/// tokenizers disagrees with the char count: chars are not tokens.
/// The X-vs-X corpus check shows the mocks are sound (identical
/// tokenizers agree). The task-level driver then runs all four cases
/// and reports the honest seam failure: no tokenizer binding exists,
/// so the X/Y divergence the design wants measured cannot be
/// measured by phlow — the tokenizer-binding product decision is
/// banked in the task-level `how`, not implemented on gauntlet
/// authority.
#[test]
fn char_budget_is_the_ruler() {
    assert_eq!(task_76::ID, "task-76");
    assert_eq!(task_76::NAME, "tokenizer boundary mismatch");
    assert_eq!(task_76::KIND, TaskKind::Rust);
    assert_eq!(task_76::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_76::run_case("char_budget_is_the_ruler")
        .unwrap_or_else(|e| panic!("task-76 case failed to run: {e}"));
    assert!(
        report.passed,
        "char budget case must hold: {}",
        report.failures.join("; ")
    );
    assert!(
        report.metrics["char_count"].as_u64().unwrap() > 0,
        "metrics must show the measured char count"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("must_dumps(messages).chars().count()"),
        "evidence must show the budget expression:\\n{joined}"
    );
    assert!(
        joined.contains("X-vs-X on the corpus: counts match exactly"),
        "evidence must show the mocks are sound:\\n{joined}"
    );
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` banks the tokenizer-binding product decision for Matt.
    let (where_, how) = match task_76::run(&ctx()) {
        TaskOutcome::Fail { where_, how, .. } => (where_, how),
        TaskOutcome::Pass { evidence } => panic!(
            "task-76 passed: a tokenizer binding was invented, not found\\nevidence: {evidence:?}"
        ),
    };
    assert_eq!(where_, "seam", "task-76 must fail at the seam");
    assert!(
        how.contains("product decision for Matt"),
        "the 'how' must bank the product decision: {how}"
    );
    assert!(
        how.contains("no tokenizer binding"),
        "the 'how' must name the missing binding: {how}"
    );
}

/// V2: the design's mismatch scenario — mock tokenizers X and Y
/// diverge on the corpus, and the divergence is MEASURED and
/// SURFACED (both counts recorded, the X count flagged as an
/// estimate), never hidden. Phlow's real rulers
/// (`max_context_chars` in chars, `max_tokens_for` in token units)
/// are neither the serving model's tokenizer.
#[test]
fn mock_tokenizers_diverge_and_surface() {
    let report = task_76::run_case("mock_tokenizers_diverge_and_surface")
        .unwrap_or_else(|e| panic!("task-76 case failed to run: {e}"));
    assert!(
        report.passed,
        "divergence case must hold: {}",
        report.failures.join("; ")
    );
    assert!(
        report.metrics["divergent"].as_u64().unwrap() > 0,
        "the mocks must diverge on at least one fixture"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("divergence measured"),
        "evidence must show the divergence was measured:\\n{joined}"
    );
}

// --- adversarial ---

/// A1: truncation at a char boundary (what `truncate_chars` does)
/// lands mid-token in Y — the mock re-encode remedy yields Y's
/// canonical sequence, but phlow has no serving tokenizer to
/// re-encode through: nothing detects or repairs the split.
#[test]
fn char_truncation_splits_model_tokens() {
    let report = task_76::run_case("char_truncation_splits_model_tokens")
        .unwrap_or_else(|e| panic!("task-76 case failed to run: {e}"));
    assert!(
        report.passed,
        "truncation case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(
        report.metrics["split_y_token"], true,
        "the char cut must split a Y token"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("mid-token cut in Y"),
        "evidence must show the split:\\n{joined}"
    );
    assert!(
        joined.contains("nothing detects or repairs it"),
        "evidence must state the gap:\\n{joined}"
    );
}

/// A2: budget accounting uses a tokenizer-free number while the
/// provider bills in tokens — `max_tokens_for` feeds the real Ollama
/// payload's `max_tokens` field (8192 for the default config) with
/// no tokenizer behind it and no estimate flag.
#[test]
fn config_number_presented_as_tokens() {
    let report = task_76::run_case("config_number_presented_as_tokens")
        .unwrap_or_else(|e| panic!("task-76 case failed to run: {e}"));
    assert!(
        report.passed,
        "config-number case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["max_tokens"], 8192);
    assert_eq!(report.metrics["presented_as_y"], true);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no tokenizer involved"),
        "evidence must show the number is configured, not tokenized:\\n{joined}"
    );
}
