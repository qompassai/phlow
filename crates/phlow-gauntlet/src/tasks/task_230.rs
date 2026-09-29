//! task-230: bounded permission delta.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. Admission must accept 4096 unique
//! permissions (`PERMISSIONS_MAX`) and compute their delta within one second, and must reject
//! malformed sets: non-string members, and sparse tables (a JSON object standing in for an array,
//! or an array with `null` holes).

use std::time::{Duration, Instant};

use phlow_approval::{PERMISSIONS_MAX, Request};
use serde_json::{Value, json};

use super::task_209::{req_with, run_cases};
use super::task_225::{delta_of, is_delta};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-230";
/// Desired invariant.
pub const NAME: &str = "bounded permission delta";
/// Drives the phlow-approval admission seam and queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "single_permission",
    "large_valid_set",
    "non_string_member_rejected",
    "sparse_set_rejected",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => Ok(is_delta(&delta_of(json!([]), json!(["x"]))?, &["x"], &[])),
        1 => large_valid_set(),
        2 => rejected(json!([false])),
        3 => Ok(rejected(json!({ "2": "root" }))? && rejected(json!([null, "root"]))?),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

fn large_valid_set() -> Result<bool, String> {
    const DEADLINE: Duration = Duration::from_secs(1);
    let after: Vec<String> = (1..=PERMISSIONS_MAX)
        .map(|i| format!("permission-{i:04}"))
        .collect();
    let started = Instant::now();
    let delta = delta_of(json!([]), json!(after))?;
    let elapsed = started.elapsed();
    Ok(PERMISSIONS_MAX == 4096
        && delta.added == after
        && delta.removed.is_empty()
        && elapsed < DEADLINE)
}

fn rejected(permissions_after: Value) -> Result<bool, String> {
    let input =
        req_with(json!({ "permissions_before": [], "permissions_after": permissions_after }))?;
    Ok(Request::from_json(&input).is_err_and(|error| !error.to_string().is_empty()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn single_permission() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn large_valid_set() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn non_string_member_rejected() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn sparse_set_rejected() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
