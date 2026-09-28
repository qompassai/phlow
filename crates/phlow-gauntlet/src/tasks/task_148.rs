//! Task 148 — triager feedback ingestion (rust, V/A).
//!
//! The seam is `NeedsMoreInfo` handling: "needs more info" returns the
//! finding to *validation*, never to recon — the new evidence re-enters
//! the pipeline, and the cycle does not restart. A driver-local
//! ingester applies scripted [`FakePlatform`] verdicts to a finding
//! registry: `Triage → NeedsMoreInfo` routes to the validation queue
//! (the recon queue must stay empty); the operator attaches new
//! evidence and the full [`ValidationPipeline`] runs, passing back to
//! `Reportable`; feedback for a terminal finding is rejected; feedback
//! for an unknown finding id is a typed [`FeedbackError::UnknownFinding`]
//! and nothing is created implicitly. All doubles are scripted and
//! labeled MOCK.

use crate::bounty::feed::{snapshot, target};
use crate::bounty::types::IllegalTransition;
use crate::bounty::validate::CheckCtx;
use crate::bounty::{
    Evidence, FakePlatform, Finding, FindingState, FindingStore, TargetId, TargetKind, TriageEvent,
    TriageKind, ValidationPipeline,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use std::collections::HashMap;

/// Task id.
pub const ID: &str = "task-148";
/// Task name.
pub const NAME: &str = "triager feedback ingestion";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "needs_more_info_routes_to_validation",
    "new_evidence_revalidates_to_reportable",
    "terminal_finding_rejects_feedback",
    "unknown_finding_id_typed_error",
];

/// Fixed scripted time (MOCK clock).
const NOW: u64 = 1_700_000_000;
/// Fingerprint of the fixture finding.
const FINGERPRINT: &str = "fp-wave26-148";

/// Typed refusal for feedback ingestion.
#[derive(Debug, PartialEq, Eq)]
enum FeedbackError {
    /// The event names a finding the registry never saw. Nothing is
    /// created implicitly.
    UnknownFinding { id: String },
    /// The feedback asked for a transition the machine forbids (e.g.
    /// out of a terminal state).
    Illegal(IllegalTransition),
}

/// Routes triager feedback. Owns the validation queue (where
/// `NeedsMoreInfo` findings go) and the recon queue (which must stay
/// empty — feedback never restarts recon).
#[derive(Default)]
struct FeedbackIngester {
    /// Finding ids awaiting re-validation.
    validation_queue: Vec<String>,
    /// Finding ids sent back to recon. Must stay empty.
    recon_queue: Vec<String>,
    /// What the ingester did, in order.
    event_log: Vec<String>,
}

impl FeedbackIngester {
    fn new() -> Self {
        FeedbackIngester {
            validation_queue: Vec::new(),
            recon_queue: Vec::new(),
            event_log: Vec::new(),
        }
    }

    /// Ingest one `NeedsMoreInfo` verdict for `finding_id`.
    fn ingest(
        &mut self,
        registry: &mut HashMap<String, Finding>,
        finding_id: &str,
    ) -> Result<(), FeedbackError> {
        let f = registry
            .get_mut(finding_id)
            .ok_or_else(|| FeedbackError::UnknownFinding {
                id: finding_id.to_string(),
            })?;
        let next = f
            .state
            .transition(FindingState::NeedsMoreInfo)
            .map_err(FeedbackError::Illegal)?;
        f.state = next;
        // Back to validation — never to recon.
        self.validation_queue.push(finding_id.to_string());
        self.event_log.push(format!(
            "{finding_id}: Triage -> NeedsMoreInfo; queued for validation (not recon)"
        ));
        Ok(())
    }
}

/// A finding sitting in `Triage`, with non-empty evidence.
fn triage_finding(id: &str) -> Finding {
    let raw = b"poc-bytes-fixture".to_vec();
    Finding {
        id: id.to_string(),
        target_id: TargetId("t-1".to_string()),
        fingerprint: FINGERPRINT.to_string(),
        title: format!("finding {id}"),
        state: FindingState::Triage,
        evidence: Evidence {
            sha256: "fixture-sha256".to_string(),
            raw,
            custody: Vec::new(),
            truncated: false,
        },
        observation_count: 1,
        reject_reason: None,
    }
}

