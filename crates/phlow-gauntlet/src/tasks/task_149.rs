//! Task 149 — secret non-disclosure (rust, A).
//!
//! The seam is every text surface: logs, reports, diffs, error
//! messages. Platform tokens and PoC credentials must never appear in
//! any persisted or displayed text. Redaction is at write time
//! ([`redact_text`]) against a configured secret list; the
//! [`SecretVault`] holds the real values in memory only, with the
//! single authorized read path (`get`) reserved for the authenticated
//! submit call. Four scenarios: tool output with a token is redacted
//! in the log (V1); a rendered report is redacted while the vault
//! retains the real value (V2); exfiltration via the finding title is
//! redacted (A1); exfiltration via an error message is redacted at the
//! log boundary (A2). Every surface is scanned for secret substrings.
//! Fixture secrets are realistic-looking but FAKE — never real
//! credentials. All doubles are scripted and labeled MOCK.

use crate::bounty::{AuditLog, SecretVault, redact_text};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-149";
/// Task name.
pub const NAME: &str = "secret non-disclosure";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "tool_output_redacted_in_logs",
    "report_redacted_vault_retains_real_value",
    "title_exfiltration_redacted",
    "error_message_redacted_at_boundary",
];

/// Fixture secrets. FAKE values, gauntlet-only — never real credentials.
const PLATFORM_TOKEN: &str = "sk-live-F4K3t0k3n9x2q7m";
const POC_PASSWORD: &str = "hunt3r2-f4k3-pw-2026";

/// The scaffold source, for the no-Serialize-impl check: if anyone adds
/// a `Serialize` impl to `SecretVault`, this test fails closed.
const SECRET_RS: &str = include_str!("../bounty/secret.rs");

/// Narrow check for a real `Serialize` impl on `SecretVault`. The word
/// "Serialize" appears in the struct's own doc comment ("Deliberately
/// has no `Serialize`"), so a plain substring test always fires. Look
/// for actual impl/derive syntax instead.
fn secret_rs_has_serialize() -> bool {
    const MARKERS: [&str; 5] = [
        "impl Serialize for SecretVault",
        "impl serde::Serialize for SecretVault",
        "derive(Serialize",
        "Serialize)]",
        "Serialize,",
    ];
    MARKERS.iter().any(|m| SECRET_RS.contains(m))
}

/// A vault holding the fixture secrets, as the workflow would.
fn fixture_vault() -> SecretVault {
    let mut v = SecretVault::new();
    v.insert("platform_token", PLATFORM_TOKEN);
    v.insert("poc_password", POC_PASSWORD);
    v
}

/// Record a violation when a secret substring appears in `text`. The
/// failure message names the surface, never the secret.
fn scan_clean(text: &str, secrets: &[String], surface: &str, failures: &mut Vec<String>) {
    for s in secrets {
        if !s.is_empty() && text.contains(s.as_str()) {
            failures.push(format!("{surface}: LEAKED a secret value"));
        }
    }
}

/// Render a finding report, redacting at write time. (MOCK renderer —
/// stands in for the real report pipeline.)
fn render_report(title: &str, body: &str, secrets: &[String]) -> String {
    let redacted_title = redact_text(title, secrets);
    let redacted_body = redact_text(body, secrets);
    format!("FINDING REPORT\ntitle: {redacted_title}\nbody: {redacted_body}\n")
}

