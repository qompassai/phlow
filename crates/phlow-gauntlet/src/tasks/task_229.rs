//! task-229: permission delta snapshots.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. Records own their before/after sets
//! and computed delta: later edits to the caller's input document or to a returned snapshot must
//! not reach the queue. Calls drive `ApprovalQueue::request`/`get` directly.

use serde_json::json;

use super::task_209::{pending_from, record, req_with, run_cases};
use super::task_225::{delta_of, is_delta};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-229";
/// Desired invariant.
pub const NAME: &str = "permission delta snapshots";
/// Drives the phlow-approval queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "input_retained",
    "two_independent_requests",
    "caller_mutation_isolated",
    "reader_mutation_isolated",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => {
            let input = req_with(json!({
                "permissions_before": ["a"],
                "permissions_after": ["a", "b"],
            }))?;
            let (queue, id) = pending_from(&input)?;
            let stored = record(&queue, &id)?;
            Ok(is_delta(&stored.permission_delta, &["b"], &[])
                && stored.permissions_before.iter().eq(["a"])
                && stored.permissions_after.iter().eq(["a", "b"]))
        }
        1 => {
            let first = delta_of(json!([]), json!(["a"]))?;
            let second = delta_of(json!([]), json!(["b"]))?;
            Ok(is_delta(&first, &["a"], &[]) && is_delta(&second, &["b"], &[]))
        }
        2 => {
            let mut input =
                req_with(json!({ "permissions_before": [], "permissions_after": ["a"] }))?;
            let (queue, id) = pending_from(&input)?;
            input["permissions_after"][0] = json!("root");
            let stored = record(&queue, &id)?;
            Ok(is_delta(&stored.permission_delta, &["a"], &[])
                && stored.permissions_after.iter().eq(["a"]))
        }
        3 => reader_mutation_isolated(),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

fn reader_mutation_isolated() -> Result<bool, String> {
    let input = req_with(json!({ "permissions_before": [], "permissions_after": ["a"] }))?;
    let (queue, id) = pending_from(&input)?;
    let mut copy = record(&queue, &id)?;
    let Some(first) = copy.permission_delta.added.first_mut() else {
        return Ok(false);
    };
    *first = "root".to_owned();
    copy.permission_delta.removed.push("a".to_owned());
    Ok(is_delta(
        &record(&queue, &id)?.permission_delta,
        &["a"],
        &[],
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn input_retained() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn two_independent_requests() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn caller_mutation_isolated() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn reader_mutation_isolated() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
