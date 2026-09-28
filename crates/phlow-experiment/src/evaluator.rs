//! Evaluator skeleton: stages, budgets, and evidence.
//!
//! The evaluator is deliberately a skeleton: it encodes the stage order
//! (Validate → Prepare → Execute → Verify → Review → Promote), the
//! fail-closed budget rules, and the shape of evidence — not the execution
//! of checks, models, or tools. [`Evaluator::verify`] never reports success
//! by default: missing evidence yields `Ok(false)`, and timeout or
//! exhausted budget yields an error.

use crate::error::ExperimentError;

// ---------------------------------------------------------------------------
// Bounds (all with units)
// ---------------------------------------------------------------------------

/// Maximum checks recorded in one evidence bundle.
pub const CHECKS_MAX: usize = 64;
/// Maximum artifact digests recorded in one evidence bundle.
pub const ARTIFACTS_MAX: usize = 64;
/// Maximum coverage entries in one verification outcome.
pub const COVERAGE_MAX: usize = 128;
/// Maximum characters in a check or artifact name.
pub const EVIDENCE_NAME_CHARS_MAX: usize = 256;
/// Maximum characters in one argv entry.
pub const ARG_ENTRY_CHARS_MAX: usize = 1_024;
/// Maximum argv entries in one check record.
pub const CHECK_ARGV_MAX: usize = 64;

// ---------------------------------------------------------------------------
// Evaluation stages
// ---------------------------------------------------------------------------

/// The evaluator's stages, in required order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalStage {
    /// Validate inputs, contracts, and budgets before any work.
    Validate,
    /// Prepare the isolated workspace and immutable snapshots.
    Prepare,
    /// Execute the bounded task.
    Execute,
    /// Verify the result against host-collected evidence.
    Verify,
    /// Independent review of the result and evidence.
    Review,
    /// Hand the verified result to the promotion gate.
    Promote,
}

impl EvalStage {
    /// The stable machine-readable name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::Prepare => "prepare",
            Self::Execute => "execute",
            Self::Verify => "verify",
            Self::Review => "review",
            Self::Promote => "promote",
        }
    }

    /// The stage that must follow `self`, or `None` after Promote.
    pub fn next(self) -> Option<EvalStage> {
        match self {
            Self::Validate => Some(Self::Prepare),
            Self::Prepare => Some(Self::Execute),
            Self::Execute => Some(Self::Verify),
            Self::Verify => Some(Self::Review),
            Self::Review => Some(Self::Promote),
            Self::Promote => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Evidence
// ---------------------------------------------------------------------------

/// One check run, with the exact argv the host executed.
///
/// Recording argv (not just a pass/fail bit) is what lets a later audit
/// distinguish "the host ran the check" from "the model claimed it ran".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRun {
    name: String,
    argv: Vec<String>,
    passed: bool,
}

impl CheckRun {
    /// Records a check run. Rejected: empty or over-long name, empty argv,
    /// too many argv entries, or an over-long argv entry.
    pub fn new(name: &str, argv: Vec<String>, passed: bool) -> Result<Self, ExperimentError> {
        if name.is_empty() {
            return Err(ExperimentError::EmptyField { field: "check name" });
        }
        if name.len() > EVIDENCE_NAME_CHARS_MAX {
            return Err(ExperimentError::TextTooLong {
                field: "check name",
                max: EVIDENCE_NAME_CHARS_MAX,
                got: name.len(),
            });
        }
        if argv.is_empty() {
            return Err(ExperimentError::EmptyField { field: "check argv" });
        }
        if argv.len() > CHECK_ARGV_MAX {
            return Err(ExperimentError::TooManyItems {
                field: "check argv",
                max: CHECK_ARGV_MAX,
            });
        }
        for entry in &argv {
            if entry.len() > ARG_ENTRY_CHARS_MAX {
                return Err(ExperimentError::TextTooLong {
                    field: "check argv entry",
                    max: ARG_ENTRY_CHARS_MAX,
                    got: entry.len(),
                });
            }
        }
        Ok(Self {
            name: name.to_string(),
            argv,
            passed,
        })
    }

    /// The check name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The exact argv the host executed.
    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    /// Whether the host recorded a pass.
    pub fn passed(&self) -> bool {
        self.passed
    }
}

/// A content-addressed artifact produced during evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactDigest {
    name: String,
    digest: String,
}

