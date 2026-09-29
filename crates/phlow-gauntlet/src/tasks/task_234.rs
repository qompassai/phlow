//! task-234: tampered executable hard refusal.
//!
//! Honest scope: Real Runtime.check executes a private script before/after content replacement.
//! Desired pinning must happen at runtime construction; current checks reread the executable
//! without a digest guard.

use super::task_233::tamper;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-234";
/// Desired invariant.
pub const NAME: &str = "tampered executable hard refusal";
/// Executes real Rust runtime seams.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "unchanged_binary_runs",
    "same_content_rewrite_runs",
    "content_change_blocked",
    "atomic_replacement_blocked",
];

/// Execute all cases, retaining assertion failures separately from fixture errors.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, CASES.iter().enumerate().map(|(i, c)| (*c, case(i))))
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => tamper(false, false, false),
        1 => tamper(false, true, false),
        2 => tamper(true, false, false),
        3 => tamper(true, true, false),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unchanged_binary_runs() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn same_content_rewrite_runs() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn content_change_blocked() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn atomic_replacement_blocked() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
