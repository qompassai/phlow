//! Task 143 — false-positive rejection with reason (rust, V/A).
//!
//! The fate of the pipeline's rejects (task 142 judges; this task
//! buries). A plausible-looking finding that fails validation is
//! discarded *with a recorded reason*, held in a queryable quarantine
//! with its evidence intact — never silently dropped. Rejection is
//! sticky: it is keyed by the composite (fingerprint, content-hash),
//! so re-submitting the same false positive next cycle stays rejected
//! instead of resurrecting it. Genuinely different content under the
//! same fingerprint is a new finding (the task-139 collision tiebreak),
//! not a resurrection.
//!
//! The rejection reasons are the pipeline's own check names and fail
//! reasons, recorded verbatim. All verdict evidence comes from scripted
//! fixtures (MOCK).

use crate::bounty::validate::CheckCtx;
use crate::bounty::*;
use crate::skillopt::driver::{CaseReport, TaskDriverError, verdict_line};
use crate::skillopt::learner::Verdict;
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-143";
/// Task name.
pub const NAME: &str = "false-positive-rejection";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "rejection_records_reason",
    "quarantine_queryable",
    "empty_evidence_rejected",
    "rejection_sticky_across_cycles",
];

/// Reproducibility check, local to this driver (same rule as the
/// task-142 driver; drivers stay disjoint, so the small check is
/// duplicated rather than shared).
pub struct ReproducibleCheck;

impl Check for ReproducibleCheck {
    fn name(&self) -> &'static str {
        "reproducible"
    }

    fn check(&self, finding: &Finding, _ctx: &CheckCtx) -> CheckResult {
        if finding.observation_count >= 2 {
            CheckResult::Pass
        } else {
            CheckResult::Fail {
                reason: "observed once: not reproduced".to_string(),
            }
        }
    }
}

fn validation_pipeline() -> ValidationPipeline {
    let mut pipeline = ValidationPipeline::with_defaults();
    pipeline.add(ReproducibleCheck);
    pipeline
}

/// A quarantined finding: rejected, kept auditable, evidence intact.
#[derive(Clone, Debug)]
pub struct QuarantinedFinding {
    pub finding_id: String,
    pub fingerprint: String,
    pub reason: String,
    pub evidence: Evidence,
}

/// The quarantine list: append-only, queryable by fingerprint — the
/// operator's audit surface for everything the pipeline rejected.
#[derive(Debug, Default)]
pub struct Quarantine {
    items: Vec<QuarantinedFinding>,
}

impl Quarantine {
    pub fn new() -> Self {
        Quarantine { items: Vec::new() }
    }

    pub fn hold(&mut self, finding: &Finding, reason: &str) {
        self.items.push(QuarantinedFinding {
            finding_id: finding.id.clone(),
            fingerprint: finding.fingerprint.clone(),
            reason: reason.to_string(),
            evidence: finding.evidence.clone(),
        });
    }

    pub fn get(&self, fingerprint: &str) -> Option<&QuarantinedFinding> {
        self.items.iter().find(|q| q.fingerprint == fingerprint)
    }

