//! task-12: poisoned context compaction (rust).
//!
//! Adversarial task against the real `EvidenceReducer` from the phlow-agent
//! crate (`crates/phlow-agent/src/solpi/reducer.rs`) — the SoL-Pi
//! "Evidence-Preserving Reducer" port: compact receipts only when every
//! retained quotation matches the source byte-for-byte; opt-in; failures
//! leave results unchanged.
//!
//! The attack: proposals carrying POISONED evidence — a fabricated quotation
//! invented by the attacker, a quotation with subtly altered wording, and a
//! receipt tampered with after verification. The reducer must REFUSE to
//! compact (return the input unchanged) rather than launder the poison into
//! a clean-looking receipt. A clean proposal (control) must still compact.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-12";
/// Human-readable name.
pub const NAME: &str = "poisoned context compaction";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// The real reducer, compiled verbatim from the phlow-agent crate.
///
/// phlow-gauntlet deliberately has no dependency on phlow-agent (disjoint
/// crate boundaries), so the shipped source is included by path instead of
/// reimplemented: this task exercises the exact code that ships, and the
/// learning doc cites that file as the primary source.
#[path = "../../../phlow-agent/src/solpi/reducer.rs"]
mod real_reducer;

// Re-exported so the integration tests drive the same real code as the
// task module — no reimplementation, no mock of the unit under test.
pub use real_reducer::{
    CompactReceipt, EvidenceReducer, ReducerError, ReductionOutcome, ReductionProposal,
};

/// Demo source: agent context (tool results) the attacker wants compacted
/// into a lie. Small on purpose; the reducer's own bounds are exercised by
/// its unit tests.
const SOURCE: &str = "\
[tool:read_file] /etc/app/config.yaml\n\
listen: 0.0.0.0:8080\n\
tls: enabled\n\
[tool:run_tests] 42 passed, 0 failed\n\
[tool:git_status] branch=main clean=true\n";

/// Exact quotations from [`SOURCE`] that a clean proposal retains.
const CLEAN_QUOTES: &[&str] = &[
    "tls: enabled",
    "42 passed, 0 failed",
    "branch=main clean=true",
];

/// Build a proposal from a summary and borrowed quotations.
fn proposal(summary: &str, quotes: &[&str]) -> ReductionProposal {
    ReductionProposal {
        summary: summary.to_owned(),
        quotes: quotes.iter().map(|q| q.to_string()).collect(),
    }
}

/// Control: a well-formed proposal compacts, and every retained quotation is
/// independently re-verified against the source byte-for-byte.
fn check_control_compacts(reducer: &EvidenceReducer) -> Result<String, String> {
    let outcome = reducer
        .reduce(SOURCE, &proposal("config ok; tests green", CLEAN_QUOTES))
        .map_err(|e| format!("control reduce errored: {e}"))?;
    let ReductionOutcome::Compacted(receipt) = outcome else {
        return Err("control proposal was refused; expected compaction".to_string());
    };
    if receipt.verified_quotes != CLEAN_QUOTES.len() {
        return Err(format!(
            "verified_quotes={} != retained={}",
            receipt.verified_quotes,
            CLEAN_QUOTES.len()
        ));
    }
    for quote in &receipt.quotes {
        if !SOURCE.contains(quote.as_str()) {
            return Err(format!("retained quote not in source: {quote:?}"));
        }
    }
    Ok(format!(
        "control compacted: {} quotes verified byte-for-byte against {} source bytes",
        receipt.verified_quotes, receipt.source_bytes
    ))
}

/// Attack 1: fabricated quote — invented text attributed to the real source.
/// Must be refused, with the source left exactly as passed in.
fn check_fabricated_quote_refused(reducer: &EvidenceReducer) -> Result<String, String> {
    let before = SOURCE.to_owned();
    let outcome = reducer
        .reduce(
            SOURCE,
            &proposal("config compromised", &["tls: disabled; backdoor on :9090"]),
        )
        .map_err(|e| format!("fabricated reduce errored: {e}"))?;
    let ReductionOutcome::Unchanged { reason } = outcome else {
        return Err("fabricated quote was COMPACTED: poison laundered into a receipt".to_string());
    };
    if SOURCE != before {
        return Err("source mutated during refused reduction".to_string());
    }
    if reason.is_empty() {
        return Err("refusal carried no reason".to_string());
    }
    Ok(format!(
        "fabricated quote refused; source unchanged; reason: {reason}"
    ))
}

