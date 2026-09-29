//! task-216: policy version pinning.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. `version` is required; version 1 is
//! the only implemented schema and is retained on the parsed policy. Missing or unknown versions
//! are rejected even when the rest of the document is valid.

use phlow_approval::{Error, POLICY_VERSION, Policy};
use phlow_json::JsonError;
use serde_json::json;

use super::task_209::run_cases;
use super::task_233::err;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-216";
/// Desired permission invariant.
pub const NAME: &str = "policy version pinning";
/// Drives the phlow-approval policy parser directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "version_one_accepted",
    "version_one_retained",
    "future_version_rejected",
    "missing_version_rejected",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => Ok(Policy::from_json(&json!({ "version": 1, "default": "deny" })).is_ok()),
        1 => {
            let parsed = Policy::from_json(&json!({ "version": 1, "default": "deny" }));
            Ok(parsed.map_err(err)?.version() == 1 && POLICY_VERSION == 1)
        }
        2 => {
            let parsed = Policy::from_json(&json!({ "version": 999, "default": "allow" }));
            Ok(matches!(
                parsed,
                Err(Error::UnsupportedVersion { version: 999 })
            ))
        }
        3 => {
            let parsed = Policy::from_json(&json!({ "default": "allow" }));
            Ok(matches!(
                parsed,
                Err(Error::Json(JsonError::MissingField { field })) if field == "version"
            ))
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_one_accepted() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn version_one_retained() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn future_version_rejected() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn missing_version_rejected() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