/// Registry holding one finding, plus the store the pipeline checks
/// against. The store holds the same record (same id), so the
/// non-duplicate check passes on the re-validation path.
fn registry_with(
    state: FindingState,
) -> Result<(HashMap<String, Finding>, FindingStore, String), TaskDriverError> {
    let mut store = FindingStore::new();
    let mut f = triage_finding("placeholder");
    f.state = state;
    let (id, _) = store.insert(f);
    let finding = match store.findings_for(FINGERPRINT) {
        [only] => only.clone(),
        _ => {
            return Err(TaskDriverError::Fixture {
                what: "finding".to_string(),
                detail: "task-148: fixture finding missing from store".to_string(),
            });
        }
    };
    let mut registry = HashMap::new();
    registry.insert(id.clone(), finding);
    Ok((registry, store, id))
}

fn needs_more_info_event(finding_id: &str) -> TriageEvent {
    TriageEvent {
        finding_id: finding_id.to_string(),
        kind: TriageKind::NeedsMoreInfo,
        at: NOW,
    }
}

/// V1: a `NeedsMoreInfo` event moves the finding `Triage →
/// NeedsMoreInfo` and queues it for validation — never for recon.
fn case_needs_more_info_routes_to_validation() -> Result<CaseReport, TaskDriverError> {
    let (mut registry, _store, id) = registry_with(FindingState::Triage)?;
    let mut platform = FakePlatform::new();
    platform.queue_event(needs_more_info_event(&id));
    let mut ingester = FeedbackIngester::new();
    let mut failures = Vec::new();
    for e in platform.poll_events() {
        if e.kind != TriageKind::NeedsMoreInfo {
            failures.push("scripted the wrong verdict".to_string());
            continue;
        }
        if let Err(e) = ingester.ingest(&mut registry, &e.finding_id) {
            failures.push(format!("ingest failed: {e:?}"));
        }
    }
    let state = registry
        .get(&id)
        .map(|f| format!("{:?}", f.state))
        .unwrap_or_else(|| "<missing>".to_string());
    if state != "NeedsMoreInfo" {
        failures.push(format!("state {state}, want NeedsMoreInfo"));
    }
    if ingester.validation_queue != vec![id.clone()] {
        failures.push(format!(
            "validation queue wrong: {:?}",
            ingester.validation_queue
        ));
    }
    if !ingester.recon_queue.is_empty() {
        failures.push("RECON QUEUE NON-EMPTY — the cycle restarted!".to_string());
    }
    let mut evidence = ingester.event_log.clone();
    evidence.push(format!("state: {state}"));
    evidence.push(format!("validation_queue: {:?}", ingester.validation_queue));
    evidence.push(format!("recon_queue: {:?}", ingester.recon_queue));
    evidence.push("backend: FakePlatform scripted events (MOCK)".to_string());
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "state": state,
            "validation_queue": ingester.validation_queue,
            "recon_queue": ingester.recon_queue,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: the operator attaches new evidence; the finding re-enters the
/// full validation pipeline (`NeedsMoreInfo → Validated`, all checks
/// pass, `Validated → Reportable`).
fn case_new_evidence_revalidates_to_reportable() -> Result<CaseReport, TaskDriverError> {
    let (mut registry, store, id) = registry_with(FindingState::Triage)?;
    let mut ingester = FeedbackIngester::new();
    let mut failures = Vec::new();
    if let Err(e) = ingester.ingest(&mut registry, &id) {
        failures.push(format!("ingest failed: {e:?}"));
    }
    let finding = registry
        .get_mut(&id)
        .ok_or_else(|| TaskDriverError::Fixture {
            what: "finding".to_string(),
            detail: "task-148: finding vanished from registry".to_string(),
        })?;
    // The operator attaches new evidence (the triager's ask, answered).
    let new_raw = b"poc-bytes-v2-with-triager-requested-detail".to_vec();
    finding.evidence = Evidence {
        sha256: "fixture-sha256-v2".to_string(),
        raw: new_raw,
        custody: Vec::new(),
        truncated: false,
    };
    // NeedsMoreInfo -> Validated: back into the pipeline.
    match finding.state.transition(FindingState::Validated) {
        Ok(next) => finding.state = next,
        Err(e) => failures.push(format!("return to validation refused: {e:?}")),
    }
    // The full pipeline, with the real default checks.
    let pipeline = ValidationPipeline::with_defaults();
    let scope = snapshot(
        7,
        NOW,
        vec![target("t-1", TargetKind::Domain, "app.example.com")],
    );
    let ctx = CheckCtx {
        scope: Some(&scope),
        store: &store,
    };
    match pipeline.validate(finding, &ctx) {
        Ok(()) => match finding.state.transition(FindingState::Reportable) {
            Ok(next) => finding.state = next,
            Err(e) => failures.push(format!("reportable refused: {e:?}")),
        },
        Err((name, result)) => {
            failures.push(format!("re-validation failed at check {name}: {result:?}"))
        }
    }
    let state = format!("{:?}", finding.state);
    if state != "Reportable" {
        failures.push(format!("state {state}, want Reportable"));
    }
    let mut evidence = vec![
        format!("checks run: {:?}", pipeline.check_names()),
        format!("state: {state}"),
    ];
    evidence.extend(ingester.event_log.iter().cloned());
    evidence.push("backend: FakePlatform scripted events (MOCK)".to_string());
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "state": state,
            "checks": pipeline.check_names(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1: `NeedsMoreInfo` for an `Accepted` finding → rejected. Terminal
/// states are terminal; feedback cannot resurrect them.
fn case_terminal_finding_rejects_feedback() -> Result<CaseReport, TaskDriverError> {
    let (mut registry, _store, id) = registry_with(FindingState::Accepted)?;
    let mut ingester = FeedbackIngester::new();
    let mut failures = Vec::new();
    match ingester.ingest(&mut registry, &id) {
        Err(FeedbackError::Illegal(IllegalTransition { from, to }))
            if from == "Accepted" && to == "NeedsMoreInfo" => {}
        Err(e) => failures.push(format!("wrong refusal: {e:?}")),
        Ok(()) => failures.push("ACCEPTED finding left its terminal state!".to_string()),
    }
    let state = registry
        .get(&id)
        .map(|f| format!("{:?}", f.state))
        .unwrap_or_else(|| "<missing>".to_string());
    if state != "Accepted" {
        failures.push(format!("terminal state moved: {state}"));
    }
    if !ingester.validation_queue.is_empty() {
        failures.push("terminal finding was queued".to_string());
    }
    let mut evidence = vec![
        "refusal: Illegal{from: Accepted, to: NeedsMoreInfo}".to_string(),
        format!("state unchanged: {state}"),
        "backend: FakePlatform scripted events (MOCK)".to_string(),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "refusal": "IllegalTransition",
            "state": state,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2: a feedback event for an unknown finding id → typed
/// `UnknownFinding`. Nothing is created implicitly.
fn case_unknown_finding_id_typed_error() -> Result<CaseReport, TaskDriverError> {
    let (mut registry, _store, _id) = registry_with(FindingState::Triage)?;
    let before = registry.len();
    let mut ingester = FeedbackIngester::new();
    let mut failures = Vec::new();
    match ingester.ingest(&mut registry, "f999999") {
        Err(FeedbackError::UnknownFinding { id }) if id == "f999999" => {}
        Err(e) => failures.push(format!("wrong refusal: {e:?}")),
        Ok(()) => failures.push("unknown finding was INGESTED!".to_string()),
    }
    if registry.len() != before || registry.contains_key("f999999") {
        failures.push("registry mutated by unknown-id feedback".to_string());
    }
    if !ingester.validation_queue.is_empty() {
        failures.push("unknown finding was queued".to_string());
    }
    let mut evidence = vec![
        "refusal: FeedbackError::UnknownFinding{id: f999999}".to_string(),
        format!("registry size unchanged: {before}"),
        "backend: FakePlatform scripted events (MOCK)".to_string(),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "refusal": "UnknownFinding",
            "registry_size": registry.len(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "needs_more_info_routes_to_validation" => case_needs_more_info_routes_to_validation(),
        "new_evidence_revalidates_to_reportable" => case_new_evidence_revalidates_to_reportable(),
        "terminal_finding_rejects_feedback" => case_terminal_finding_rejects_feedback(),
        "unknown_finding_id_typed_error" => case_unknown_finding_id_typed_error(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-148: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// route back to validation.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-148".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-148".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
