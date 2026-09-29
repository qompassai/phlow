//! task-203: tool result credential containment.
//!
//! Drives `ConversationContext::add_message` and its public snapshot.
//!
//! Honest scope: runtime `tool_turn` is private; this probes the public agent
//! context store with actual tool-result text, not a mocked runtime. There is
//! no known-secret registration API yet. The desired boundary recognizes the
//! synthetic API-key pattern and redacts or drops it before context retention.
//! Passing this task alone would not prove runtime tool dispatch is protected.

use super::task_201::{CANARY, case_index, report, run_cases};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use phlow_agent::ConversationContext;
use serde_json::json;

/// Stable task identifier.
pub const ID: &str = "task-203";
/// Human-readable invariant.
pub const NAME: &str = "tool result credential containment";
/// Offline Rust seams; no network, subprocess, or Neovim required.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation controls, then two adversarial credential probes.
pub const CASES: [&str; 4] = [
    "successful_tool_result_preserved",
    "ordinary_tool_error_preserved",
    "successful_tool_secret_removed",
    "nested_tool_error_secret_removed",
];

/// Run one bounded in-memory probe; unknown names return a driver error.
/// Fixtures contain at most two calls/messages and less than 1 KiB of input.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    let index = case_index(case, &CASES)?;
    let value = match index {
        0 => json!({"status": "ok", "content": "ordinary file"}),
        1 => json!({"status": "error", "error": "file missing"}),
        2 => json!({"status": "ok", "content": format!("api_key={CANARY}")}),
        _ => {
            json!({"status": "error", "error": {"stderr": format!("Authorization: Bearer {CANARY}")}})
        }
    };
    let input = value.to_string();
    let mut context = ConversationContext::new(2);
    context.add_message("tool", &input);
    let snapshot = context.messages();
    let clean = snapshot
        .iter()
        .all(|message| !message.content.contains(CANARY));
    let preserved = index >= 2
        || (snapshot.len() == 1 && snapshot[0].role == "tool" && snapshot[0].content == input);
    let output_bytes = snapshot.iter().map(|message| message.content.len()).sum();
    Ok(report(
        CASES[index],
        clean && preserved,
        output_bytes,
        "ConversationContext tool result",
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
    fn successful_tool_result_preserved() {
        check(0);
    }

    #[test]
    fn ordinary_tool_error_preserved() {
        check(1);
    }

    #[test]
    fn successful_tool_secret_removed() {
        check(2);
    }

    #[test]
    fn nested_tool_error_secret_removed() {
        check(3);
    }
}