impl ArtifactDigest {
    /// Records an artifact digest. Rejected: empty or over-long fields.
    pub fn new(name: &str, digest: &str) -> Result<Self, ExperimentError> {
        if name.is_empty() {
            return Err(ExperimentError::EmptyField {
                field: "artifact name",
            });
        }
        if name.len() > EVIDENCE_NAME_CHARS_MAX {
            return Err(ExperimentError::TextTooLong {
                field: "artifact name",
                max: EVIDENCE_NAME_CHARS_MAX,
                got: name.len(),
            });
        }
        if digest.is_empty() {
            return Err(ExperimentError::EmptyField {
                field: "artifact digest",
            });
        }
        if digest.len() > EVIDENCE_NAME_CHARS_MAX {
            return Err(ExperimentError::TextTooLong {
                field: "artifact digest",
                max: EVIDENCE_NAME_CHARS_MAX,
                got: digest.len(),
            });
        }
        Ok(Self {
            name: name.to_string(),
            digest: digest.to_string(),
        })
    }

    /// The artifact name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The artifact digest.
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// The host's verification outcome: a boolean plus what it covered.
///
/// `verified` is set only by host code holding complete evidence; there is
/// no public setter that model-controlled code can flip directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationOutcome {
    verified: bool,
    coverage: Vec<String>,
}

impl VerificationOutcome {
    /// Builds an outcome. Rejected: more than [`COVERAGE_MAX`] entries or
    /// an over-long entry.
    pub fn new(verified: bool, coverage: Vec<String>) -> Result<Self, ExperimentError> {
        if coverage.len() > COVERAGE_MAX {
            return Err(ExperimentError::TooManyItems {
                field: "verification coverage",
                max: COVERAGE_MAX,
            });
        }
        for entry in &coverage {
            if entry.len() > EVIDENCE_NAME_CHARS_MAX {
                return Err(ExperimentError::TextTooLong {
                    field: "coverage entry",
                    max: EVIDENCE_NAME_CHARS_MAX,
                    got: entry.len(),
                });
            }
        }
        Ok(Self { verified, coverage })
    }

    /// Whether the host verified the result.
    pub fn verified(&self) -> bool {
        self.verified
    }

    /// What the verification covered.
    pub fn coverage(&self) -> &[String] {
        &self.coverage
    }
}

/// The complete evidence bundle for one evaluation.
///
/// Completeness ([`EvidenceBundle::is_complete`]) requires: at least one
/// check, every check passed with a recorded non-empty argv, at least one
/// artifact digest, and a verified outcome with non-empty coverage. Anything
/// less fails closed at every gate that consumes evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceBundle {
    checks: Vec<CheckRun>,
    artifacts: Vec<ArtifactDigest>,
    verification: VerificationOutcome,
}

impl EvidenceBundle {
    /// Builds a bundle. Rejected: more than [`CHECKS_MAX`] checks or
    /// [`ARTIFACTS_MAX`] artifacts.
    pub fn new(
        checks: Vec<CheckRun>,
        artifacts: Vec<ArtifactDigest>,
        verification: VerificationOutcome,
    ) -> Result<Self, ExperimentError> {
        if checks.len() > CHECKS_MAX {
            return Err(ExperimentError::TooManyItems {
                field: "evidence checks",
                max: CHECKS_MAX,
            });
        }
        if artifacts.len() > ARTIFACTS_MAX {
            return Err(ExperimentError::TooManyItems {
                field: "evidence artifacts",
                max: ARTIFACTS_MAX,
            });
        }
        Ok(Self {
            checks,
            artifacts,
            verification,
        })
    }

    /// True only when the bundle is complete: every check passed with
    /// recorded argv, artifacts are present, and the host verified with
    /// stated coverage. Missing evidence is never "verified".
    pub fn is_complete(&self) -> bool {
        !self.checks.is_empty()
            && self.checks.iter().all(|c| c.passed() && !c.argv().is_empty())
            && !self.artifacts.is_empty()
            && self.verification.verified()
            && !self.verification.coverage().is_empty()
    }

