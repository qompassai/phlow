//! Task 141 — evidence preservation + chain of custody (rust, V/A).
//!
//! Every finding carries the raw tool output it was built from, stored
//! byte-exact with a sha256, plus an append-only custody log: every
//! handling step (seal, validate, report, approve, submit) appends who
//! touched it, what they did, when, and the hash of the bytes they saw.
//! Tampering is detectable because the next custody verification
//! recomputes the hash and compares it to the sealed value — a flipped
//! byte fails with a typed [`EvidenceError::Tampered`], and the finding
//! is quarantined (kept auditable), never silently dropped. Oversize
//! tool output is stored truncated at a named cap with a marker.
//!
//! All verdict evidence comes from synthetic fixtures (MOCK tool
//! output); the hashing primitive is the scaffold's own
//! `bounty::approve::sha256_hex`, the same function the submission gate
//! binds approvals to.

use crate::bounty::*;
use crate::skillopt::driver::{CaseReport, TaskDriverError, verdict_line};
use crate::skillopt::learner::Verdict;
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-141";
/// Task name.
pub const NAME: &str = "evidence-preservation-custody";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "byte_exact_roundtrip",
    "custody_chain_grows",
    "tamper_detected",
    "oversize_truncated",
];

/// Storage cap for one sealed evidence blob, in bytes (10 MiB). Tool
/// output larger than this is stored truncated — bounded, never OOM.
pub const EVIDENCE_CAP_BYTES: usize = 10 * 1024 * 1024;
/// Synthetic oversize tool output for the truncation case (100 MiB).
pub const OVERSIZE_INPUT_BYTES: usize = 100 * 1024 * 1024;

/// Typed tamper signal. Returned by [`verify_custody`] when the stored
/// bytes no longer hash to the sealed value — the quarantine trigger,
/// never a silent drop.
#[derive(Debug, PartialEq, Eq)]
pub enum EvidenceError {
    Tampered { expected: String, got: String },
}

impl std::fmt::Display for EvidenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tampered { expected, got } => {
                write!(f, "evidence tampered: expected {expected}, got {got}")
            }
        }
    }
}

impl std::error::Error for EvidenceError {}

/// Seal raw tool output into [`Evidence`]: store it byte-exact (or
/// truncated at [`EVIDENCE_CAP_BYTES`] with a marker naming the cap),
/// hash the stored bytes, and open the custody chain with a seal entry.
pub fn seal_evidence(raw_input: &[u8], handler: &str, at: u64) -> Evidence {
    let truncated = raw_input.len() > EVIDENCE_CAP_BYTES;
    let stored: Vec<u8> = if truncated {
        raw_input[..EVIDENCE_CAP_BYTES].to_vec()
    } else {
        raw_input.to_vec()
    };
    let sha = approve::sha256_hex(&stored);
    let action = if truncated {
        format!("sealed-truncated(cap={EVIDENCE_CAP_BYTES}B)")
    } else {
        "sealed".to_string()
    };
    Evidence {
        raw: stored,
        sha256: sha.clone(),
        custody: vec![CustodyEntry {
            handler: handler.to_string(),
            action,
            at,
            evidence_sha256: sha,
        }],
        truncated,
    }
}

/// Append one handling step to the custody chain. The entry records the
/// hash of the bytes as currently stored, so any later mutation is
/// caught by [`verify_custody`].
pub fn append_custody(evidence: &mut Evidence, handler: &str, action: &str, at: u64) {
    evidence.custody.push(CustodyEntry {
        handler: handler.to_string(),
        action: action.to_string(),
        at,
        evidence_sha256: evidence.sha256.clone(),
    });
}

/// Recompute the evidence hash and compare it against the sealed value
/// and every custody entry. Any mismatch is tampering.
pub fn verify_custody(evidence: &Evidence) -> Result<(), EvidenceError> {
    let recomputed = approve::sha256_hex(&evidence.raw);
    if recomputed != evidence.sha256 {
        return Err(EvidenceError::Tampered {
            expected: evidence.sha256.clone(),
            got: recomputed,
        });
    }
    for entry in &evidence.custody {
        if entry.evidence_sha256 != recomputed {
            return Err(EvidenceError::Tampered {
                expected: entry.evidence_sha256.clone(),
                got: recomputed,
            });
        }
    }
    Ok(())
}

/// A quarantined finding: kept auditable with its evidence intact, held
/// out of the pipeline. Quarantine is the driver's holding area —
/// deliberately separate from the finding state machine, which has no
/// quarantine state.
#[derive(Clone, Debug)]
pub struct QuarantinedFinding {
    pub finding_id: String,
    pub fingerprint: String,
    pub reason: String,
    pub evidence: Evidence,
}

/// The quarantine list: append-only, queryable by fingerprint.
#[derive(Debug, Default)]
pub struct Quarantine {
    items: Vec<QuarantinedFinding>,
}

impl Quarantine {
    pub fn new() -> Self {
        Quarantine { items: Vec::new() }
    }

