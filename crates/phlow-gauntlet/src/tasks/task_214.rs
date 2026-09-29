//! task-214: unknown policy fields fail closed.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. Drives `Policy::from_json` directly:
//! construction must reject, by name, any top-level or rule key outside the closed schema rather
//! than silently discarding it.

use phlow_approval::{Error, Policy, Verdict, decide};
use serde_json::{Value, json};

use super::task_209::{policy, req, run_cases};
use super::task_233::err;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-214";
/// Desired permission invariant.
pub const NAME: &str = "unknown policy fields fail closed";
/// Drives the phlow-approval policy parser directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "empty_policy_denies",
    "known_rule_accepted",
    "unknown_top_level_rejected",
    "unknown_rule_field_rejected",
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
        2 => Ok(unknown_rejected(
            json!({ "version": 1, "default": "allow", "unknown_permission": true }),
            "unknown_permission",
        )),
        3 => Ok(unknown_rejected(
            json!({ "version": 1, "rules": [{
                "risk": "local_reversible",
                "decision": "allow",
                "tools": ["fs.write"],
                "typo_paths": ["/work/a"],
            }] }),
            "typo_paths",
        )),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

fn unknown_rejected(document: Value, name: &str) -> bool {
    matches!(Policy::from_json(&document), Err(Error::UnknownField { field, .. }) if field == name)
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_policy_denies() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn known_rule_accepted() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn unknown_top_level_rejected() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn unknown_rule_field_rejected() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
