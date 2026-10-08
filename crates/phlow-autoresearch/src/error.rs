//! The single typed error for the crate, plus the failure taxonomy the
//! ledger records. Failure classes are data, not just diagnostics: the
//! loop's halt decisions are computed from them, and a human reviewing
//! the ledger reads them as the experiment's history.

use std::fmt;

/// Why one iteration (or the whole run) failed. Serialized into ledger
/// entries verbatim, so names are part of the on-disk contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    /// The proposer emitted something invalid: schema, bounds, or a
    /// change-set aimed at a surface it may not touch in this way.
    ProposalInvalid,
    /// A valid change-set that did not apply or did not build.
    ApplyFailed,
    /// The sampler, executor, or trainer errored during evaluation.
    EvaluationFailed,
    /// The per-experiment wall-clock budget was exceeded.
    EvaluationTimeout,
    /// Evaluation returned without a usable metric or with malformed
    /// evidence (e.g. a receipt hash that is not a SHA-256 hex digest).
    MetricMissing,
    /// The ledger's hash chain did not verify. Always run-ending.
    LedgerCorrupt,
    /// An iteration or total wall-clock budget was reached.
    BudgetExhausted,
    /// A change-set aimed at a forbidden surface: paths outside the
    /// experiment worktree, the fixed harness, the ledger itself, or
    /// confirmation/promotion state. Recorded distinctly because it is
    /// the class a human most wants to see; two occurrences halt a run.
    GateViolation,
}

impl FailureClass {
    /// Stable snake-case name, identical to the serialized form.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            FailureClass::ProposalInvalid => "proposal_invalid",
            FailureClass::ApplyFailed => "apply_failed",
            FailureClass::EvaluationFailed => "evaluation_failed",
            FailureClass::EvaluationTimeout => "evaluation_timeout",
            FailureClass::MetricMissing => "metric_missing",
            FailureClass::LedgerCorrupt => "ledger_corrupt",
            FailureClass::BudgetExhausted => "budget_exhausted",
            FailureClass::GateViolation => "gate_violation",
        }
    }
}

/// Every error the orchestration can return. Expected failures (bad
/// input, corrupt evidence, exhausted budgets) are values here, never
/// panics; `assert!` is reserved for relationships a correct
/// implementation has already established.
#[derive(Debug)]
pub enum AutoresearchError {
    /// A caller-supplied value exceeded a named bound.
    LimitExceeded(String),
    /// A caller-supplied value was invalid for a non-size reason.
    Invalid(String),
    /// The ledger hash chain failed verification; the loop must not run.
    LedgerCorrupt(String),
    /// An underlying I/O failure.
    Io(std::io::Error),
    /// A JSON encode/decode failure.
    Json(serde_json::Error),
}

impl fmt::Display for AutoresearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AutoresearchError::LimitExceeded(msg) => write!(f, "limit exceeded: {msg}"),
            AutoresearchError::Invalid(msg) => write!(f, "invalid: {msg}"),
            AutoresearchError::LedgerCorrupt(msg) => write!(f, "ledger corrupt: {msg}"),
            AutoresearchError::Io(err) => write!(f, "io: {err}"),
            AutoresearchError::Json(err) => write!(f, "json: {err}"),
        }
    }
}

impl std::error::Error for AutoresearchError {}

impl From<std::io::Error> for AutoresearchError {
    fn from(err: std::io::Error) -> Self {
        AutoresearchError::Io(err)
    }
}

impl From<serde_json::Error> for AutoresearchError {
    fn from(err: serde_json::Error) -> Self {
        AutoresearchError::Json(err)
    }
}
