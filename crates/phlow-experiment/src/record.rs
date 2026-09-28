//! The immutable evaluation record: one JSON report per experiment.
//!
//! The schema mirrors the plan's evaluation-record contract exactly:
//! `schema_version`, `experiment_id`, `status`, `baseline_revision`,
//! `candidate_revision`, digests, model/toolchain/limit lists, the task
//! graph, events, changed files, checks, security/performance/review
//! results, a `verification` section, a `promotion` section, and a
//! `stop_reason`.
//!
//! `verified`, `eligible`, and `human_approved` are separate states that
//! start false. There is no public setter that model-controlled code can
//! flip directly: `verified` requires a complete [`EvidenceBundle`],
//! promotion eligibility requires complete evidence too, and
//! `human_approved` requires a [`HumanApproval`] token — whose
//! unforgeability is the gate. Fields stay private; mutation goes through
//! explicit bounded methods.

use crate::error::ExperimentError;
use crate::evaluator::EvidenceBundle;
use crate::promotion::HumanApproval;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Bounds (all with units)
// ---------------------------------------------------------------------------

/// The record schema version this crate writes.
pub const SCHEMA_VERSION: u32 = 1;
/// Maximum events retained in one record.
pub const EVENTS_MAX: usize = 1_024;
/// Maximum changed files listed in one record.
pub const CHANGED_FILES_MAX: usize = 256;
/// Maximum check records in one record.
pub const RECORD_CHECKS_MAX: usize = 256;
/// Maximum entries in any record string list.
pub const RECORD_LIST_MAX: usize = 256;
/// Maximum characters in one record text field.
pub const RECORD_TEXT_CHARS_MAX: usize = 4_096;

// ---------------------------------------------------------------------------
// Record sections
// ---------------------------------------------------------------------------

/// One check entry in the record: name, exact argv, outcome, required flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckRecord {
    name: String,
    argv: Vec<String>,
    passed: bool,
    required: bool,
}

impl CheckRecord {
    /// Builds a check record. Rejected: empty or over-long name, empty argv.
    pub fn new(
        name: &str,
        argv: Vec<String>,
        passed: bool,
        required: bool,
    ) -> Result<Self, ExperimentError> {
        if name.is_empty() {
            return Err(ExperimentError::EmptyField {
                field: "check name",
            });
        }
        if name.len() > RECORD_TEXT_CHARS_MAX {
            return Err(ExperimentError::TextTooLong {
                field: "check name",
                max: RECORD_TEXT_CHARS_MAX,
                got: name.len(),
            });
        }
        if argv.is_empty() {
            return Err(ExperimentError::EmptyField {
                field: "check argv",
            });
        }
        Ok(Self {
            name: name.to_string(),
            argv,
            passed,
            required,
        })
    }

    /// Whether the check passed.
    pub fn passed(&self) -> bool {
        self.passed
    }
}

/// The verification section: a boolean plus its coverage.
///
/// Mirrors the plan's `"verification": {"verified": false, "coverage": {}}`
/// shape, with coverage as an explicit list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerificationSection {
    verified: bool,
    coverage: Vec<String>,
}

/// The promotion section: eligibility and human approval as separate
/// states, both starting false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromotionSection {
    eligible: bool,
    human_approved: bool,
}

// ---------------------------------------------------------------------------
// The record
// ---------------------------------------------------------------------------

/// Constructor parameters for [`EvaluationRecord`].
#[derive(Debug, Clone)]
pub struct RecordParams {
    /// Content-addressed experiment id.
    pub experiment_id: String,
    /// Pinned baseline revision.
    pub baseline_revision: String,
    /// Digest of the workspace the experiment ran in.
    pub workspace_digest: String,
    /// Digest of the operator configuration in force.
    pub operator_config_digest: String,
    /// Model ids involved.
    pub model_ids: Vec<String>,
    /// Toolchain versions recorded.
    pub toolchain_versions: Vec<String>,
    /// Limit descriptions in force.
    pub limits: Vec<String>,
    /// Why the experiment stopped.
    pub stop_reason: String,
}

