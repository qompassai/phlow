//! task-224: partial approvals cannot combine authority.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. No grant combiner exists: `decide`
//! authorizes a compound request only when one rule covers its entire scope, so two separately
//! granted paths do not add up to a grant for both.

use phlow_approval::Verdict;
use serde_json::json;

use super::task_209::{req_json, req_with, run_cases, split_policy, verdict_for};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-224";
/// Desired permission invariant.
pub const NAME: &str = "partial approvals cannot combine authority";
/// Drives the phlow-approval policy seam directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "first_scope_allowed",
    "second_scope_allowed",
    "union_of_scopes_denied",
    "union_with_unapproved_scope_denied",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    let split = split_policy()?;
    let (request, expected) = match index {
        0 => (req_json(), Verdict::Allow),
        1 => (req_with(json!({ "paths": ["/work/b"] }))?, Verdict::Allow),
        2 => (
            req_with(json!({ "paths": ["/work/a", "/work/b"] }))?,
            Verdict::Deny,
        ),
        3 => (
            req_with(json!({ "paths": ["/work/a", "/work/b", "/work/c"] }))?,
            Verdict::Deny,
        ),
        _ => return Err("case index outside fixed array".to_owned()),
    };
    Ok(verdict_for(Some(&split), &request)? == expected)
}

#[cfg(test)]
mod tests {
    #[test]
    fn first_scope_allowed() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn second_scope_allowed() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn union_of_scopes_denied() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn union_with_unapproved_scope_denied() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
