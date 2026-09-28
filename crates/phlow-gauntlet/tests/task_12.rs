//! Integration tests for task-12: poisoned context compaction.
//!
//! 50/50 split against the real `EvidenceReducer` — the task module includes
//! the shipped phlow-agent source verbatim (`#[path]`), so there is no
//! reimplementation and no mock of the unit under test:
//! 2 validation (clean evidence compacts with every retained quote matching
//! its source; a failed reduction leaves the input unchanged, never partially
//! compacted) + 2 adversarial (a fabricated quote attributed to a real source
//! is rejected; a subtly altered quote is rejected). A fifth test drives the
//! task module's `run()` end to end.

use phlow_gauntlet::tasks::task_12::{EvidenceReducer, ReductionOutcome, ReductionProposal};
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;

/// Realistic agent context: tool results an attacker wants compacted into a
/// lie. Mirrors the source used by the task driver.
const SOURCE: &str = "\
[tool:read_file] /etc/app/config.yaml\n\
listen: 0.0.0.0:8080\n\
tls: enabled\n\
[tool:run_tests] 42 passed, 0 failed\n\
[tool:git_status] branch=main clean=true\n";

/// Build a proposal from a summary and borrowed quotations.
fn proposal(summary: &str, quotes: &[&str]) -> ReductionProposal {
    ReductionProposal {
        summary: summary.to_owned(),
        quotes: quotes.iter().map(|q| q.to_string()).collect(),
    }
}

/// V: well-formed evidence compacts, and every retained quote is verified to
/// occur in the source byte-for-byte (independent re-check, not the
/// reducer's word for it).
#[test]
fn well_formed_evidence_compacts_and_quotes_match_source() {
    let reducer = EvidenceReducer::opt_in();
    let quotes = ["tls: enabled", "42 passed, 0 failed"];
    let outcome = reducer
        .reduce(SOURCE, &proposal("all green", &quotes))
        .expect("opted-in reducer must run");
    let ReductionOutcome::Compacted(receipt) = outcome else {
        panic!("clean proposal must compact, evidence: {outcome:?}");
    };
    assert_eq!(
        receipt.verified_quotes,
        quotes.len(),
        "verified count must equal retained count"
    );
    assert_eq!(receipt.quotes.len(), quotes.len());
    assert_eq!(receipt.summary, "all green");
    assert_eq!(receipt.source_bytes, SOURCE.len());
    for quote in &receipt.quotes {
        assert!(
            SOURCE.contains(quote.as_str()),
            "retained quote must occur in the source byte-for-byte: {quote:?}"
        );
    }
}

/// V: a refused reduction leaves the input unchanged — the outcome is
/// `Unchanged`, never a partial or edited receipt, and the source bytes are
/// identical before and after.
#[test]
fn failed_reduction_leaves_input_unchanged() {
    let reducer = EvidenceReducer::opt_in();
    let source = String::from(SOURCE);
    let before = source.clone();
    let outcome = reducer
        .reduce(&source, &proposal("lies", &["the moon is made of cheese"]))
        .expect("opted-in reducer must run");
    match outcome {
        ReductionOutcome::Unchanged { reason } => {
            assert!(!reason.is_empty(), "refusal must carry a reason");
            assert!(
                reason.contains("quotation 0"),
                "reason must name the offending quotation, got: {reason}"
            );
        }
        ReductionOutcome::Compacted(_) => panic!("poisoned proposal must not compact"),
    }
    assert_eq!(
        source, before,
        "refused reduction must not mutate the source"
    );
}

/// A: fabricated quote — invented text attributed to a real source. The wolf
/// writes a plausible-sounding claim and pins it on genuine context; the
/// reducer must refuse to launder it into a receipt.
#[test]
fn fabricated_quote_attributed_to_real_source_is_rejected() {
    let reducer = EvidenceReducer::opt_in();
    let outcome = reducer
        .reduce(
            SOURCE,
            &proposal("breach detected", &["tls: disabled; exfil to 10.0.0.9"]),
        )
        .expect("opted-in reducer must run");
    match outcome {
        ReductionOutcome::Unchanged { reason } => {
            assert!(
                reason.contains("quotation 0"),
                "reason must name the fabricated quotation, got: {reason}"
            );
        }
        ReductionOutcome::Compacted(_) => {
            panic!("fabricated quote must be rejected, not compacted")
        }
    }
}

/// A: subtly altered quote — one digit flipped (`0` -> `1`), reversing the
/// meaning of the test result while looking almost identical. Byte-for-byte
/// means even this must fail.
#[test]
fn single_digit_altered_quote_is_rejected() {
    let reducer = EvidenceReducer::opt_in();
    let outcome = reducer
        .reduce(SOURCE, &proposal("tests", &["42 passed, 1 failed"]))
        .expect("opted-in reducer must run");
    assert!(
        matches!(outcome, ReductionOutcome::Unchanged { .. }),
        "one-digit alteration must be rejected byte-for-byte, got: {outcome:?}"
    );
}

/// End to end: the task driver runs all five checks (control + three poison
/// attacks + opt-in gate) and reports pass with one evidence line per check.
#[test]
fn task_driver_reports_pass_with_evidence_per_check() {
    // task_12::run is a Rust-kind driver: it never spawns nvim, so these
    // dummy paths are never touched; Ctx::new only requires non-empty.
    let ctx = Ctx::new(
        PathBuf::from("task-12-test/dummy-nvim"),
        PathBuf::from("task-12-test/dummy-diver-lua"),
        std::env::temp_dir().join("gauntlet-task-12-driver"),
    )
    .expect("Ctx::new must accept non-empty paths");
    match phlow_gauntlet::tasks::task_12::run(&ctx) {
        TaskOutcome::Pass { evidence } => {
            assert!(!evidence.is_empty(), "pass must carry evidence");
            for name in [
                "control-compacts",
                "fabricated-quote-refused",
                "altered-quote-refused",
                "tampered-receipt-refused",
                "requires-opt-in",
            ] {
                assert!(
                    evidence
                        .iter()
                        .any(|line| line.contains(&format!("check={name} result=pass"))),
                    "missing pass evidence line for check {name}: {evidence:?}"
                );
            }
        }
        TaskOutcome::Fail { where_, how, .. } => {
            panic!("task driver failed at {where_}: {how}")
        }
    }
}
