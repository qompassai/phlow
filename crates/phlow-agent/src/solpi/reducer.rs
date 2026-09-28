//! Evidence-Preserving Reducer: compact receipts only on byte-for-byte proof.
//!
//! Re-expresses SoL-Pi's Evidence-Preserving Reducer ("long diagnostic
//! logs become compact receipts only when every retained quotation matches
//! the archived source"). A caller proposes a reduction — a summary plus
//! the quotations it retains — and the reducer verifies every quotation
//! against the source byte-for-byte. On the first mismatch the proposal is
//! rejected and the original is left unchanged; failures never produce a
//! partial or edited receipt.
//!
//! The source is only borrowed, never mutated, so "unchanged" is
//! structural: a rejected reduction cannot alter the caller's text.

use std::fmt;

/// Maximum source bytes accepted for reduction.
pub const SOURCE_BYTES_MAX: usize = 1024 * 1024;

/// Maximum summary bytes accepted in a proposal.
pub const SUMMARY_BYTES_MAX: usize = 8 * 1024;

/// Maximum quotations accepted in one proposal.
pub const QUOTES_MAX: usize = 64;

/// Maximum bytes of a single quotation.
pub const QUOTE_BYTES_MAX: usize = 8 * 1024;

/// Maximum characters of an unchanged-reason string.
pub const REASON_CHARS_MAX: usize = 512;

/// A proposed reduction: a summary plus the exact quotations it retains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReductionProposal {
    /// The compact summary text.
    pub summary: String,
    /// Quotations the summary retains; each must occur in the source
    /// byte-for-byte.
    pub quotes: Vec<String>,
}

/// A verified compact receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactReceipt {
    /// The proposal's summary, unchanged.
    pub summary: String,
    /// The verified quotations, unchanged and in proposal order.
    pub quotes: Vec<String>,
    /// Source byte length the quotes were verified against.
    pub source_bytes: usize,
    /// Number of quotations verified (== `quotes.len()`; the equality is
    /// the invariant, the field makes it observable).
    pub verified_quotes: usize,
}

/// The outcome of a reduction attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReductionOutcome {
    /// Every quotation verified byte-for-byte; the receipt is safe to keep
    /// in place of the source.
    Compacted(CompactReceipt),
    /// Verification failed: the original must be kept unchanged. Carries
    /// the bounded reason; no partial receipt is produced.
    Unchanged {
        /// Why the proposal was rejected, bounded to
        /// [`REASON_CHARS_MAX`] characters.
        reason: String,
    },
}

/// Input-bound violations: caller bugs, not verification outcomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReducerError {
    /// The reducer is not opted in. Nothing is verified.
    NotEnabled,
    /// The source exceeds [`SOURCE_BYTES_MAX`].
    SourceTooLarge {
        /// Source byte length.
        bytes: usize,
        /// The bound in force.
        max: usize,
    },
    /// The summary exceeds [`SUMMARY_BYTES_MAX`].
    SummaryTooLarge {
        /// Summary byte length.
        bytes: usize,
        /// The bound in force.
        max: usize,
    },
    /// The proposal carries more than [`QUOTES_MAX`] quotations.
    TooManyQuotes {
        /// Quotation count.
        count: usize,
        /// The bound in force.
        max: usize,
    },
    /// One quotation exceeds [`QUOTE_BYTES_MAX`].
    QuoteTooLarge {
        /// Quotation index in the proposal.
        index: usize,
        /// Quotation byte length.
        bytes: usize,
        /// The bound in force.
        max: usize,
    },
}

impl fmt::Display for ReducerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReducerError::NotEnabled => write!(f, "evidence reducer is not enabled"),
            ReducerError::SourceTooLarge { bytes, max } => {
                write!(f, "source of {bytes} bytes exceeds the {max}-byte limit")
            }
            ReducerError::SummaryTooLarge { bytes, max } => {
                write!(f, "summary of {bytes} bytes exceeds the {max}-byte limit")
            }
            ReducerError::TooManyQuotes { count, max } => {
                write!(f, "{count} quotations exceed the {max}-quote limit")
            }
            ReducerError::QuoteTooLarge { index, bytes, max } => write!(
                f,
                "quotation {index} of {bytes} bytes exceeds the {max}-byte limit"
            ),
        }
    }
}

impl std::error::Error for ReducerError {}

/// The evidence-preserving reducer. Disabled by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvidenceReducer {
    enabled: bool,
}

impl EvidenceReducer {
    /// Disabled reducer: [`Self::reduce`] refuses with
    /// [`ReducerError::NotEnabled`].
    pub fn new() -> Self {
        Self { enabled: false }
    }

    /// Explicit opt-in.
    pub fn opt_in() -> Self {
        Self { enabled: true }
    }

