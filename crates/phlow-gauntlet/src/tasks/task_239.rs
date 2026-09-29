//! task-239: policy state isolation.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. A parsed `Policy` shares nothing with
//! its input document and has no mutating API; a direct rule write does not compile (proved by
//! the crate's `compile_fail` doctest, which this runtime case cannot express). At runtime the
//! only write an agent can make is replacing its own copy, which must not reach the operator's
//! policy. This does not claim OS isolation.

use phlow_approval::{Policy, Verdict, decide};
use serde_json::json;

use super::task_209::{policy, req, rule, run_cases};
use super::task_233::err;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-239";
/// Desired invariant.
pub const NAME: &str = "policy state isolation";
/// Drives the phlow-approval policy parser and seam directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "deny_default",
    "explicit_allow",
    "input_alias_cannot_grant",
    "agent_write_refused",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    let scope = req()?.scope().clone();
    match index {
        0 => {
            let empty = Policy::from_json(&json!({ "version": 1 })).map_err(err)?;
            Ok(decide(Some(&empty), &scope).verdict == Verdict::Deny)
        }
        1 => Ok(decide(Some(&policy("allow")?), &scope).verdict == Verdict::Allow),
        2 => {
            let mut input = json!({ "version": 1, "rules": [rule("deny")] });
            let parsed = Policy::from_json(&input).map_err(err)?;
            input["rules"][0]["decision"] = json!("allow");
            Ok(decide(Some(&parsed), &scope).verdict == Verdict::Deny)
        }
        3 => {
            let operator = policy("deny")?;
            let mut agent_copy = operator.clone();
            let copy_before = decide(Some(&agent_copy), &scope).verdict;
            agent_copy = policy("allow")?;
            Ok(copy_before == Verdict::Deny
                && decide(Some(&agent_copy), &scope).verdict == Verdict::Allow
                && decide(Some(&operator), &scope).verdict == Verdict::Deny)
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn deny_default() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn explicit_allow() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn input_alias_cannot_grant() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn agent_write_refused() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
