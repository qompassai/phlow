//! Task 147 — post-submission state tracking (rust, V/A).
//!
//! The seam is the finding state machine under platform events:
//! `Submitted → Triage → Accepted | Duplicate | NeedsMoreInfo | Closed`.
//! A driver-local tracker applies scripted [`FakePlatform`] events
//! through [`FindingState::transition`]: legal sequences are tracked
//! exactly; an unknown platform state is recorded as `Unknown("…")` on
//! the event log with the finding state unchanged (never guessed); an
//! illegal transition quarantines the event with the state untouched.
//! The platform's intake acknowledgment (`Submitted → Triage`) is the
//! tracker's own bookkeeping step — [`TriageKind`] carries verdicts,
//! not an intake ack. All doubles are scripted and labeled MOCK.

use crate::bounty::approve::sha256_hex;
use crate::bounty::types::IllegalTransition;
use crate::bounty::{
    Evidence, FakePlatform, Finding, FindingState, TargetId, TriageEvent, TriageKind,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-147";
/// Task name.
pub const NAME: &str = "post-submission state tracking";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "submitted_triage_accepted",
    "duplicate_links_original",
    "unknown_state_never_mapped",
    "skipped_state_rejected",
];

/// Fixed scripted time (MOCK clock).
const NOW: u64 = 1_700_000_000;

/// A finding that has just been submitted.
fn submitted_finding(id: &str) -> Finding {
    let raw = b"poc-bytes-fixture".to_vec();
    Finding {
        id: id.to_string(),
        target_id: TargetId("t-1".to_string()),
        fingerprint: format!("fp-{id}"),
        title: format!("finding {id}"),
        state: FindingState::Submitted,
        evidence: Evidence {
            sha256: sha256_hex(&raw),
            raw,
            custody: Vec::new(),
            truncated: false,
        },
        observation_count: 1,
        reject_reason: None,
    }
}

/// Applies platform triage events to a finding through the legal state
/// machine. Owns the event log (the tracking ledger), the quarantine
/// bin for refused events, and the duplicate-of links.
#[derive(Default)]
struct TriageTracker {
    /// The tracking ledger: every consumed event, in order.
    event_log: Vec<String>,
    /// Events refused as illegal transitions, preserved verbatim.
    quarantine: Vec<TriageEvent>,
    /// duplicate-of links: (finding id, original report id).
    duplicate_of: Vec<(String, String)>,
}

impl TriageTracker {
    fn new() -> Self {
        TriageTracker {
            event_log: Vec::new(),
            quarantine: Vec::new(),
            duplicate_of: Vec::new(),
        }
    }

    /// The platform acknowledged the submission: `Submitted → Triage`.
    /// The tracker's own bookkeeping step, not a platform verdict.
    fn note_intake(&mut self, finding: &mut Finding) -> Result<(), IllegalTransition> {
        let next = finding.state.transition(FindingState::Triage)?;
        self.event_log
            .push(format!("{}: Submitted -> Triage (intake ack)", finding.id));
        finding.state = next;
        Ok(())
    }

    /// Apply one platform verdict event. Unknown states are recorded,
    /// never mapped; illegal transitions quarantine the event.
    fn apply(
        &mut self,
        finding: &mut Finding,
        event: &TriageEvent,
    ) -> Result<(), IllegalTransition> {
        let target = match &event.kind {
            TriageKind::Unknown(raw) => {
                self.event_log.push(format!(
                    "{}: Unknown({raw}) recorded; state unchanged ({:?})",
                    finding.id, finding.state
                ));
                return Ok(());
            }
            TriageKind::NeedsMoreInfo => FindingState::NeedsMoreInfo,
            TriageKind::Accepted => FindingState::Accepted,
            TriageKind::Closed => FindingState::Closed,
            TriageKind::DuplicateOf(_) => FindingState::Duplicate,
        };
        let from = format!("{:?}", finding.state);
        match finding.state.transition(target) {
            Ok(next) => {
                if let TriageKind::DuplicateOf(orig) = &event.kind {
                    self.duplicate_of.push((finding.id.clone(), orig.clone()));
                    self.event_log.push(format!(
                        "{}: {from} -> Duplicate (duplicate-of {orig})",
                        finding.id
                    ));
                } else {
                    self.event_log
                        .push(format!("{}: {from} -> {:?}", finding.id, next));
                }
                finding.state = next;
                Ok(())
            }
            Err(e) => {
                self.quarantine.push(event.clone());
                Err(e)
            }
        }
    }
}

fn scripted_platform(events: Vec<TriageEvent>) -> FakePlatform {
    let mut p = FakePlatform::new();
    for e in events {
        p.queue_event(e);
    }
    p
}

fn verdict_event(finding_id: &str, kind: TriageKind) -> TriageEvent {
    TriageEvent {
        finding_id: finding_id.to_string(),
        kind,
        at: NOW,
    }
}