/// One immutable evaluation report.
///
/// Constructed with `verified = false`, `eligible = false`, and
/// `human_approved = false`; only explicit, evidence-backed methods move
/// those states. Serializes to the plan's JSON shape via
/// [`EvaluationRecord::to_json`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvaluationRecord {
    schema_version: u32,
    experiment_id: String,
    status: String,
    baseline_revision: String,
    candidate_revision: Option<String>,
    workspace_digest: String,
    operator_config_digest: String,
    model_ids: Vec<String>,
    toolchain_versions: Vec<String>,
    limits: Vec<String>,
    task_graph: Vec<String>,
    events: Vec<String>,
    changed_files: Vec<String>,
    checks: Vec<CheckRecord>,
    security_results: Vec<String>,
    performance_results: Vec<String>,
    review_results: Vec<String>,
    verification: VerificationSection,
    promotion: PromotionSection,
    stop_reason: String,
}

impl EvaluationRecord {
    /// Starts a record. `status` begins as `"experimental"`; `verified`,
    /// `eligible`, and `human_approved` begin false. Rejected: empty or
    /// over-long text fields, oversized id/toolchain/limit lists.
    pub fn new(params: RecordParams) -> Result<Self, ExperimentError> {
        let experiment_id = check_record_text("experiment_id", &params.experiment_id)?;
        let baseline_revision = check_record_text("baseline_revision", &params.baseline_revision)?;
        let workspace_digest = check_record_text("workspace_digest", &params.workspace_digest)?;
        let operator_config_digest =
            check_record_text("operator_config_digest", &params.operator_config_digest)?;
        let stop_reason = check_record_text("stop_reason", &params.stop_reason)?;
        check_list("model_ids", &params.model_ids)?;
        check_list("toolchain_versions", &params.toolchain_versions)?;
        check_list("limits", &params.limits)?;
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            experiment_id,
            status: "experimental".to_string(),
            baseline_revision,
            candidate_revision: None,
            workspace_digest,
            operator_config_digest,
            model_ids: params.model_ids,
            toolchain_versions: params.toolchain_versions,
            limits: params.limits,
            task_graph: Vec::new(),
            events: Vec::new(),
            changed_files: Vec::new(),
            checks: Vec::new(),
            security_results: Vec::new(),
            performance_results: Vec::new(),
            review_results: Vec::new(),
            verification: VerificationSection {
                verified: false,
                coverage: Vec::new(),
            },
            promotion: PromotionSection {
                eligible: false,
                human_approved: false,
            },
            stop_reason,
        })
    }

    /// The schema version (always [`SCHEMA_VERSION`]).
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Whether the host verified the result.
    pub fn verified(&self) -> bool {
        self.verification.verified
    }

    /// Whether the candidate is eligible for human review.
    pub fn promotion_eligible(&self) -> bool {
        self.promotion.eligible
    }

    /// Whether a human approved promotion.
    pub fn human_approved(&self) -> bool {
        self.promotion.human_approved
    }

    /// Number of events recorded.
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Number of checks recorded.
    pub fn check_count(&self) -> usize {
        self.checks.len()
    }

    /// Records the candidate revision under evaluation, if any.
    pub fn set_candidate_revision(&mut self, revision: &str) -> Result<(), ExperimentError> {
        self.candidate_revision = Some(check_record_text("candidate_revision", revision)?);
        Ok(())
    }

    /// Appends a machine-readable event (bounded at [`EVENTS_MAX`]).
    pub fn record_event(&mut self, event: &str) -> Result<(), ExperimentError> {
        let event = check_record_text("event", event)?;
        if self.events.len() >= EVENTS_MAX {
            return Err(ExperimentError::TooManyItems {
                field: "events",
                max: EVENTS_MAX,
            });
        }
        self.events.push(event);
        Ok(())
    }

    /// Appends a task-graph entry (bounded).
    pub fn add_task_graph_entry(&mut self, entry: &str) -> Result<(), ExperimentError> {
        push_limited(&mut self.task_graph, "task_graph", entry)
    }

    /// Appends a changed file (bounded at [`CHANGED_FILES_MAX`]).
    pub fn add_changed_file(&mut self, path: &str) -> Result<(), ExperimentError> {
        let path = check_record_text("changed file", path)?;
        if self.changed_files.len() >= CHANGED_FILES_MAX {
            return Err(ExperimentError::TooManyItems {
                field: "changed_files",
                max: CHANGED_FILES_MAX,
            });
        }
        self.changed_files.push(path);
        Ok(())
    }

    /// Appends a check record (bounded at [`RECORD_CHECKS_MAX`]).
    pub fn record_check(&mut self, check: CheckRecord) -> Result<(), ExperimentError> {
        if self.checks.len() >= RECORD_CHECKS_MAX {
            return Err(ExperimentError::TooManyItems {
                field: "checks",
                max: RECORD_CHECKS_MAX,
            });
        }
        self.checks.push(check);
        Ok(())
    }

    /// Appends a security result entry (bounded).
    pub fn add_security_result(&mut self, result: &str) -> Result<(), ExperimentError> {
        push_limited(&mut self.security_results, "security_results", result)
    }

    /// Appends a performance result entry (bounded).
    pub fn add_performance_result(&mut self, result: &str) -> Result<(), ExperimentError> {
        push_limited(&mut self.performance_results, "performance_results", result)
    }

    /// Appends a review result entry (bounded).
    pub fn add_review_result(&mut self, result: &str) -> Result<(), ExperimentError> {
        push_limited(&mut self.review_results, "review_results", result)
    }

    /// Marks the result verified — only on complete host evidence.
    ///
    /// There is deliberately no setter that flips `verified` from a bare
    /// boolean: the only path requires `evidence.is_complete()`, so model
    /// output alone can never mark a result verified.
    pub fn mark_verified(&mut self, evidence: &EvidenceBundle) -> Result<(), ExperimentError> {
        if !evidence.is_complete() {
            return Err(ExperimentError::IncompleteEvidence {
                missing: "complete evidence bundle",
            });
        }
        self.verification.verified = true;
        self.verification.coverage = evidence.verification().coverage().to_vec();
        Ok(())
    }

    /// Marks the candidate eligible for human review — only on complete
    /// evidence. Eligibility is not approval; see
    /// [`EvaluationRecord::record_human_approval`].
    pub fn mark_promotion_eligible(
        &mut self,
        evidence: &EvidenceBundle,
    ) -> Result<(), ExperimentError> {
        if !evidence.is_complete() {
            return Err(ExperimentError::IncompleteEvidence {
                missing: "complete evidence bundle",
            });
        }
        self.promotion.eligible = true;
        Ok(())
    }

    /// Records a human approval. The [`HumanApproval`] token is consumed by
    /// reference here (the promotion gate consumes it by value); only a
    /// token produced by [`HumanApproval::from_operator_record`] can call
    /// this, which is what makes the state trustworthy.
    pub fn record_human_approval(&mut self, _approval: &HumanApproval) {
        self.promotion.human_approved = true;
    }

    /// Serializes the record to pretty JSON in the plan's schema.
    pub fn to_json(&self) -> Result<String, ExperimentError> {
        serde_json::to_string_pretty(self).map_err(|error| ExperimentError::SerializationFailed {
            reason: error.to_string(),
        })
    }
}

