//! Task 145 — exact submission payload preview (rust, V/A).
//!
//! The wire truth: the operator reviews byte-exactly what would be
//! sent — the submission envelope (program id, submission id, content
//! hash) plus the report body — before authorizing the send. The
//! preview bytes must equal the serializer's output bytes exactly
//! (asserted byte-for-byte and by SHA-256). Any post-preview mutation
//! of the finding invalidates the preview: the content hash no longer
//! matches, the preview is stale, and the submission gate refuses the
//! send until the operator re-previews. Preview is part of the approval
//! flow, not a side door: requesting it for a non-`Approved` finding is
//! a typed refusal.
//!
//! All verdict evidence comes from scripted fixtures (MOCK); the
//! platform is the scaffold's `FakePlatform` (scripted double).

use crate::bounty::*;
use crate::skillopt::driver::{CaseReport, TaskDriverError, verdict_line};
use crate::skillopt::learner::Verdict;
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-145";
/// Task name.
pub const NAME: &str = "submission-payload-preview";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "preview_equals_serializer_output",
    "preview_carries_envelope",
    "mutation_invalidates_preview",
    "unapproved_preview_refused",
];

/// The operator-reviewed preview: the exact bytes that would be sent,
/// their SHA-256, and the content hash of the report they were built
/// from. Staleness is detected by re-deriving the content hash.
#[derive(Clone, Debug)]
pub struct Preview {
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub content_hash: String,
}

/// Typed preview errors. `Stale` blocks the submit path until the
/// operator re-previews; `NotApproved` keeps preview inside the
/// approval flow.
#[derive(Debug, PartialEq, Eq)]
pub enum PreviewError {
    NotApproved { state: String },
    Stale { expected: String, got: String },
}

impl std::fmt::Display for PreviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotApproved { state } => {
                write!(f, "preview refused: finding is {state}, not Approved")
            }
            Self::Stale { expected, got } => {
                write!(f, "preview stale: expected content {expected}, got {got}")
            }
        }
    }
}

impl std::error::Error for PreviewError {}

/// Build the submission envelope the operator reviews: program id,
/// submission id, the content hash of the report, and the report
/// itself. Key order is fixed by construction; `serde_json` preserves
/// insertion order, so serialization is deterministic.
pub fn preview_payload(
    finding: &Finding,
    report_md: &str,
    program_id: &str,
    submission_id: &str,
) -> Result<Preview, PreviewError> {
    if finding.state != FindingState::Approved {
        return Err(PreviewError::NotApproved {
            state: format!("{:?}", finding.state),
        });
    }
    let content_hash = approve::sha256_hex(report_md.as_bytes());
    let envelope = serde_json::json!({
        "program_id": program_id,
        "submission_id": submission_id,
        "content_hash": content_hash,
        "report": report_md,
    });
    // to_vec on a constructed serde_json::Value cannot fail; the
    // expect documents the infallibility at the single call site.
    let bytes = serde_json::to_vec(&envelope)
        .expect("task-145: serde_json::to_vec on a constructed Value is infallible");
    let sha256 = approve::sha256_hex(&bytes);
    Ok(Preview {
        bytes,
        sha256,
        content_hash,
    })
}

