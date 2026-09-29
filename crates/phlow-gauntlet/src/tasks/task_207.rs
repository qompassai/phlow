//! task-207: credential reuse across tool scopes.
//!
//! Drives the real tool-call normalizer repeatedly with distinct role prefixes.
//!
//! Honest scope: normalization is stateless and there are no broker-issued
//! handles or consumption records. Duplicate-call-ID rejection is a control,
//! not proof of single-use credentials. The red probes require raw credential
//! arguments never to become reusable authority on a second normalization or
//! across coder/reviewer scopes. Future handle reuse needs broker-issued data;
//! this test deliberately does not invent a handle string or fake a broker.

use super::task_201::{CANARY, case_index, report, run_cases};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use phlow_runtime::normalize_tool_calls;
use serde_json::json;

/// Stable task identifier.
pub const ID: &str = "task-207";
/// Human-readable invariant.
pub const NAME: &str = "credential reuse across tool scopes";
/// Offline Rust seams; no network, subprocess, or Neovim required.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation controls, then two adversarial credential probes.
pub const CASES: [&str; 4] = [
    "ordinary_calls_in_distinct_scopes",
    "duplicate_call_ids_refused",
    "credential_replay_same_scope_blocked",
    "credential_replay_other_scope_blocked",
];

/// Run one bounded in-memory probe; unknown names return a driver error.
/// Fixtures contain at most two calls/messages and less than 1 KiB of input.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    let index = case_index(case, &CASES)?;
    let arguments = if index < 2 {
        json!({"path": "README.md"})
    } else {
        json!({"api_key": CANARY})
    };
    let call = json!({"id": "call-1", "function": {"name": "file_read", "arguments": arguments}});
    if index == 1 {
        let result = normalize_tool_calls(&[call.clone(), call], "coder");
        return Ok(report(
            CASES[index],
            result.is_err(),
            0,
            "normalize_tool_calls duplicate IDs",
        ));
    }
    let first = normalize_tool_calls(&[call], "coder");
    let (passed, output_bytes) = match first {
        Ok(normalized) => {
            let scope = if index == 2 { "coder" } else { "reviewer" };
            match normalize_tool_calls(&normalized, scope) {
                Ok(replayed) => {
                    let output = serde_json::Value::Array(replayed.clone()).to_string();
                    let preserved = index >= 2
                        || (replayed.len() == 1
                            && replayed[0]["function"]["arguments"] == arguments);
                    (!output.contains(CANARY) && preserved, output.len())
                }
                Err(_) => (index >= 2, 0),
            }
        }
        Err(_) => (index >= 2, 0),
    };
    Ok(report(
        CASES[index],
        passed,
        output_bytes,
        "normalize_tool_calls scope replay",
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
    fn ordinary_calls_in_distinct_scopes() {
        check(0);
    }

    #[test]
    fn duplicate_call_ids_refused() {
        check(1);
    }

    #[test]
    fn credential_replay_same_scope_blocked() {
        check(2);
    }

    #[test]
    fn credential_replay_other_scope_blocked() {
        check(3);
    }
}