    /// True only after explicit [`Self::opt_in`].
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Verify a reduction proposal against its source.
    ///
    /// Contract: input bounds are checked first ([`ReducerError`] on
    /// violation); then every quotation must occur in `source`
    /// byte-for-byte and be non-empty. The first failure yields
    /// [`ReductionOutcome::Unchanged`] and the source — borrowed, never
    /// mutated — is left exactly as passed in.
    pub fn reduce(
        &self,
        source: &str,
        proposal: &ReductionProposal,
    ) -> Result<ReductionOutcome, ReducerError> {
        if !self.enabled {
            return Err(ReducerError::NotEnabled);
        }
        check_bounds(source, proposal)?;

        for (index, quote) in proposal.quotes.iter().enumerate() {
            if quote.is_empty() {
                return Ok(unchanged(format!(
                    "quotation {index} is empty; an empty quotation matches \
                     everything and proves nothing"
                )));
            }
            if !source.contains(quote.as_str()) {
                return Ok(unchanged(format!(
                    "quotation {index} does not occur in the source byte-for-byte"
                )));
            }
        }

        Ok(ReductionOutcome::Compacted(CompactReceipt {
            summary: proposal.summary.clone(),
            quotes: proposal.quotes.clone(),
            source_bytes: source.len(),
            verified_quotes: proposal.quotes.len(),
        }))
    }
}

impl Default for EvidenceReducer {
    /// Default is disabled, matching the missing-config rule.
    fn default() -> Self {
        Self::new()
    }
}

/// Build an [`ReductionOutcome::Unchanged`] with a bounded reason.
fn unchanged(reason: String) -> ReductionOutcome {
    let bounded: String = reason.chars().take(REASON_CHARS_MAX).collect();
    ReductionOutcome::Unchanged { reason: bounded }
}

