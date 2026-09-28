//! Integration tests for task-149 (secret non-disclosure).
//!
//! Four driver cases — 2 validation, 2 adversarial — against the
//! clearly labeled scripted double (fixture SecretVault + AuditLog,
//! deterministic, fast). Write-time redaction must hold on every text
//! surface: audit logs (incl. diff blocks), rendered reports, finding
//! titles, and error messages. The vault retains the real values for
//! the authorized read path and carries no Serialize impl. Fixture
//! secrets are realistic-looking but FAKE.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_149;

fn check_case(case: &str) -> CaseReport {
    let report = task_149::run_case(case)
        .unwrap_or_else(|e| panic!("task-149 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-149 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: tool output containing a token and a password — including a
/// diff block — is redacted line-by-line at the log boundary.
#[test]
fn tool_output_redacted_in_logs() {
    assert_eq!(task_149::ID, "task-149");
    let report = check_case("tool_output_redacted_in_logs");
    let m = &report.metrics;
    assert_eq!(m["leaks_none"].as_bool().unwrap(), true);
    assert!(m["entries"].as_u64().unwrap() > 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("[REDACTED]"),
        "evidence must show the redaction markers:\n{joined}"
    );
    for leak in ["sk-live-", "hunt3r2"] {
        assert!(
            !joined.contains(leak),
            "evidence itself must not carry a fixture secret"
        );
    }
}

/// V2: the rendered report is clean; the vault still serves the real
/// value on the authorized read path; no Serialize impl exists.
#[test]
fn report_redacted_vault_retains_real_value() {
    let report = check_case("report_redacted_vault_retains_real_value");
    let m = &report.metrics;
    assert_eq!(m["report_clean"].as_bool().unwrap(), true);
    assert_eq!(
        m["vault_has_serialize"].as_bool().unwrap(),
        false,
        "SecretVault must stay memory-only"
    );
    let joined = report.evidence.join("\n");
    for leak in ["sk-live-", "hunt3r2"] {
        assert!(
            !joined.contains(leak),
            "the rendered report must not carry a fixture secret"
        );
    }
}

// --- adversarial ---

/// A1: exfiltration via the finding title — redacted everywhere the
/// title is stored or displayed.
#[test]
fn title_exfiltration_redacted() {
    let report = check_case("title_exfiltration_redacted");
    let m = &report.metrics;
    assert_eq!(m["title_clean"].as_bool().unwrap(), true);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("stored title:") && joined.contains("[REDACTED]"),
        "evidence must show the redacted title:\n{joined}"
    );
}

/// A2: exfiltration via an error message — redacted at the log
/// boundary.
#[test]
fn error_message_redacted_at_boundary() {
    let report = check_case("error_message_redacted_at_boundary");
    let m = &report.metrics;
    assert_eq!(m["leaks_none"].as_bool().unwrap(), true);
    let joined = report.evidence.join("\n");
    assert!(
        !joined.contains("sk-live-"),
        "the logged error must not carry the fixture secret"
    );
}