/// Validates one record text field.
fn check_record_text(field: &'static str, value: &str) -> Result<String, ExperimentError> {
    if value.is_empty() {
        return Err(ExperimentError::EmptyField { field });
    }
    if value.len() > RECORD_TEXT_CHARS_MAX {
        return Err(ExperimentError::TextTooLong {
            field,
            max: RECORD_TEXT_CHARS_MAX,
            got: value.len(),
        });
    }
    Ok(value.to_string())
}

/// Validates one constructor string list.
fn check_list(field: &'static str, values: &[String]) -> Result<(), ExperimentError> {
    if values.len() > RECORD_LIST_MAX {
        return Err(ExperimentError::TooManyItems {
            field,
            max: RECORD_LIST_MAX,
        });
    }
    for value in values {
        check_record_text(field, value)?;
    }
    Ok(())
}

/// Pushes one entry onto a bounded record list.
fn push_limited(
    list: &mut Vec<String>,
    field: &'static str,
    value: &str,
) -> Result<(), ExperimentError> {
    let value = check_record_text(field, value)?;
    if list.len() >= RECORD_LIST_MAX {
        return Err(ExperimentError::TooManyItems {
            field,
            max: RECORD_LIST_MAX,
        });
    }
    list.push(value);
    Ok(())
}
