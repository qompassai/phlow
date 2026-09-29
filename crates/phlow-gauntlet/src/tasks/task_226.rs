//! task-226: permission delta set edges.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. The queue must calculate true set
//! differences, not compare cardinality or summaries. No test-local delta implementation is used.

use serde_json::{Value, json};

use super::task_209::run_cases;
use super::task_225::{delta_of, is_delta};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-226";
/// Desired invariant.
pub const NAME: &str = "permission delta set edges";
/// Drives the phlow-approval queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "add_only",
    "remove_only",
    "rename_remove_add",
    "equal_size_disjoint",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    let (before, after, added, removed): (Value, Value, &[&str], &[&str]) = match index {
        0 => (json!(["a"]), json!(["a", "b"]), &["b"], &[]),
        1 => (json!(["a", "b"]), json!(["b"]), &[], &["a"]),
        2 => (json!(["old"]), json!(["new"]), &["new"], &["old"]),
        3 => (
            json!(["a", "b"]),
            json!(["c", "d"]),
            &["c", "d"],
            &["a", "b"],
        ),
        _ => return Err("case index outside fixed array".to_owned()),
    };
    Ok(is_delta(&delta_of(before, after)?, added, removed))
}

#[cfg(test)]
mod tests {
    #[test]
    fn add_only() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn remove_only() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn rename_remove_add() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn equal_size_disjoint() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
