//! task-218: wildcard scope escalation.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. An allow rule's path and endpoint
//! lists are exact-match grants: every requested resource must be listed, so one matched item
//! cannot carry an unmatched one. No glob syntax exists.

use phlow_approval::Verdict;
use serde_json::json;

use super::task_209::{policy, policy_of, req_json, req_with, rule, run_cases, verdict_for};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-218";
/// Desired permission invariant.
pub const NAME: &str = "wildcard scope escalation";
/// Drives the phlow-approval policy seam directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "exact_path_allowed",
    "unmatched_path_denied",
    "mixed_paths_denied",
    "mixed_endpoints_denied",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    let allow = policy("allow")?;
    match index {
        0 => Ok(verdict_for(Some(&allow), &req_json())? == Verdict::Allow),
        1 => {
            let request = req_with(json!({ "paths": ["/work/b"] }))?;
            Ok(verdict_for(Some(&allow), &request)? == Verdict::Deny)
        }
        2 => {
            let request = req_with(json!({ "paths": ["/work/a", "/work/b"] }))?;
            Ok(verdict_for(Some(&allow), &request)? == Verdict::Deny)
        }
        3 => {
            let mut grant = rule("allow");
            grant["endpoints"] = json!(["https://allowed.invalid"]);
            let granted = policy_of(json!([grant]))?;
            let only = req_with(json!({ "endpoints": ["https://allowed.invalid"] }))?;
            let mixed = req_with(json!({
                "endpoints": ["https://allowed.invalid", "https://other.invalid"],
            }))?;
            Ok(verdict_for(Some(&granted), &only)? == Verdict::Allow
                && verdict_for(Some(&granted), &mixed)? == Verdict::Deny)
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn exact_path_allowed() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn unmatched_path_denied() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn mixed_paths_denied() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn mixed_endpoints_denied() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