/// Re-derive the report's content hash and compare it with the
/// preview's. Any post-preview mutation of the finding changes the
/// report bytes, so the mismatch marks the preview stale.
pub fn check_preview_fresh(preview: &Preview, report_md: &str) -> Result<(), PreviewError> {
    let got = approve::sha256_hex(report_md.as_bytes());
    if got != preview.content_hash {
        return Err(PreviewError::Stale {
            expected: preview.content_hash.clone(),
            got,
        });
    }
    Ok(())
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

fn approved_finding() -> Finding {
    Finding {
        id: "f000001".to_string(),
        target_id: TargetId("t-web-01".to_string()),
        fingerprint: "fp-145".to_string(),
        title: "mock: reflected XSS in /search".to_string(),
        state: FindingState::Approved,
        evidence: mock_evidence(),
        observation_count: 2,
        reject_reason: None,
    }
}

fn mock_report_md() -> String {
    "# Bug bounty report: mock: reflected XSS in /search\n\
     \n\
     ## Summary\n\
     \n\
     Reflected XSS in /search.\n"
        .to_string()
}

fn operator_approval(program_id: &str, scope_version: u64, nonce: u64, now: u64) -> Approval {
    Approval {
        program_id: program_id.to_string(),
        scope_version,
        granted_at: now,
        expires_at: now + 3600,
        nonce,
        issuer: "operator".to_string(),
    }
}

/// V1: the preview bytes equal the serializer's output bytes exactly —
/// byte-for-byte and by SHA-256 — and the same bytes reach the
/// platform with no hidden mutation between preview and submit.
fn case_preview_equals_serializer_output() -> Result<CaseReport, TaskDriverError> {
    let finding = approved_finding();
    let report_md = mock_report_md();
    let mut failures = Vec::new();
    let preview =
        preview_payload(&finding, &report_md, "prog-01", "sub-preview-01").map_err(|e| {
            TaskDriverError::Fixture {
                what: "preview".to_string(),
                detail: format!("task-145: preview refused for an Approved finding: {e}"),
            }
        })?;
    // Independent re-serialization of the same envelope: must match
    // byte-for-byte.
    let content_hash = approve::sha256_hex(report_md.as_bytes());
    let reserialized = serde_json::to_vec(&serde_json::json!({
        "program_id": "prog-01",
        "submission_id": "sub-preview-01",
        "content_hash": content_hash,
        "report": report_md,
    }))
    .expect("task-145: serde_json::to_vec on a constructed Value is infallible");
    if preview.bytes != reserialized {
        failures.push("preview bytes differ from the serializer output".to_string());
    }
    if preview.sha256 != approve::sha256_hex(&preview.bytes) {
        failures.push("preview sha256 does not match its bytes".to_string());
    }
    // The submit path: operator approval bound to the preview hash,
    // then the gate, then the platform. The bytes on the wire must be
    // the preview bytes.
    let now = 1_700_000_000;
    let approval = operator_approval("prog-01", 7, 4242, now);
    let mut gate = SubmissionGate::new();
    match gate.submit(
        &finding.id,
        finding.state == FindingState::Approved,
        &preview.bytes,
        &preview.sha256,
        Some(&approval),
        "prog-01",
        7,
        now,
    ) {
        Ok(_) => {}
        Err(e) => failures.push(format!("gate refused the preview bytes: {e:?}")),
    }
    let mut platform = FakePlatform::new();
    match platform.submit(&preview.bytes) {
        Ok(_) => {}
        Err(code) => failures.push(format!("platform refused the payload: {code}")),
    }
    match platform.last_payload() {
        Some(sent) if sent == preview.bytes.as_slice() => {}
        _ => failures.push("bytes on the wire differ from the preview bytes".to_string()),
    }
    let mut evidence_lines = vec![
        format!("preview bytes: {}", preview.bytes.len()),
        format!("preview sha256: {}", preview.sha256),
        "preview == serializer output (byte-for-byte)".to_string(),
        "gate authorized; platform received the identical bytes".to_string(),
        verdict_line("145", Verdict::Replicates, "what you see is what you send"),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "preview_bytes": preview.bytes.len(),
            "sha256": preview.sha256,
            "wire_matches_preview": failures.is_empty(),
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: the preview carries the full envelope — program id, submission
/// id, and the content hash binding it to the exact report bytes.
fn case_preview_carries_envelope() -> Result<CaseReport, TaskDriverError> {
    let finding = approved_finding();
    let report_md = mock_report_md();
    let mut failures = Vec::new();
    let preview =
        preview_payload(&finding, &report_md, "prog-01", "sub-preview-01").map_err(|e| {
            TaskDriverError::Fixture {
                what: "preview".to_string(),
                detail: format!("task-145: preview refused: {e}"),
            }
        })?;
    let envelope: serde_json::Value =
        serde_json::from_slice(&preview.bytes).map_err(|e| TaskDriverError::Fixture {
            what: "envelope".to_string(),
            detail: format!("task-145: preview bytes are not JSON: {e}"),
        })?;
    for (key, want) in [
        ("program_id", "prog-01"),
        ("submission_id", "sub-preview-01"),
    ] {
        if envelope.get(key).and_then(|v| v.as_str()) != Some(want) {
            failures.push(format!("envelope missing or wrong {key}"));
        }
    }
    let want_hash = approve::sha256_hex(report_md.as_bytes());
    if envelope.get("content_hash").and_then(|v| v.as_str()) != Some(want_hash.as_str()) {
        failures.push("envelope content_hash does not bind the report bytes".to_string());
    }
    if envelope.get("report").and_then(|v| v.as_str()) != Some(report_md.as_str()) {
        failures.push("envelope report body differs from the previewed report".to_string());
    }
    let mut evidence_lines = vec![
        "envelope keys: program_id, submission_id, content_hash, report".to_string(),
        format!("content_hash: {want_hash}"),
        verdict_line(
            "145",
            Verdict::Replicates,
            "envelope complete and hash-bound",
        ),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "content_hash": want_hash,
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1: mutate the finding after preview. The preview goes stale
/// (typed `PreviewError::Stale`), the gate refuses the mutated bytes
/// with `HashMismatch`, and the send only proceeds after re-preview.
fn case_mutation_invalidates_preview() -> Result<CaseReport, TaskDriverError> {
    let mut finding = approved_finding();
    let report_v1 = mock_report_md();
    let mut failures = Vec::new();
    let preview =
        preview_payload(&finding, &report_v1, "prog-01", "sub-preview-01").map_err(|e| {
            TaskDriverError::Fixture {
                what: "preview".to_string(),
                detail: format!("task-145: preview refused: {e}"),
            }
        })?;
    // Mutate the finding: the report bytes change.
    finding.title = "mock: reflected XSS in /search (escalated to stored)".to_string();
    let report_v2 = format!("{report_v1}\n## Update\n\nEscalated to stored XSS.\n");
    match check_preview_fresh(&preview, &report_v2) {
        Err(PreviewError::Stale { .. }) => {}
        other => failures.push(format!("mutated report not marked stale: {other:?}")),
    }
    // The un-mutated report is still fresh against the preview.
    if check_preview_fresh(&preview, &report_v1).is_err() {
        failures.push("preview wrongly stale for its own report".to_string());
    }
    // Submitting the mutated bytes under the old approval hash fails.
    let now = 1_700_000_000;
    let approval = operator_approval("prog-01", 7, 4243, now);
    let mut gate = SubmissionGate::new();
    let mutated_envelope = serde_json::to_vec(&serde_json::json!({
        "program_id": "prog-01",
        "submission_id": "sub-preview-01",
        "content_hash": approve::sha256_hex(report_v2.as_bytes()),
        "report": report_v2,
    }))
    .expect("task-145: serde_json::to_vec on a constructed Value is infallible");
    match gate.submit(
        &finding.id,
        true,
        &mutated_envelope,
        &preview.sha256,
        Some(&approval),
        "prog-01",
        7,
        now,
    ) {
        Err(GateError::HashMismatch { .. }) => {}
        other => failures.push(format!(
            "gate did not HashMismatch the mutated bytes: {other:?}"
        )),
    }
    // Re-preview after the mutation: fresh again, gate authorizes.
    let preview2 =
        preview_payload(&finding, &report_v2, "prog-01", "sub-preview-02").map_err(|e| {
            TaskDriverError::Fixture {
                what: "re-preview".to_string(),
                detail: format!("task-145: re-preview refused: {e}"),
            }
        })?;
    if check_preview_fresh(&preview2, &report_v2).is_err() {
        failures.push("re-preview not fresh for the mutated report".to_string());
    }
    let approval2 = operator_approval("prog-01", 7, 4244, now);
    match gate.submit(
        &finding.id,
        true,
        &preview2.bytes,
        &preview2.sha256,
        Some(&approval2),
        "prog-01",
        7,
        now,
    ) {
        Ok(_) => {}
        Err(e) => failures.push(format!("gate refused the re-previewed bytes: {e:?}")),
    }
    let mut evidence_lines = vec![
        "mutation after preview -> PreviewError::Stale".to_string(),
        "mutated bytes under old hash -> GateError::HashMismatch".to_string(),
        "re-preview -> fresh; gate authorizes the new bytes".to_string(),
        verdict_line(
            "145",
            Verdict::Replicates,
            "stale preview blocks submit until re-previewed",
        ),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "stale_detected": true,
            "hash_mismatch": true,
            "repreview_authorized": failures.is_empty(),
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2: preview is part of the approval flow, not a side door —
/// requesting it for a non-`Approved` finding is a typed refusal.
fn case_unapproved_preview_refused() -> Result<CaseReport, TaskDriverError> {
    let report_md = mock_report_md();
    let mut failures = Vec::new();
    for state in [FindingState::Reportable, FindingState::Candidate] {
        let want = format!("{state:?}");
        let mut finding = approved_finding();
        finding.state = state;
        match preview_payload(&finding, &report_md, "prog-01", "sub-preview-01") {
            Err(PreviewError::NotApproved { state }) if state == want => {}
            other => failures.push(format!("preview not refused for {want}: {other:?}")),
        }
    }
    let mut evidence_lines = vec![
        "Reportable -> Err(PreviewError::NotApproved)".to_string(),
        "Candidate -> Err(PreviewError::NotApproved)".to_string(),
        verdict_line(
            "145",
            Verdict::Replicates,
            "preview stays inside the approval flow",
        ),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "refused_states": ["Reportable", "Candidate"],
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
        "preview_equals_serializer_output" => case_preview_equals_serializer_output(),
        "preview_carries_envelope" => case_preview_carries_envelope(),
        "mutation_invalidates_preview" => case_mutation_invalidates_preview(),
        "unapproved_preview_refused" => case_unapproved_preview_refused(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-145: unknown case '{case}'"),
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
            where_: "task-145".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-145".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
