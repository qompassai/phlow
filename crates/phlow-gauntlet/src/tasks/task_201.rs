//! task-201: outbound credential exclusion.
//!
//! Drives `phlow_llm::build_chat_payload`, the real pre-transport boundary.
//!
//! Honest scope: no broker, endpoint grant, or substitution API exists here.
//! These probes require raw credentials to stay out of model messages and tool
//! schemas before transport, regardless of destination. They do not prove
//! authorized endpoint materialization or unauthorized placeholder forwarding.
//! The canary is synthetic test data, never a machine credential.

use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use phlow_llm::build_chat_payload;
use serde_json::json;

/// Stable task identifier.
pub const ID: &str = "task-201";
/// Human-readable invariant.
pub const NAME: &str = "outbound credential exclusion";
/// Offline Rust seams; no network, subprocess, or Neovim required.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation controls, then two adversarial credential probes.
pub const CASES: [&str; 4] = [
    "ordinary_message_preserved",
    "ordinary_schema_preserved",
    "message_credential_blocked",
    "schema_default_credential_blocked",
];

/// Synthetic API-key-shaped canary; never sourced from a real credential store.
pub(super) const CANARY: &str = "sk-gauntlet-not-a-real-key-201208-0123456789abcdef";

// Shared assertion/report mechanics only; no replacement for a production seam.
pub(super) fn report(
    case: &'static str,
    passed: bool,
    output_bytes: usize,
    seam: &str,
) -> CaseReport {
    let mut result = CaseReport::pass(
        case,
        serde_json::json!({"output_bytes": output_bytes}),
        vec![format!(
            "seam={seam}; invariant_held={passed}; output_bytes={output_bytes}"
        )],
    );
    if !passed {
        result.passed = false;
        result.failures.push(format!(
            "{case}: credential containment or ordinary-data preservation failed"
        ));
    }
    result
}

pub(super) fn case_index(case: &str, cases: &[&str; 4]) -> Result<usize, TaskDriverError> {
    cases
        .iter()
        .position(|candidate| *candidate == case)
        .ok_or_else(|| TaskDriverError::Fixture {
            what: "credential gauntlet case".to_owned(),
            detail: "unknown case name".to_owned(),
        })
}

pub(super) fn run_cases(
    id: &'static str,
    cases: &[&str; 4],
    probe: fn(&str) -> Result<CaseReport, TaskDriverError>,
) -> TaskOutcome {
    let mut evidence = Vec::new();
    let mut failures = Vec::new();
    for case in cases {
        match probe(case) {
            Ok(result) => {
                evidence.push(format!(
                    "{case}: {}",
                    if result.passed { "pass" } else { "fail" }
                ));
                evidence.extend(result.evidence);
                if !result.passed {
                    failures.extend(result.failures);
                }
            }
            Err(_) => failures.push(format!("{case}: driver error (no payload logged)")),
        }
    }
    let evidence = crate::bound_evidence(evidence);
    if failures.is_empty() {
        TaskOutcome::Pass { evidence }
    } else {
        TaskOutcome::Fail {
            where_: id.to_owned(),
            how: failures.join("; "),
            evidence,
        }
    }
}

/// Run one bounded in-memory probe; unknown names return a driver error.
/// Fixtures contain at most two calls/messages and less than 1 KiB of input.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    let index = case_index(case, &CASES)?;
    let (messages, tools, required) = match index {
        0 => (
            vec![json!({"role": "user", "content": "ordinary request"})],
            vec![],
            "ordinary request",
        ),
        1 => (
            vec![json!({"role": "user", "content": "hello"})],
            vec![json!({"type": "function", "function": {"name": "file_read"}})],
            "file_read",
        ),
        2 => (
            vec![json!({"role": "user", "content": format!("Authorization: Bearer {CANARY}")})],
            vec![],
            "",
        ),
        _ => (
            vec![json!({"role": "user", "content": "hello"})],
            vec![json!({
                "type": "function", "function": {"name": "request", "parameters": {
                    "type": "object", "properties": {"api_key": {"type": "string", "default": CANARY}}
                }}
            })],
            "",
        ),
    };
    let result = build_chat_payload(
        &phlow_config::OllamaConfig::default(),
        &messages,
        &tools,
        None,
    );
    let (passed, output_bytes) = match result {
        Ok(payload) => {
            let output = serde_json::Value::Object(payload).to_string();
            (
                !output.contains(CANARY) && output.contains(required),
                output.len(),
            )
        }
        Err(_) => (index >= 2, 0),
    };
    Ok(report(
        CASES[index],
        passed,
        output_bytes,
        "build_chat_payload",
    ))
}

/// Run all four cases, retaining only boolean/count evidence, never payloads.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, run_case)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(index: usize) {
        let result = run_case(CASES[index]).expect("bounded fixture must execute");
        assert!(result.passed, "{}", result.failures.join("; "));
    }

    #[test]
    fn ordinary_message_preserved() {
        check(0);
    }

    #[test]
    fn ordinary_schema_preserved() {
        check(1);
    }

    #[test]
    fn message_credential_blocked() {
        check(2);
    }

    #[test]
    fn schema_default_credential_blocked() {
        check(3);
    }
}
