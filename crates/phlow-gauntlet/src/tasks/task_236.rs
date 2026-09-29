//! task-236: canonical path substitution resistance.
//!
//! Honest scope: Real symlinks are switched after Runtime construction. Both different-byte and
//! identical-byte targets must be refused when their canonical paths differ; no global PATH
//! mutation occurs.

use super::task_233::swap_link;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-236";
/// Desired invariant.
pub const NAME: &str = "canonical path substitution resistance";
/// Executes real Rust runtime seams.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "stable_link_runs",
    "same_target_relinked_runs",
    "different_target_blocked",
    "same_digest_different_path_blocked",
];

/// Execute all cases, retaining assertion failures separately from fixture errors.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, CASES.iter().enumerate().map(|(i, c)| (*c, case(i))))
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => swap_link(false, false),
        1 => swap_link(false, true),
        2 => swap_link(true, false),
        3 => swap_link(true, true),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn stable_link_runs() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn same_target_relinked_runs() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn different_target_blocked() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn same_digest_different_path_blocked() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
