//! task-205: credential-safe run reports.
//!
//! Drives `phlow_runtime::new_report`, the real run-report constructor.
//!
//! Honest scope: the runtime has tool events but no credential materialization
//! ledger. These probes assert that report/log output cannot retain credential
//! values from task text, even across escaped newlines. They do not claim that
//! substitutions emit what/where/when/authorized-by records: no substitution
//! occurs here, so inventing such an event would be fabricated evidence.
//! A future broker needs a separate per-materialization ledger integration test.

use super::task_201::{CANARY, case_index, report, run_cases};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use phlow_runtime::new_report;
use serde_json::json;

/// Stable task identifier.
pub const ID: &str = "task-205";
/// Human-readable invariant.
pub const NAME: &str = "credential-safe run reports";
/// Offline Rust seams; no network, subprocess, or Neovim required.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation controls, then two adversarial credential probes.
pub const CASES: [&str; 4] = [
    "ordinary_task_report_preserved",
    "empty_task_report_preserved",
    "credential_in_report_blocked",
    "multiline_credential_in_report_blocked",
];

/// Run one bounded in-memory probe; unknown names return a driver error.
/// Fixtures contain at most two calls/messages and less than 1 KiB of input.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    let index = case_index(case, &CASES)?;
    let task = match index {
        0 => "inspect README.md".to_owned(),
        1 => String::new(),
        2 => format!("inspect with api_key={CANARY}"),
        _ => format!("diagnostic\nAuthorization: Bearer {CANARY}\ncontinue"),
    };
    let value = new_report(&task);
    let output = value.to_string();
    let preserved = index >= 2
        || (value["task"] == json!(task)
            && value["events"] == json!([])
            && value["tool_calls"] == json!(0));
    Ok(report(
        CASES[index],
        !output.contains(CANARY) && preserved,
        output.len(),
        "new_report serialized agent/operator output",
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
    fn ordinary_task_report_preserved() {
        check(0);
    }

    #[test]
    fn empty_task_report_preserved() {
        check(1);
    }

    #[test]
    fn credential_in_report_blocked() {
        check(2);
    }

    #[test]
    fn multiline_credential_in_report_blocked() {
        check(3);
    }
}
