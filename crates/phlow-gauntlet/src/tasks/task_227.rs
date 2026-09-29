//! task-227: byte deterministic permission delta.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. Only the queue-computed delta's
//! canonical `PermissionDelta::to_json` encoding is serialized; approval IDs and timestamps are
//! outside the delta contract.

use serde_json::{Value, json};

use super::task_209::run_cases;
use super::task_225::{delta_of, is_delta};
use super::task_233::err;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-227";
/// Desired invariant.
pub const NAME: &str = "byte deterministic permission delta";
/// Drives the phlow-approval queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "identical_inputs_identical_bytes",
    "sorted_additions",
    "permutation_invariant",
    "duplicates_are_set_members",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => {
            let first = delta_bytes(json!(["a"]), json!(["b"]))?;
            let second = delta_bytes(json!(["a"]), json!(["b"]))?;
            Ok(first == second && first == br#"{"added":["b"],"removed":["a"]}"#)
        }
        1 => Ok(is_delta(
            &delta_of(json!([]), json!(["z", "a"]))?,
            &["a", "z"],
            &[],
        )),
        2 => Ok(delta_bytes(json!([]), json!(["b", "a"]))?
            == delta_bytes(json!([]), json!(["a", "b"]))?),
        3 => Ok(is_delta(
            &delta_of(json!(["a", "a"]), json!(["b", "b"]))?,
            &["b"],
            &["a"],
        )),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

fn delta_bytes(before: Value, after: Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&delta_of(before, after)?.to_json()).map_err(err)
}

#[cfg(test)]
mod tests {
    #[test]
    fn identical_inputs_identical_bytes() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn sorted_additions() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn permutation_invariant() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn duplicates_are_set_members() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