/// Validate proposal input bounds before any verification work.
fn check_bounds(source: &str, proposal: &ReductionProposal) -> Result<(), ReducerError> {
    if source.len() > SOURCE_BYTES_MAX {
        return Err(ReducerError::SourceTooLarge {
            bytes: source.len(),
            max: SOURCE_BYTES_MAX,
        });
    }
    if proposal.summary.len() > SUMMARY_BYTES_MAX {
        return Err(ReducerError::SummaryTooLarge {
            bytes: proposal.summary.len(),
            max: SUMMARY_BYTES_MAX,
        });
    }
    if proposal.quotes.len() > QUOTES_MAX {
        return Err(ReducerError::TooManyQuotes {
            count: proposal.quotes.len(),
            max: QUOTES_MAX,
        });
    }
    for (index, quote) in proposal.quotes.iter().enumerate() {
        if quote.len() > QUOTE_BYTES_MAX {
            return Err(ReducerError::QuoteTooLarge {
                index,
                bytes: quote.len(),
                max: QUOTE_BYTES_MAX,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        EvidenceReducer, QUOTE_BYTES_MAX, QUOTES_MAX, ReducerError, ReductionOutcome,
        ReductionProposal, SOURCE_BYTES_MAX, SUMMARY_BYTES_MAX,
    };

    fn proposal(summary: &str, quotes: &[&str]) -> ReductionProposal {
        ReductionProposal {
            summary: summary.to_owned(),
            quotes: quotes.iter().map(|q| q.to_string()).collect(),
        }
    }

    // ---------------- validation tests ----------------

    #[test]
    fn reducer_compacts_when_every_quote_matches() {
        let reducer = EvidenceReducer::opt_in();
        let source = "error: broken pipe\nat net.rs:42\nretrying in 5s\n";
        let outcome = reducer
            .reduce(
                source,
                &proposal("pipe broke; retrying", &["broken pipe", "net.rs:42"]),
            )
            .expect("opted-in reducer runs");
        match outcome {
            ReductionOutcome::Compacted(receipt) => {
                assert_eq!(receipt.summary, "pipe broke; retrying");
                assert_eq!(receipt.verified_quotes, 2);
                assert_eq!(receipt.quotes.len(), 2);
                assert_eq!(receipt.source_bytes, source.len());
            }
            ReductionOutcome::Unchanged { .. } => panic!("expected compaction"),
        }
    }

    #[test]
    fn reducer_accepts_empty_quote_list() {
        // Vacuous verification: no quotations retained, nothing to prove.
        let reducer = EvidenceReducer::opt_in();
        let outcome = reducer
            .reduce("some log", &proposal("nothing retained", &[]))
            .expect("opted-in reducer runs");
        assert!(matches!(outcome, ReductionOutcome::Compacted(_)));
    }

    #[test]
    fn reducer_matches_quotes_at_source_boundaries() {
        let reducer = EvidenceReducer::opt_in();
        let source = "START middle END";
        let outcome = reducer
            .reduce(source, &proposal("edges", &["START", "END"]))
            .expect("opted-in reducer runs");
        assert!(matches!(outcome, ReductionOutcome::Compacted(_)));
    }

    #[test]
    fn reducer_accepts_duplicate_identical_quotes() {
        let reducer = EvidenceReducer::opt_in();
        let outcome = reducer
            .reduce("abc abc", &proposal("twice", &["abc", "abc"]))
            .expect("opted-in reducer runs");
        match outcome {
            ReductionOutcome::Compacted(receipt) => assert_eq!(receipt.verified_quotes, 2),
            ReductionOutcome::Unchanged { .. } => panic!("expected compaction"),
        }
    }

    #[test]
    fn reducer_leaves_source_unmodified_on_rejection() {
        let reducer = EvidenceReducer::opt_in();
        let source = String::from("stable log line\n");
        let before = source.clone();
        let outcome = reducer
            .reduce(&source, &proposal("wrong", &["not in the source"]))
            .expect("opted-in reducer runs");
        assert!(matches!(outcome, ReductionOutcome::Unchanged { .. }));
        assert_eq!(source, before, "source borrowed, never mutated");
    }

    // ---------------- adversarial tests ----------------

    #[test]
    fn reducer_refuses_without_opt_in() {
        let reducer = EvidenceReducer::new();
        assert!(!reducer.is_enabled());
        let result = reducer.reduce("log", &proposal("s", &["log"]));
        assert_eq!(result, Err(ReducerError::NotEnabled));
    }

    #[test]
    fn reducer_rejects_single_byte_tampered_quote() {
        // Crafted adversarial data: one byte changed (AML.T0043).
        let reducer = EvidenceReducer::opt_in();
        let source = "checksum 9f2ac4 ok";
        let outcome = reducer
            .reduce(source, &proposal("checksum", &["checksum 9f2ac5 ok"]))
            .expect("opted-in reducer runs");
        match outcome {
            ReductionOutcome::Unchanged { reason } => {
                assert!(reason.contains("quotation 0"));
            }
            ReductionOutcome::Compacted(_) => panic!("tampered quote must not verify"),
        }
    }

    #[test]
    fn reducer_rejects_absent_quote() {
        let reducer = EvidenceReducer::opt_in();
        let outcome = reducer
            .reduce("real log", &proposal("fabricated", &["database dropped"]))
            .expect("opted-in reducer runs");
        assert!(matches!(outcome, ReductionOutcome::Unchanged { .. }));
    }

    #[test]
    fn reducer_rejects_empty_quote() {
        // An empty quotation matches everything; accepting it would let a
        // proposal claim evidence it does not have.
        let reducer = EvidenceReducer::opt_in();
        let outcome = reducer
            .reduce("anything", &proposal("sneaky", &[""]))
            .expect("opted-in reducer runs");
        match outcome {
            ReductionOutcome::Unchanged { reason } => assert!(reason.contains("empty")),
            ReductionOutcome::Compacted(_) => panic!("empty quote must not verify"),
        }
    }

    #[test]
    fn reducer_rejects_near_match_case_changed_quote() {
        // Poisoned-receipt attempt: a plausible but inexact quotation must
        // not enter a compact receipt (AML.T0020).
        let reducer = EvidenceReducer::opt_in();
        let source = "ERROR: disk full";
        let outcome = reducer
            .reduce(source, &proposal("disk issue", &["error: disk full"]))
            .expect("opted-in reducer runs");
        assert!(
            matches!(outcome, ReductionOutcome::Unchanged { .. }),
            "byte-for-byte means case counts"
        );
        let oversized = "s".repeat(SOURCE_BYTES_MAX + 1);
        let bound = reducer.reduce(&oversized, &proposal("s", &[]));
        assert_eq!(
            bound,
            Err(ReducerError::SourceTooLarge {
                bytes: SOURCE_BYTES_MAX + 1,
                max: SOURCE_BYTES_MAX,
            })
        );
        let big_summary = proposal(&"s".repeat(SUMMARY_BYTES_MAX + 1), &[]);
        assert_eq!(
            reducer.reduce("ok", &big_summary),
            Err(ReducerError::SummaryTooLarge {
                bytes: SUMMARY_BYTES_MAX + 1,
                max: SUMMARY_BYTES_MAX,
            })
        );
        let many_quotes = ReductionProposal {
            summary: "s".to_owned(),
            quotes: vec!["q".to_owned(); QUOTES_MAX + 1],
        };
        assert_eq!(
            reducer.reduce("q", &many_quotes),
            Err(ReducerError::TooManyQuotes {
                count: QUOTES_MAX + 1,
                max: QUOTES_MAX,
            })
        );
        let big_quote = proposal("s", &[&"q".repeat(QUOTE_BYTES_MAX + 1)]);
        assert!(matches!(
            reducer.reduce("q", &big_quote),
            Err(ReducerError::QuoteTooLarge { index: 0, .. })
        ));
    }
}