/// V1: scripted `Submitted → Triage → Accepted` → the ledger matches
/// exactly and the finding ends `Accepted`.
fn case_submitted_triage_accepted() -> Result<CaseReport, TaskDriverError> {
    let mut platform = scripted_platform(vec![verdict_event("f000001", TriageKind::Accepted)]);
    let mut finding = submitted_finding("f000001");
    let mut tracker = TriageTracker::new();
    let mut failures = Vec::new();
    if let Err(e) = tracker.note_intake(&mut finding) {
        failures.push(format!("intake failed: {e:?}"));
    }
    for e in platform.poll_events() {
        if let Err(e) = tracker.apply(&mut finding, &e) {
            failures.push(format!("verdict refused: {e:?}"));
        }
    }
    if finding.state != FindingState::Accepted {
        failures.push(format!("final state {:?}, want Accepted", finding.state));
    }
    if tracker.event_log.len() != 2 {
        failures.push(format!(
            "event log has {} entries, want 2",
            tracker.event_log.len()
        ));
    }
    if !tracker.quarantine.is_empty() {
        failures.push("quarantine not empty on the legal path".to_string());
    }
    let mut evidence = tracker.event_log.clone();
    evidence.push(format!("final: {:?}", finding.state));
    evidence.push("backend: FakePlatform scripted events (MOCK)".to_string());
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "final_state": format!("{:?}", finding.state),
            "events": tracker.event_log.len(),
            "quarantined": tracker.quarantine.len(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: the platform reports `DuplicateOf(orig-123)` → the finding is
/// marked `Duplicate` and linked to the original report id (the
/// original_report_id the HackerOne state_change API carries).
fn case_duplicate_links_original() -> Result<CaseReport, TaskDriverError> {
    let mut platform = scripted_platform(vec![verdict_event(
        "f000002",
        TriageKind::DuplicateOf("orig-123".to_string()),
    )]);
    let mut finding = submitted_finding("f000002");
    let mut tracker = TriageTracker::new();
    let mut failures = Vec::new();
    if let Err(e) = tracker.note_intake(&mut finding) {
        failures.push(format!("intake failed: {e:?}"));
    }
    for e in platform.poll_events() {
        if let Err(e) = tracker.apply(&mut finding, &e) {
            failures.push(format!("verdict refused: {e:?}"));
        }
    }
    if finding.state != FindingState::Duplicate {
        failures.push(format!("final state {:?}, want Duplicate", finding.state));
    }
    if tracker.duplicate_of != vec![("f000002".to_string(), "orig-123".to_string())] {
        failures.push(format!(
            "duplicate-of link wrong: {:?}",
            tracker.duplicate_of
        ));
    }
    let mut evidence = tracker.event_log.clone();
    evidence.push(format!("duplicate_of: {:?}", tracker.duplicate_of));
    evidence.push("backend: FakePlatform scripted events (MOCK)".to_string());
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "final_state": format!("{:?}", finding.state),
            "duplicate_of": tracker.duplicate_of,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1: the platform sends an unknown state string → recorded as
/// `Unknown("…")` on the event log, finding state unchanged. Never
/// guessed, never mapped.
fn case_unknown_state_never_mapped() -> Result<CaseReport, TaskDriverError> {
    let mut platform = scripted_platform(vec![verdict_event(
        "f000003",
        TriageKind::Unknown("triaged_by_contractor".to_string()),
    )]);
    let mut finding = submitted_finding("f000003");
    let mut tracker = TriageTracker::new();
    let mut failures = Vec::new();
    if let Err(e) = tracker.note_intake(&mut finding) {
        failures.push(format!("intake failed: {e:?}"));
    }
    for e in platform.poll_events() {
        if let Err(e) = tracker.apply(&mut finding, &e) {
            failures.push(format!("unknown state should not error: {e:?}"));
        }
    }
    if finding.state != FindingState::Triage {
        failures.push(format!(
            "state moved on unknown input: {:?}, want Triage",
            finding.state
        ));
    }
    let joined = tracker.event_log.join("\n");
    if !joined.contains("Unknown(triaged_by_contractor)") {
        failures.push(format!("unknown state not recorded verbatim:\n{joined}"));
    }
    if !tracker.quarantine.is_empty() {
        failures.push("unknown state should be logged, not quarantined".to_string());
    }
    let mut evidence = tracker.event_log.clone();
    evidence.push(format!("state unchanged: {:?}", finding.state));
    evidence.push("backend: FakePlatform scripted events (MOCK)".to_string());
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "state": format!("{:?}", finding.state),
            "quarantined": tracker.quarantine.len(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2: the platform sends `Accepted` for a finding still in
/// `Submitted` (skipping `Triage`) → rejected as an illegal
/// transition, the event quarantined, the state untouched.
fn case_skipped_state_rejected() -> Result<CaseReport, TaskDriverError> {
    let mut platform = scripted_platform(vec![verdict_event("f000004", TriageKind::Accepted)]);
    let mut finding = submitted_finding("f000004");
    let mut tracker = TriageTracker::new();
    let mut failures = Vec::new();
    // No intake: the finding is still Submitted when the verdict lands.
    for e in platform.poll_events() {
        match tracker.apply(&mut finding, &e) {
            Err(IllegalTransition { from, to }) if from == "Submitted" && to == "Accepted" => {}
            Err(e) => failures.push(format!("wrong rejection: {e:?}")),
            Ok(()) => failures.push("skipped Triage was ACCEPTED!".to_string()),
        }
    }
    if finding.state != FindingState::Submitted {
        failures.push(format!("state moved: {:?}, want Submitted", finding.state));
    }
    if tracker.quarantine.len() != 1 {
        failures.push(format!(
            "quarantine has {} events, want 1",
            tracker.quarantine.len()
        ));
    }
    let mut evidence = vec![
        "rejection: IllegalTransition{from: Submitted, to: Accepted}".to_string(),
        format!("state untouched: {:?}", finding.state),
        format!("quarantined events: {}", tracker.quarantine.len()),
        "backend: FakePlatform scripted events (MOCK)".to_string(),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "rejection": "IllegalTransition",
            "state": format!("{:?}", finding.state),
            "quarantined": tracker.quarantine.len(),
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
        "submitted_triage_accepted" => case_submitted_triage_accepted(),
        "duplicate_links_original" => case_duplicate_links_original(),
        "unknown_state_never_mapped" => case_unknown_state_never_mapped(),
        "skipped_state_rejected" => case_skipped_state_rejected(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-147: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// core tracking sequence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-147".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-147".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
