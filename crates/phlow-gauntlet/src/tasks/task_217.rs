//! task-217: conflicting policy rules deny.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. `decide` takes the most restrictive
//! matching rule, so an allow/deny conflict on the same scope denies regardless of rule order.

use phlow_approval::{Verdict, decide};
use serde_json::json;

use super::task_209::{policy, policy_of, req, rule, run_cases};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-217";
/// Desired permission invariant.
pub const NAME: &str = "conflicting policy rules deny";
/// Drives the phlow-approval policy seam directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "single_allow_works",
    "single_deny_works",
    "allow_then_deny_denies",
    "deny_then_allow_denies",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    let scope = req()?.scope().clone();
    let (chosen, expected) = match index {
        0 => (policy("allow")?, Verdict::Allow),
        1 => (policy("deny")?, Verdict::Deny),
        2 => (
            policy_of(json!([rule("allow"), rule("deny")]))?,
            Verdict::Deny,
        ),
        3 => (
            policy_of(json!([rule("deny"), rule("allow")]))?,
            Verdict::Deny,
        ),
        _ => return Err("case index outside fixed array".to_owned()),
    };
    Ok(decide(Some(&chosen), &scope).verdict == expected)
}

#[cfg(test)]
mod tests {
    #[test]
    fn single_allow_works() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn single_deny_works() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn allow_then_deny_denies() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn deny_then_allow_denies() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