/// V1: tool output containing a token and a password — including a diff
/// block — is redacted line-by-line as it enters the audit log.
fn case_tool_output_redacted_in_logs() -> Result<CaseReport, TaskDriverError> {
    let vault = fixture_vault();
    let secrets = vault.secret_values();
    // Raw tool output: the secrets are present BEFORE the log boundary.
    // Redaction happens at write time, inside AuditLog::append.
    let tool_output = format!(
        "nuclei scan complete: 1 finding\n\
         --- a/login\n\
         +++ b/login\n\
         +Authorization: Bearer {PLATFORM_TOKEN}\n\
         request: POST /login user=admin password={POC_PASSWORD}\n"
    );
    let mut log = AuditLog::new(secrets.clone());
    for line in tool_output.lines() {
        log.append(line);
    }
    let mut failures = Vec::new();
    let mut redacted_count = 0usize;
    for entry in log.entries() {
        scan_clean(entry, &secrets, "audit-log", &mut failures);
        if entry.contains("[REDACTED]") {
            redacted_count += 1;
        }
    }
    // Exactly the two lines that carried secrets must be redacted; the
    // secret-free lines pass through untouched.
    if redacted_count != 2 {
        failures.push(format!("want 2 redacted entries, got {redacted_count}"));
    }
    if !log.leaks_none() {
        failures.push("AuditLog::leaks_none() is false".to_string());
    }
    let mut evidence: Vec<String> = log.entries().to_vec();
    evidence.push("surface: audit log (incl. diff block)".to_string());
    evidence.push("backend: AuditLog + fixture SecretVault (MOCK)".to_string());
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "entries": log.entries().len(),
            "leaks_none": log.leaks_none(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: the rendered report is fully redacted, while the vault retains
/// the real value for the authenticated submit call — and the vault
/// carries no `Serialize` impl (memory-only, never serialized).
fn case_report_redacted_vault_retains_real_value() -> Result<CaseReport, TaskDriverError> {
    let vault = fixture_vault();
    let secrets = vault.secret_values();
    let title = format!("xss in login — session token={PLATFORM_TOKEN}");
    let body =
        format!("PoC: POST /login with password {POC_PASSWORD}; token above replays the session.");
    let report = render_report(&title, &body, &secrets);
    let mut failures = Vec::new();
    scan_clean(&report, &secrets, "finding-report", &mut failures);
    if !report.contains("[REDACTED]") {
        failures.push("report has no redaction markers".to_string());
    }
    // The authorized read path still serves the real value — the
    // authenticated submit call needs it, and only it.
    match vault.get("platform_token") {
        Some(v) if v == PLATFORM_TOKEN => {}
        other => failures.push(format!("vault authorized read broken: {}", other.is_some())),
    }
    // Memory-only: no Serialize impl may ever appear on SecretVault.
    if secret_rs_has_serialize() {
        failures.push("SecretVault gained a Serialize impl — memory-only violated".to_string());
    }
    let mut evidence = vec![report.clone()];
    // Documented scaffold gap (reported, not fixed — four waves share
    // the scaffold): SecretVault derives Debug, so {:?} prints raw
    // values. Never Debug-log the vault.
    let debug = format!("{vault:?}");
    if debug.contains(PLATFORM_TOKEN) {
        evidence.push(
            "DOCUMENTED GAP: SecretVault derives Debug; {:?} prints raw secret values \
             — never Debug-log the vault (reported to coordinator, scaffold untouched)"
                .to_string(),
        );
    }
    evidence.push("surface: rendered finding report".to_string());
    evidence.push("backend: render_report (MOCK) + fixture SecretVault".to_string());
    evidence.extend(failures.iter().cloned());
    let mut report_case = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "report_clean": failures.is_empty(),
            "vault_has_serialize": secret_rs_has_serialize(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report_case.passed = failures.is_empty();
    Ok(report_case)
}

/// A1: exfiltration via the finding title — the secret is redacted
/// everywhere the title is stored or displayed.
fn case_title_exfiltration_redacted() -> Result<CaseReport, TaskDriverError> {
    let vault = fixture_vault();
    let secrets = vault.secret_values();
    // The attacker plants the secret in the title.
    let hostile_title = format!("xss in login token={PLATFORM_TOKEN} works every time");
    // Stored and displayed only after write-time redaction.
    let stored_title = redact_text(&hostile_title, &secrets);
    let mut log = AuditLog::new(secrets.clone());
    log.append(&format!("finding filed: {stored_title}"));
    let mut failures = Vec::new();
    scan_clean(&stored_title, &secrets, "stored-title", &mut failures);
    for entry in log.entries() {
        scan_clean(entry, &secrets, "audit-log", &mut failures);
    }
    if !stored_title.contains("[REDACTED]") {
        failures.push("title not redacted".to_string());
    }
    let mut evidence = vec![
        format!("stored title: {stored_title}"),
        "surface: finding title (stored + audit-logged)".to_string(),
        "backend: redact_text + AuditLog (MOCK fixtures)".to_string(),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "title_clean": failures.is_empty(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2: exfiltration via an error message — a validator error echoing
/// the secret is redacted at the log boundary.
fn case_error_message_redacted_at_boundary() -> Result<CaseReport, TaskDriverError> {
    let vault = fixture_vault();
    let secrets = vault.secret_values();
    // A validator error that echoes the secret (the classic leak path).
    let validator_error = format!("auth check failed: token {PLATFORM_TOKEN} rejected by platform");
    // The raw error exists in memory; only the redacted form is logged.
    let mut log = AuditLog::new(secrets.clone());
    log.append(&format!("validation error: {validator_error}"));
    let mut failures = Vec::new();
    for entry in log.entries() {
        scan_clean(entry, &secrets, "audit-log", &mut failures);
    }
    if !log.leaks_none() {
        failures.push("AuditLog::leaks_none() is false".to_string());
    }
    let mut evidence: Vec<String> = log.entries().to_vec();
    evidence.push("surface: error message at the log boundary".to_string());
    evidence.push("backend: AuditLog + fixture SecretVault (MOCK)".to_string());
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "entries": log.entries().len(),
            "leaks_none": log.leaks_none(),
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
        "tool_output_redacted_in_logs" => case_tool_output_redacted_in_logs(),
        "report_redacted_vault_retains_real_value" => {
            case_report_redacted_vault_retains_real_value()
        }
        "title_exfiltration_redacted" => case_title_exfiltration_redacted(),
        "error_message_redacted_at_boundary" => case_error_message_redacted_at_boundary(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-149: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// write-time redaction boundary itself.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-149".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-149".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
