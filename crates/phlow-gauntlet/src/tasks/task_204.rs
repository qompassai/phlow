//! task-204: tool argument credential opacity.
//!
//! Drives the production `normalize_tool_calls` public API.
//!
//! Honest scope: no issued credential handles or public broker resolver exist.
//! These tests pin the prerequisite that normalized tool arguments cannot expose
//! raw credentials, including JSON embedded inside a string. Ordinary arguments
//! must survive. They cannot prove that every public API refuses dereferencing
//! future handles; add broker-issued-handle probes when that API exists.

use super::task_201::{CANARY, case_index, report, run_cases};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use phlow_runtime::normalize_tool_calls;
use serde_json::json;

/// Stable task identifier.
pub const ID: &str = "task-204";
/// Human-readable invariant.
pub const NAME: &str = "tool argument credential opacity";
/// Offline Rust seams; no network, subprocess, or Neovim required.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation controls, then two adversarial credential probes.
pub const CASES: [&str; 4] = [
    "ordinary_object_arguments_preserved",
    "ordinary_string_arguments_preserved",
    "raw_credential_argument_blocked",
    "string_encoded_credential_blocked",
];

/// Run one bounded in-memory probe; unknown names return a driver error.
/// Fixtures contain at most two calls/messages and less than 1 KiB of input.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    let index = case_index(case, &CASES)?;
    let arguments = match index {
        0 => json!({"path": "README.md"}),
        1 => json!(r#"{"path":"README.md"}"#),
        2 => json!({"api_key": CANARY}),
        _ => json!(json!({"headers": {"Authorization": format!("Bearer {CANARY}")}}).to_string()),
    };
    let calls =
        [json!({"id": "call-1", "function": {"name": "file_read", "arguments": arguments}})];
    let (passed, output_bytes) = match normalize_tool_calls(&calls, "coder") {
        Ok(normalized) => {
            let output = serde_json::Value::Array(normalized.clone()).to_string();
            let preserved = index >= 2
                || (normalized.len() == 1 && normalized[0]["function"]["arguments"] == arguments);
            (!output.contains(CANARY) && preserved, output.len())
        }
        Err(_) => (index >= 2, 0),
    };
    Ok(report(
        CASES[index],
        passed,
        output_bytes,
        "normalize_tool_calls arguments",
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
    fn ordinary_object_arguments_preserved() {
        check(0);
    }

    #[test]
    fn ordinary_string_arguments_preserved() {
        check(1);
    }

    #[test]
    fn raw_credential_argument_blocked() {
        check(2);
    }

    #[test]
    fn string_encoded_credential_blocked() {
        check(3);
    }
}