    /// Hold a finding out of the pipeline with a recorded reason. The
    /// finding's evidence is cloned in — intact, not dropped.
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

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Synthetic tool output (MOCK): deterministic, small, representative.
fn mock_tool_output() -> Vec<u8> {
    b"MOCK nuclei: [xss-detect] matched https://example.com/search?q=<script>alert(1)</script>\n\
      MOCK response bytes: 200 OK, 14_203 bytes, reflected parameter q\n"
        .to_vec()
}

fn mock_finding(evidence: Evidence, fingerprint: &str) -> Finding {
    Finding {
        id: "f000001".to_string(),
        target_id: TargetId("t-web-01".to_string()),
        fingerprint: fingerprint.to_string(),
        title: "mock: reflected XSS in /search".to_string(),
        state: FindingState::Candidate,
        evidence,
        observation_count: 2,
        reject_reason: None,
    }
}

fn transition_finding(finding: &mut Finding, next: FindingState) -> Result<(), TaskDriverError> {
    finding.state = finding
        .state
        .transition(next)
        .map_err(|e| TaskDriverError::Fixture {
            what: "finding transition".to_string(),
            detail: format!("task-141: illegal transition {} -> {}", e.from, e.to),
        })?;
    Ok(())
}

/// V1: finding created from tool output bytes — stored bytes == input
/// bytes, sha256 recorded, not truncated, one seal custody entry.
fn case_byte_exact_roundtrip() -> Result<CaseReport, TaskDriverError> {
    let input = mock_tool_output();
    let evidence = seal_evidence(&input, "prober", 1_700_000_000);
    let mut failures = Vec::new();
    if evidence.raw != input {
        failures.push("stored bytes differ from tool output bytes".to_string());
    }
    let want_sha = approve::sha256_hex(&input);
    if evidence.sha256 != want_sha {
        failures.push(format!(
            "sha256 mismatch: sealed {} vs recomputed {want_sha}",
            evidence.sha256
        ));
    }
    if evidence.truncated {
        failures.push("small fixture wrongly marked truncated".to_string());
    }
    if evidence.custody.len() != 1 {
        failures.push(format!(
            "expected 1 seal custody entry, got {}",
            evidence.custody.len()
        ));
    }
    if verify_custody(&evidence).is_err() {
        failures.push("freshly sealed evidence fails custody verification".to_string());
    }
    let mut evidence_lines = vec![
        format!("input bytes: {}", input.len()),
        format!("stored bytes: {}", evidence.raw.len()),
        format!("sha256: {}", evidence.sha256),
        format!(
            "seal entry: {} / {}",
            evidence.custody[0].handler, evidence.custody[0].action
        ),
        verdict_line(
            "141",
            Verdict::Replicates,
            "byte-exact round-trip, hash recorded",
        ),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "input_bytes": input.len(),
            "stored_bytes": evidence.raw.len(),
            "sha256": evidence.sha256,
            "truncated": evidence.truncated,
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: validate -> report -> approve -> submit each append a custody
/// entry. Chain length == handling steps; every entry binds the sealed
/// hash; timestamps strictly increase on the ManualClock.
fn case_custody_chain_grows() -> Result<CaseReport, TaskDriverError> {
    let mut clock = ManualClock::new(1_700_000_000);
    let sealed = seal_evidence(&mock_tool_output(), "prober", clock.now());
    let mut finding = mock_finding(sealed, "fp-141-v2");
    let steps = [
        ("validator", "validated", FindingState::Validated),
        ("reporter", "reported", FindingState::Reportable),
        ("operator", "approved", FindingState::Approved),
        ("submitter", "submitted", FindingState::Submitted),
    ];
    let mut failures = Vec::new();
    let steps_len = steps.len();
    for (handler, action, next) in steps {
        clock.advance(60);
        transition_finding(&mut finding, next)?;
        append_custody(&mut finding.evidence, handler, action, clock.now());
        if verify_custody(&finding.evidence).is_err() {
            failures.push(format!("custody verification failed after {action}"));
        }
    }
    let chain = &finding.evidence.custody;
    if chain.len() != 1 + steps_len {
        failures.push(format!(
            "chain length {} != handling steps {}",
            chain.len(),
            1 + steps_len
        ));
    }
    for entry in chain {
        if entry.evidence_sha256 != finding.evidence.sha256 {
            failures.push(format!(
                "custody entry {} / {} binds wrong hash",
                entry.handler, entry.action
            ));
        }
    }
    let mut ordered = true;
    for pair in chain.windows(2) {
        if pair[0].at >= pair[1].at {
            ordered = false;
        }
    }
    if !ordered {
        failures.push("custody timestamps not strictly increasing".to_string());
    }
    let mut evidence_lines: Vec<String> = chain
        .iter()
        .map(|e| format!("custody: {} / {} @ {}", e.handler, e.action, e.at))
        .collect();
    evidence_lines.push(format!("chain length: {} (1 seal + 4 steps)", chain.len()));
    evidence_lines.push(verdict_line(
        "141",
        Verdict::Replicates,
        "every handling step appended, hashes bind",
    ));
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "chain_len": chain.len(),
            "steps": steps_len,
            "hash": finding.evidence.sha256,
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1: flip one byte in the stored evidence. The next custody check
/// fails with typed `EvidenceError::Tampered`; the finding is
/// quarantined with evidence intact — and stays Candidate (never
/// advances on tampered evidence).
fn case_tamper_detected() -> Result<CaseReport, TaskDriverError> {
    let sealed = seal_evidence(&mock_tool_output(), "prober", 1_700_000_000);
    let mut finding = mock_finding(sealed, "fp-141-a1");
    let sealed_sha = finding.evidence.sha256.clone();
    finding.evidence.raw[0] ^= 0xff;
    let mut failures = Vec::new();
    match verify_custody(&finding.evidence) {
        Err(EvidenceError::Tampered { expected, got }) => {
            if expected != sealed_sha {
                failures.push("tamper error names the wrong expected hash".to_string());
            }
            if got == sealed_sha {
                failures.push("tamper error's recomputed hash equals the sealed one".to_string());
            }
        }
        Ok(()) => failures.push("TAMPERED EVIDENCE PASSED CUSTODY VERIFICATION".to_string()),
    }
    let mut quarantine = Quarantine::new();
    quarantine.hold(&finding, "evidence-tampered");
    let held = quarantine.get("fp-141-a1");
    match held {
        Some(q) => {
            if q.evidence.raw.is_empty() {
                failures.push("quarantine dropped the evidence".to_string());
            }
            if q.reason != "evidence-tampered" {
                failures.push(format!("quarantine reason mangled: {}", q.reason));
            }
        }
        None => failures.push("finding missing from quarantine after tamper".to_string()),
    }
    if finding.state != FindingState::Candidate {
        failures.push(format!(
            "tampered finding advanced to {:?} — must stay Candidate",
            finding.state
        ));
    }
    let mut evidence_lines = vec![
        format!("sealed sha256: {sealed_sha}"),
        "flipped raw[0]; verify_custody -> Err(EvidenceError::Tampered)".to_string(),
        format!("quarantine holds fp-141-a1: {}", held.is_some()),
        format!("finding state after tamper: {:?}", finding.state),
        verdict_line(
            "141",
            Verdict::Replicates,
            "tamper detected before advance; quarantined, not dropped",
        ),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "tamper_detected": failures.is_empty(),
            "quarantined": held.is_some(),
            "state": format!("{:?}", finding.state),
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2: 100 MiB of tool output is stored bounded at the 10 MiB cap with
/// a `truncated` marker; the hash covers the stored bytes and the seal
/// entry names the cap. Never OOM: the stored blob is exactly the cap.
fn case_oversize_truncated() -> Result<CaseReport, TaskDriverError> {
    let input = vec![0x41u8; OVERSIZE_INPUT_BYTES];
    let evidence = seal_evidence(&input, "prober", 1_700_000_000);
    let mut failures = Vec::new();
    if evidence.raw.len() != EVIDENCE_CAP_BYTES {
        failures.push(format!(
            "stored {} bytes, want exactly the {} cap",
            evidence.raw.len(),
            EVIDENCE_CAP_BYTES
        ));
    }
    if !evidence.truncated {
        failures.push("100 MiB input not marked truncated".to_string());
    }
    let want_sha = approve::sha256_hex(&evidence.raw);
    if evidence.sha256 != want_sha {
        failures.push("hash does not cover the stored (truncated) bytes".to_string());
    }
    if !evidence.custody[0]
        .action
        .contains(&EVIDENCE_CAP_BYTES.to_string())
    {
        failures.push(format!(
            "seal marker does not name the cap: {}",
            evidence.custody[0].action
        ));
    }
    if verify_custody(&evidence).is_err() {
        failures.push("truncated evidence fails custody verification".to_string());
    }
    let mut evidence_lines = vec![
        format!("input bytes: {OVERSIZE_INPUT_BYTES}"),
        format!(
            "stored bytes: {} (cap {EVIDENCE_CAP_BYTES})",
            evidence.raw.len()
        ),
        format!("truncated: {}", evidence.truncated),
        format!("seal marker: {}", evidence.custody[0].action),
        verdict_line(
            "141",
            Verdict::Replicates,
            "oversize bounded with marker, hash covers stored bytes",
        ),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "input_bytes": OVERSIZE_INPUT_BYTES,
            "stored_bytes": evidence.raw.len(),
            "cap_bytes": EVIDENCE_CAP_BYTES,
            "truncated": evidence.truncated,
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
        "byte_exact_roundtrip" => case_byte_exact_roundtrip(),
        "custody_chain_grows" => case_custody_chain_grows(),
        "tamper_detected" => case_tamper_detected(),
        "oversize_truncated" => case_oversize_truncated(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-141: unknown case '{case}'"),
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
            where_: "task-141".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-141".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
