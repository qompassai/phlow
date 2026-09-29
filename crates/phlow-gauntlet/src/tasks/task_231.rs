//! task-231: permission identity preservation.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. Permission identity is byte-exact:
//! a case change or a scope change is a revocation plus a grant, never an unchanged permission.

use serde_json::{Value, json};

use super::task_209::run_cases;
use super::task_225::{delta_of, is_delta};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-231";
/// Desired invariant.
pub const NAME: &str = "permission identity preservation";
/// Drives the phlow-approval queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "exact_scope_unchanged",
    "independent_scopes",
    "case_change_is_change",
    "scope_widening_visible",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    let (before, after, added, removed): (Value, Value, &[&str], &[&str]) = match index {
        0 => (
            json!(["fs.read:/work/a"]),
            json!(["fs.read:/work/a"]),
            &[],
            &[],
        ),
        1 => (
            json!([]),
            json!(["fs.read:/work/a", "fs.read:/work/b"]),
            &["fs.read:/work/a", "fs.read:/work/b"],
            &[],
        ),
        2 => (json!(["read"]), json!(["READ"]), &["READ"], &["read"]),
        3 => (
            json!(["fs.read:/work/a"]),
            json!(["fs.read:/work"]),
            &["fs.read:/work"],
            &["fs.read:/work/a"],
        ),
        _ => return Err("case index outside fixed array".to_owned()),
    };
    Ok(is_delta(&delta_of(before, after)?, added, removed))
}

#[cfg(test)]
mod tests {
    #[test]
    fn exact_scope_unchanged() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn independent_scopes() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn case_change_is_change() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn scope_widening_visible() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