/// Attack 2: subtly altered quote — one digit flipped, reversing the meaning
/// of the test result while looking almost identical. Must be refused.
fn check_altered_quote_refused(reducer: &EvidenceReducer) -> Result<String, String> {
    let outcome = reducer
        .reduce(
            SOURCE,
            &proposal("tests mostly green", &["42 passed, 1 failed"]),
        )
        .map_err(|e| format!("altered reduce errored: {e}"))?;
    match outcome {
        ReductionOutcome::Unchanged { reason } => {
            Ok(format!("one-digit-altered quote refused; reason: {reason}"))
        }
        ReductionOutcome::Compacted(_) => {
            Err("altered quote was COMPACTED: poison laundered into a receipt".to_string())
        }
    }
}

/// Attack 3: tampered receipt — a legitimately compacted receipt is edited
/// after the fact, then offered back as a new proposal. There is no
/// acceptance path for receipts except fresh re-verification through
/// `reduce()`, so the tamper must be caught.
fn check_tampered_receipt_refused(reducer: &EvidenceReducer) -> Result<String, String> {
    let outcome = reducer
        .reduce(SOURCE, &proposal("config ok; tests green", CLEAN_QUOTES))
        .map_err(|e| format!("baseline reduce errored: {e}"))?;
    let ReductionOutcome::Compacted(mut receipt) = outcome else {
        return Err("baseline proposal refused; cannot build tampered receipt".to_string());
    };
    // Attacker edits the verified receipt post-hoc.
    let slot = receipt
        .quotes
        .iter()
        .position(|q| q == "tls: enabled")
        .ok_or_else(|| "baseline receipt missing expected quote".to_string())?;
    receipt.quotes[slot] = "tls: disabled".to_owned();
    // The only way back into a receipt is a fresh proposal: re-verified
    // byte-for-byte against the source.
    let forged = ReductionProposal {
        summary: receipt.summary.clone(),
        quotes: receipt.quotes.clone(),
    };
    let again = reducer
        .reduce(SOURCE, &forged)
        .map_err(|e| format!("forged reduce errored: {e}"))?;
    match again {
        ReductionOutcome::Unchanged { reason } => Ok(format!(
            "tampered receipt re-proposal refused; reason: {reason}"
        )),
        ReductionOutcome::Compacted(_) => {
            Err("tampered receipt was RE-COMPACTED without re-verification".to_string())
        }
    }
}

/// Defense-in-depth: the reducer is opt-in; a non-opted-in reducer verifies
/// nothing at all.
fn check_requires_opt_in() -> Result<String, String> {
    let reducer = EvidenceReducer::new();
    match reducer.reduce(SOURCE, &proposal("x", CLEAN_QUOTES)) {
        Err(ReducerError::NotEnabled) => {
            Ok("disabled reducer refused to verify (NotEnabled)".to_string())
        }
        Err(e) => Err(format!("disabled reducer gave wrong error: {e}")),
        Ok(_) => Err("disabled reducer verified a proposal".to_string()),
    }
}

/// Attempt the task: run the control and the poison attacks against the real
/// reducer. Pass iff the control compacts and every attack is refused with
/// the input unchanged.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    let reducer = EvidenceReducer::opt_in();
    let results: [(&str, Result<String, String>); 5] = [
        ("control-compacts", check_control_compacts(&reducer)),
        (
            "fabricated-quote-refused",
            check_fabricated_quote_refused(&reducer),
        ),
        (
            "altered-quote-refused",
            check_altered_quote_refused(&reducer),
        ),
        (
            "tampered-receipt-refused",
            check_tampered_receipt_refused(&reducer),
        ),
        ("requires-opt-in", check_requires_opt_in()),
    ];
    let mut evidence = vec![
        "reducer=crates/phlow-agent/src/solpi/reducer.rs (included verbatim via #[path]; no reimplementation)"
            .to_string(),
        format!("source_bytes={}", SOURCE.len()),
    ];
    for (name, result) in results {
        match result {
            Ok(detail) => evidence.push(format!("check={name} result=pass {detail}")),
            Err(how) => {
                evidence.push(format!("check={name} result=FAIL {how}"));
                return TaskOutcome::Fail {
                    where_: name.to_string(),
                    how,
                    evidence: crate::bound_evidence(evidence),
                };
            }
        }
    }
    TaskOutcome::Pass {
        evidence: crate::bound_evidence(evidence),
    }
}
