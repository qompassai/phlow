//! task-210: minimal permission proposals.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. `Request::from_json` is the proposal
//! admission seam; it must reject unsupported extra authority (a `permissions` list or a `tools`
//! list beside the single `tool`) instead of silently dropping it.

use phlow_approval::{Error, Request};
use serde_json::{Value, json};

use super::task_209::{pending, record, req_json, run_cases};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-210";
/// Desired permission invariant.
pub const NAME: &str = "minimal permission proposals";
/// Drives the phlow-approval admission seam directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "single_path_retained",
    "single_tool_retained",
    "extra_permissions_rejected",
    "extra_tools_rejected",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => {
            let (queue, id) = pending()?;
            Ok(record(&queue, &id)?.scope.paths() == ["/work/a"])
        }
        1 => {
            let (queue, id) = pending()?;
            Ok(record(&queue, &id)?.scope.tool() == "fs.write")
        }
        2 => Ok(extra_rejected(
            "permissions",
            json!(["fs.write", "process.spawn"]),
        )),
        3 => Ok(extra_rejected("tools", json!(["fs.write", "fs.delete"]))),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

fn extra_rejected(name: &str, value: Value) -> bool {
    let mut input = req_json();
    input[name] = value;
    matches!(Request::from_json(&input), Err(Error::UnknownField { field, .. }) if field == name)
}

#[cfg(test)]
mod tests {
    #[test]
    fn single_path_retained() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn single_tool_retained() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn extra_permissions_rejected() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn extra_tools_rejected() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
