//! task-225: approval permission delta.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. The queue computes
//! `permission_delta` from the admitted `permissions_before`/`permissions_after` sets and exposes
//! it, with sorted `added`/`removed` lists, on every record. This file also hosts the shared delta
//! fixtures for task-226..232.

use phlow_approval::PermissionDelta;
use serde_json::{Value, json};

use super::task_209::{pending_from, record, req_with, run_cases};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-225";
/// Desired invariant.
pub const NAME: &str = "approval permission delta";
/// Drives the phlow-approval queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "empty_to_empty",
    "unchanged_permission",
    "addition_visible",
    "removal_visible",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    let (before, after, added, removed): (Value, Value, &[&str], &[&str]) = match index {
        0 => (json!([]), json!([]), &[], &[]),
        1 => (json!(["fs.read"]), json!(["fs.read"]), &[], &[]),
        2 => (json!([]), json!(["fs.write"]), &["fs.write"], &[]),
        3 => (json!(["fs.write"]), json!([]), &[], &["fs.write"]),
        _ => return Err("case index outside fixed array".to_owned()),
    };
    Ok(is_delta(&delta_of(before, after)?, added, removed))
}

// Shared delta fixtures: request construction only; the delta is always read back from the queue.

/// Admit the baseline request with `extra` fields; return the queue-computed delta.
pub(super) fn delta_with(extra: Value) -> Result<PermissionDelta, String> {
    let (queue, id) = pending_from(&req_with(extra)?)?;
    Ok(record(&queue, &id)?.permission_delta)
}

pub(super) fn delta_of(before: Value, after: Value) -> Result<PermissionDelta, String> {
    delta_with(json!({ "permissions_before": before, "permissions_after": after }))
}

pub(super) fn is_delta(delta: &PermissionDelta, added: &[&str], removed: &[&str]) -> bool {
    delta.added == added && delta.removed == removed
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_to_empty() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn unchanged_permission() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn addition_visible() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn removal_visible() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