    /// The recorded check runs.
    pub fn checks(&self) -> &[CheckRun] {
        &self.checks
    }

    /// The recorded artifact digests.
    pub fn artifacts(&self) -> &[ArtifactDigest] {
        &self.artifacts
    }

    /// The host's verification outcome.
    pub fn verification(&self) -> &VerificationOutcome {
        &self.verification
    }
}

// ---------------------------------------------------------------------------
// Budget tracker
// ---------------------------------------------------------------------------

/// Tracks one evaluation's remaining budget against an absolute deadline.
///
/// The clock is injected via [`BudgetTracker::set_now_ms`] so tests are
/// deterministic; production wiring would feed a monotonic host clock.
/// Arithmetic is checked — budgets never wrap, they fail closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetTracker {
    tool_calls_remaining: u64,
    output_bytes_max: u64,
    output_bytes_used: u64,
    deadline_ms: u64,
    now_ms: u64,
}

impl BudgetTracker {
    /// Builds a tracker. Rejected: any zero budget or zero deadline.
    pub fn new(
        tool_calls_max: u64,
        output_bytes_max: u64,
        deadline_ms: u64,
    ) -> Result<Self, ExperimentError> {
        if tool_calls_max == 0 {
            return Err(ExperimentError::InvalidBudget {
                field: "tool_calls_max",
            });
        }
        if output_bytes_max == 0 {
            return Err(ExperimentError::InvalidBudget {
                field: "output_bytes_max",
            });
        }
        if deadline_ms == 0 {
            return Err(ExperimentError::InvalidBudget {
                field: "deadline_ms",
            });
        }
        Ok(Self {
            tool_calls_remaining: tool_calls_max,
            output_bytes_max,
            output_bytes_used: 0,
            deadline_ms,
            now_ms: 0,
        })
    }

    /// Injects the current time in milliseconds (host monotonic clock in
    /// production; a test value in tests).
    pub fn set_now_ms(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
    }

    /// Remaining tool calls.
    pub fn tool_calls_remaining(&self) -> u64 {
        self.tool_calls_remaining
    }

    /// Output bytes consumed so far.
    pub fn output_bytes_used(&self) -> u64 {
        self.output_bytes_used
    }

