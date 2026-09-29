//! task-215: no permissive policy parsing.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. No parser mode exists: bypass flags
//! (`legacy`, `permissive`) are unknown fields and are rejected, and a malformed scope element
//! fails admission on its own, with or without such a flag.

use phlow_approval::{Error, Policy};
use serde_json::{Value, json};

use super::task_209::{policy, run_cases};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-215";
/// Desired permission invariant.
pub const NAME: &str = "no permissive policy parsing";
/// Drives the phlow-approval policy parser directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "known_default_accepted",
    "known_scope_accepted",
    "legacy_bypass_flag_rejected",
    "permissive_malformed_scope_rejected",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => Ok(Policy::from_json(&json!({ "version": 1, "default": "deny" })).is_ok()),
        1 => Ok(policy("allow").is_ok()),
        2 => {
            let document = json!({ "version": 1, "legacy": true, "default": "allow" });
            let parsed = Policy::from_json(&document);
            Ok(matches!(parsed, Err(Error::UnknownField { field, .. }) if field == "legacy"))
        }
        3 => {
            let flagged = json!({ "version": 1, "permissive": true, "rules": [malformed_rule()] });
            let plain = json!({ "version": 1, "rules": [malformed_rule()] });
            let plain_error = Policy::from_json(&plain);
            Ok(Policy::from_json(&flagged).is_err()
                && matches!(plain_error, Err(Error::InvalidValue { field: "paths", .. })))
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

fn malformed_rule() -> Value {
    json!({ "risk": "local_reversible", "decision": "allow", "tools": ["fs.write"], "paths": [42] })
}

#[cfg(test)]
mod tests {
    #[test]
    fn known_default_accepted() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn known_scope_accepted() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn legacy_bypass_flag_rejected() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn permissive_malformed_scope_rejected() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
