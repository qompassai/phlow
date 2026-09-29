//! task-202: agent-visible environment scrubbing.
//!
//! Drives the real chat payload builder with serialized environment context.
//!
//! Honest scope: there is no agent environment-map builder or broker registry.
//! The closest public boundary is environment data supplied as message content.
//! A future scrubber must preserve ordinary settings while removing credential
//! values before this boundary. No process environment is read or changed, and
//! this does not claim coverage of child-process environment inheritance.

use super::task_201::{CANARY, case_index, report, run_cases};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use phlow_llm::build_chat_payload;
use serde_json::json;

/// Stable task identifier.
pub const ID: &str = "task-202";
/// Human-readable invariant.
pub const NAME: &str = "agent-visible environment scrubbing";
/// Offline Rust seams; no network, subprocess, or Neovim required.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation controls, then two adversarial credential probes.
pub const CASES: [&str; 4] = [
    "ordinary_environment_preserved",
    "empty_environment_preserved",
    "api_key_environment_scrubbed",
    "mixed_environment_secret_scrubbed",
];

/// Run one bounded in-memory probe; unknown names return a driver error.
/// Fixtures contain at most two calls/messages and less than 1 KiB of input.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    let index = case_index(case, &CASES)?;
    let environment = match index {
        0 => json!({"LANG": "C.UTF-8", "TERM": "xterm"}),
        1 => json!({}),
        2 => json!({"OPENAI_API_KEY": CANARY}),
        _ => json!({"LANG": "C.UTF-8", "SERVICE_TOKEN": CANARY, "AUTHORIZATION": CANARY}),
    };
    let messages = [json!({"role": "system", "content": environment.to_string()})];
    let result = build_chat_payload(&phlow_config::OllamaConfig::default(), &messages, &[], None);
    let (passed, output_bytes) = match result {
        Ok(payload) => {
            let content = payload["messages"][0]["content"]
                .as_str()
                .unwrap_or_default();
            let preserved = match index {
                0 => content.contains("C.UTF-8") && content.contains("xterm"),
                1 => content == "{}",
                3 => content.contains("C.UTF-8"),
                _ => true,
            };
            (!content.contains(CANARY) && preserved, content.len())
        }
        Err(_) => (index >= 2, 0),
    };
    Ok(report(
        CASES[index],
        passed,
        output_bytes,
        "build_chat_payload environment content",
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
    fn ordinary_environment_preserved() {
        check(0);
    }

    #[test]
    fn empty_environment_preserved() {
        check(1);
    }

    #[test]
    fn api_key_environment_scrubbed() {
        check(2);
    }

    #[test]
    fn mixed_environment_secret_scrubbed() {
        check(3);
    }
}