    /// Consumes `tool_calls` calls and `output_bytes` bytes.
    ///
    /// Fails closed: a passed deadline yields
    /// [`ExperimentError::DeadlineExceeded`]; over-consumption yields
    /// [`ExperimentError::BudgetExhausted`]. Checked addition means usage
    /// can never wrap around to look small.
    pub fn consume(
        &mut self,
        tool_calls: u64,
        output_bytes: u64,
    ) -> Result<(), ExperimentError> {
        if self.now_ms > self.deadline_ms {
            return Err(ExperimentError::DeadlineExceeded);
        }
        if tool_calls > self.tool_calls_remaining {
            return Err(ExperimentError::BudgetExhausted {
                what: "tool calls",
            });
        }
        let used = self
            .output_bytes_used
            .checked_add(output_bytes)
            .ok_or(ExperimentError::BudgetExhausted {
                what: "output bytes",
            })?;
        if used > self.output_bytes_max {
            return Err(ExperimentError::BudgetExhausted {
                what: "output bytes",
            });
        }
        self.tool_calls_remaining -= tool_calls;
        self.output_bytes_used = used;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Evaluator skeleton
// ---------------------------------------------------------------------------

/// Maximum stage transitions one evaluator may record (six stages).
pub const STAGE_TRANSITIONS_MAX: u32 = 6;

/// The evaluator skeleton: enforces stage order and budget discipline.
///
/// Each stage method checks that it runs in order; out-of-order calls are
/// [`ExperimentError::BadStageOrder`]. [`Evaluator::verify`] is the only
/// method that can report success, and only for complete evidence with a
/// live budget — it returns `Ok(false)` for incomplete evidence and an
/// error for a passed deadline or an exhausted tool-call budget.
#[derive(Debug)]
pub struct Evaluator {
    stage: EvalStage,
    budget: BudgetTracker,
    transitions: u32,
    promoted: bool,
}

impl Evaluator {
    /// Builds an evaluator at [`EvalStage::Validate`] with `budget`.
    pub fn new(budget: BudgetTracker) -> Self {
        Self {
            stage: EvalStage::Validate,
            budget,
            transitions: 0,
            promoted: false,
        }
    }

    /// The current stage.
    pub fn stage(&self) -> EvalStage {
        self.stage
    }

    /// The budget tracker (read-only view for audit).
    pub fn budget(&self) -> &BudgetTracker {
        &self.budget
    }

    /// Advances one stage after checking order and the transition bound.
    fn advance(&mut self, expected: EvalStage) -> Result<(), ExperimentError> {
        if self.stage != expected {
            return Err(ExperimentError::BadStageOrder {
                expected: expected.name(),
                got: self.stage.name(),
            });
        }
        let next = self.stage.next().ok_or(ExperimentError::BadStageOrder {
            expected: "terminal",
            got: self.stage.name(),
        })?;
        if self.transitions >= STAGE_TRANSITIONS_MAX {
            return Err(ExperimentError::BudgetExhausted {
                what: "stage transitions",
            });
        }
        self.transitions += 1;
        self.stage = next;
        Ok(())
    }

    /// Validates inputs, contracts, and budgets. Skeleton: enforces order.
    pub fn validate(&mut self) -> Result<(), ExperimentError> {
        self.advance(EvalStage::Validate)
    }

    /// Prepares the isolated workspace and snapshots. Skeleton: order only.
    pub fn prepare(&mut self) -> Result<(), ExperimentError> {
        self.advance(EvalStage::Prepare)
    }

    /// Executes the bounded task, consuming budget. Skeleton: consumes the
    /// declared cost, then enforces order.
    pub fn execute(
        &mut self,
        tool_calls: u64,
        output_bytes: u64,
    ) -> Result<(), ExperimentError> {
        if self.stage != EvalStage::Execute {
            return Err(ExperimentError::BadStageOrder {
                expected: EvalStage::Execute.name(),
                got: self.stage.name(),
            });
        }
        self.budget.consume(tool_calls, output_bytes)?;
        self.advance(EvalStage::Execute)
    }

    /// Verifies the result against host-collected evidence.
    ///
    /// Never succeeds by default: incomplete evidence returns `Ok(false)`;
    /// a passed deadline or an exhausted tool-call budget returns an error.
    /// Only complete evidence with a live budget returns `Ok(true)` — and
    /// even then the promotion gate re-checks completeness independently.
    pub fn verify(&mut self, evidence: &EvidenceBundle) -> Result<bool, ExperimentError> {
        if self.stage != EvalStage::Verify {
            return Err(ExperimentError::BadStageOrder {
                expected: EvalStage::Verify.name(),
                got: self.stage.name(),
            });
        }
        // Fail closed on time and budget before looking at evidence.
        if self.budget.now_ms > self.budget.deadline_ms {
            return Err(ExperimentError::DeadlineExceeded);
        }
        if self.budget.tool_calls_remaining == 0 {
            // Verification itself costs a tool call; an exhausted budget
            // cannot verify.
            return Err(ExperimentError::BudgetExhausted {
                what: "tool calls",
            });
        }
        if !evidence.is_complete() {
            return Ok(false);
        }
        self.advance(EvalStage::Verify)?;
        Ok(true)
    }

    /// Independent review of the result and evidence. Skeleton: order only.
    pub fn review(&mut self) -> Result<(), ExperimentError> {
        self.advance(EvalStage::Review)
    }

    /// Hands the verified result to the promotion gate. Terminal: a second
    /// call is [`ExperimentError::BadStageOrder`].
    pub fn promote(&mut self) -> Result<(), ExperimentError> {
        if self.promoted {
            return Err(ExperimentError::BadStageOrder {
                expected: "terminal",
                got: self.stage.name(),
            });
        }
        if self.stage != EvalStage::Promote {
            return Err(ExperimentError::BadStageOrder {
                expected: EvalStage::Promote.name(),
                got: self.stage.name(),
            });
        }
        self.promoted = true;
        Ok(())
    }
}
