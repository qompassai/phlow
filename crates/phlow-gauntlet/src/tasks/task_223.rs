//! task-223: policy load failure denies all.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. A parse failure yields no policy, and
//! `decide(None, ..)` denies; no loader fallback is invented. `false` and map-shaped rules must
//! not normalize into a permissive policy, even with an allow default.

use phlow_approval::{Policy, Verdict, decide};
use serde_json::{Value, json};

use super::task_209::{req, rule, run_cases};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-223";
/// Desired permission invariant.
pub const NAME: &str = "policy load failure denies all";
/// Drives the phlow-approval policy parser and seam directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "missing_policy_denies",
    "invalid_default_denies",
    "false_rules_fail_closed",
    "map_rules_fail_closed",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => Ok(decide(None, req()?.scope()).verdict == Verdict::Deny),
        1 => fails_closed(json!({ "version": 1, "default": "invalid" })),
        2 => fails_closed(json!({ "version": 1, "default": "allow", "rules": false })),
        3 => fails_closed(json!({
            "version": 1,
            "default": "allow",
            "rules": { "hidden": rule("deny") },
        })),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

fn fails_closed(document: Value) -> Result<bool, String> {
    let parsed = Policy::from_json(&document);
    let verdict = decide(parsed.as_ref().ok(), req()?.scope()).verdict;
    Ok(parsed.is_err() && verdict == Verdict::Deny)
}

#[cfg(test)]
mod tests {
    #[test]
    fn missing_policy_denies() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn invalid_default_denies() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn false_rules_fail_closed() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn map_rules_fail_closed() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