    /// The full audit listing, in rejection order.
    pub fn list(&self) -> &[QuarantinedFinding] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

fn scope_v7() -> ScopeSnapshot {
    ScopeSnapshot {
        version: 7,
        targets: vec![Target {
            id: TargetId("t-web-01".to_string()),
            kind: TargetKind::Domain,
            value: "example.com".to_string(),
        }],
        fetched_at: 1_700_000_000,
    }
}

fn mock_evidence() -> Evidence {
    let raw = b"MOCK: nuclei template xss-detect matched".to_vec();
    Evidence {
        sha256: approve::sha256_hex(&raw),
        raw,
        custody: Vec::new(),
        truncated: false,
    }
}

fn passing_finding(fingerprint: &str) -> Finding {
    Finding {
        id: "pending".to_string(),
        target_id: TargetId("t-web-01".to_string()),
        fingerprint: fingerprint.to_string(),
        title: "mock: reflected XSS in /search".to_string(),
        state: FindingState::Candidate,
        evidence: mock_evidence(),
        observation_count: 2,
        reject_reason: None,
    }
}

/// Reject a finding through the store's state machine and hold it in
/// quarantine. The reason is recorded verbatim on both the record and
/// the quarantine entry. Returns the stored record id.
fn reject_and_quarantine(
    store: &mut FindingStore,
    quarantine: &mut Quarantine,
    mut finding: Finding,
    reason: &str,
) -> Result<String, TaskDriverError> {
    let fp = finding.fingerprint.clone();
    finding.reject_reason = Some(reason.to_string());
    let (id, is_new) = store.insert(finding);
    if !is_new {
        return Err(TaskDriverError::Fixture {
            what: "rejection".to_string(),
            detail: "task-143: reject_and_quarantine called on a known finding".to_string(),
        });
    }
    store
        .transition(&id, FindingState::Rejected)
        .map_err(|e| TaskDriverError::Fixture {
            what: "rejection".to_string(),
            detail: match e {
                StoreTransitionError::UnknownId { id } => {
                    format!("task-143: transition of unknown record id {id}")
                }
                StoreTransitionError::Illegal(t) => format!(
                    "task-143: Candidate -> Rejected refused: {} -> {}",
                    t.from, t.to
                ),
            },
        })?;
    let record = match store.findings_for(&fp) {
        [only] => only,
        _ => {
            return Err(TaskDriverError::Fixture {
                what: "rejection".to_string(),
                detail: "task-143: record vanished after insert".to_string(),
            });
        }
    };
    quarantine.hold(record, reason);
    Ok(id)
}

/// V1: a finding that fails the reproducibility check is rejected with
/// reason "not-reproducible" — recorded verbatim on the record and in
/// quarantine, evidence intact, state Rejected.
fn case_rejection_records_reason() -> Result<CaseReport, TaskDriverError> {
    let mut store = FindingStore::new();
    let mut quarantine = Quarantine::new();
    let scope = scope_v7();
    let pipeline = validation_pipeline();
    let ctx = CheckCtx {
        scope: Some(&scope),
        store: &store,
    };
    let original_raw = mock_evidence().raw.clone();
    let mut fp_finding = passing_finding("fp-143-v1");
    fp_finding.observation_count = 1;
    let mut failures = Vec::new();
    match pipeline.validate(&fp_finding, &ctx) {
        Err((name, CheckResult::Fail { .. })) if name == "reproducible" => {}
        other => failures.push(format!("expected reproducible Fail, got {other:?}")),
    }
    let id = reject_and_quarantine(&mut store, &mut quarantine, fp_finding, "not-reproducible")?;
    let record = match store.findings_for("fp-143-v1") {
        [only] => only,
        _ => {
            return Err(TaskDriverError::Fixture {
                what: "rejection".to_string(),
                detail: "task-143: rejected record not found".to_string(),
            });
        }
    };
    if record.state != FindingState::Rejected {
        failures.push(format!("record state {:?}, want Rejected", record.state));
    }
    if record.reject_reason.as_deref() != Some("not-reproducible") {
        failures.push(format!("reason not verbatim: {:?}", record.reject_reason));
    }
    match quarantine.get("fp-143-v1") {
        Some(q) => {
            if q.reason != "not-reproducible" {
                failures.push(format!("quarantine reason mangled: {}", q.reason));
            }
            if q.evidence.raw != original_raw {
                failures.push("quarantine evidence differs from the finding's".to_string());
            }
        }
        None => failures.push("rejected finding missing from quarantine".to_string()),
    }
    let mut evidence_lines = vec![
        format!("rejected record: {id} state=Rejected reason=not-reproducible"),
        "quarantine holds fp-143-v1 with evidence intact".to_string(),
        verdict_line(
            "143",
            Verdict::Replicates,
            "reason verbatim, evidence intact, state Rejected",
        ),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "record_id": id,
            "state": format!("{:?}", record.state),
            "reason": record.reject_reason,
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: the quarantine is queryable — the operator can audit everything
/// rejected, with reasons, in rejection order.
fn case_quarantine_queryable() -> Result<CaseReport, TaskDriverError> {
    let mut store = FindingStore::new();
    let mut quarantine = Quarantine::new();
    let mut failures = Vec::new();
    let mut fp1 = passing_finding("fp-143-q1");
    fp1.observation_count = 1;
    reject_and_quarantine(&mut store, &mut quarantine, fp1, "not-reproducible")?;
    let mut fp2 = passing_finding("fp-143-q2");
    fp2.evidence = Evidence {
        sha256: approve::sha256_hex(&[]),
        raw: Vec::new(),
        custody: Vec::new(),
        truncated: false,
    };
    reject_and_quarantine(&mut store, &mut quarantine, fp2, "evidence-empty")?;
    if quarantine.len() != 2 {
        failures.push(format!("quarantine holds {}, want 2", quarantine.len()));
    }
    let listed: Vec<(&str, &str)> = quarantine
        .list()
        .iter()
        .map(|q| (q.fingerprint.as_str(), q.reason.as_str()))
        .collect();
    if listed
        != [
            ("fp-143-q1", "not-reproducible"),
            ("fp-143-q2", "evidence-empty"),
        ]
    {
        failures.push(format!("quarantine listing wrong: {listed:?}"));
    }
    if quarantine.get("fp-143-nope").is_some() {
        failures.push("quarantine.get returned a phantom entry".to_string());
    }
    let mut evidence_lines: Vec<String> = listed
        .iter()
        .map(|(fp, reason)| format!("quarantined: {fp} reason={reason}"))
        .collect();
    evidence_lines.push(verdict_line(
        "143",
        Verdict::Replicates,
        "quarantine lists every rejection with its verbatim reason",
    ));
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "quarantined": quarantine.len(),
            "reasons": listed.iter().map(|(_, r)| r).collect::<Vec<_>>(),
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1: a false positive identical to a true positive except the
/// `evidence` field is empty is rejected on exactly
/// `evidence-present` — the attribution is precise, not "some check".
fn case_empty_evidence_rejected() -> Result<CaseReport, TaskDriverError> {
    let store = FindingStore::new();
    let scope = scope_v7();
    let pipeline = validation_pipeline();
    let ctx = CheckCtx {
        scope: Some(&scope),
        store: &store,
    };
    let mut failures = Vec::new();
    let tp = passing_finding("fp-143-tp");
    if pipeline.validate(&tp, &ctx).is_err() {
        failures.push("true positive failed validation".to_string());
    }
    let mut fp = passing_finding("fp-143-fp");
    fp.evidence = Evidence {
        sha256: approve::sha256_hex(&[]),
        raw: Vec::new(),
        custody: Vec::new(),
        truncated: false,
    };
    match pipeline.validate(&fp, &ctx) {
        Err((name, CheckResult::Fail { .. })) => {
            if name != "evidence-present" {
                failures.push(format!("FP rejected on {name}, want evidence-present"));
            } else if fp.title != tp.title || fp.target_id != tp.target_id {
                // TP and FP differ only in the evidence field — the
                // rejection is attributable to exactly that dimension.
                failures.push("fixtures differ in more than evidence".to_string());
            }
        }
        other => failures.push(format!("FP not rejected on evidence-present: {other:?}")),
    }
    let mut evidence_lines = vec![
        "TP (full evidence) -> Ok; FP (empty evidence) -> evidence-present Fail".to_string(),
        verdict_line(
            "143",
            Verdict::Replicates,
            "rejection attributed to exactly the failed dimension",
        ),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "tp_passed": failures.is_empty(),
            "fp_check": "evidence-present",
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2: re-submit the same rejected false positive next cycle. The
/// rejection is keyed by the composite (fingerprint, content-hash) and
/// stays sticky: no new record, state stays Rejected, reason intact.
/// (A retitled re-observation is different content and would correctly
/// open a new record — the task-139 collision tiebreak, not a
/// resurrection — so the re-submission here is content-identical.)
fn case_rejection_sticky_across_cycles() -> Result<CaseReport, TaskDriverError> {
    let mut store = FindingStore::new();
    let mut quarantine = Quarantine::new();
    let mut failures = Vec::new();
    let mut fp = passing_finding("fp-143-sticky");
    fp.observation_count = 1;
    let id = reject_and_quarantine(&mut store, &mut quarantine, fp, "not-reproducible")?;
    let records_before = store.record_count();
    // Next cycle: the same false positive is observed again.
    let resub = passing_finding("fp-143-sticky");
    let (id2, is_new) = store.insert(resub);
    if is_new {
        failures.push("rejected fingerprint created a second record".to_string());
    }
    if id2 != id {
        failures.push(format!("re-insert returned {id2}, want the original {id}"));
    }
    if store.record_count() != records_before {
        failures.push("record count grew on re-submit".to_string());
    }
    let record = match store.findings_for("fp-143-sticky") {
        [only] => only,
        _ => {
            return Err(TaskDriverError::Fixture {
                what: "stickiness".to_string(),
                detail: "task-143: record vanished".to_string(),
            });
        }
    };
    if record.state != FindingState::Rejected {
        failures.push(format!(
            "rejected finding resurrected to {:?}",
            record.state
        ));
    }
    if record.reject_reason.as_deref() != Some("not-reproducible") {
        failures.push("rejection reason lost across the cycle".to_string());
    }
    if record.observation_count < 2 {
        failures.push("re-observation did not bump observation_count".to_string());
    }
    let mut evidence_lines = vec![
        format!("cycle 1: rejected {id} (not-reproducible)"),
        format!(
            "cycle 2: re-insert -> id {id2}, is_new={is_new}, state={:?}",
            record.state
        ),
        verdict_line(
            "143",
            Verdict::Replicates,
            "rejection sticky by (fingerprint, content-hash) across cycles",
        ),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "record_id": id,
            "resubmitted_id": id2,
            "is_new": is_new,
            "state": format!("{:?}", record.state),
            "reason": record.reject_reason,
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "rejection_records_reason" => case_rejection_records_reason(),
        "quarantine_queryable" => case_quarantine_queryable(),
        "empty_evidence_rejected" => case_empty_evidence_rejected(),
        "rejection_sticky_across_cycles" => case_rejection_sticky_across_cycles(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-143: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[1]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-143".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-143".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
