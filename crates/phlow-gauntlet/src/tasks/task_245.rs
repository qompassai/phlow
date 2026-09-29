//! task-245: timeout propagation through process groups.
//!
//! Honest scope: Real Python checks fork descendants and write ready/late markers. Timeout must
//! kill the whole group before the late marker. Descendants self-expire within one second even on
//! failure; this is process-group coverage, not setsid escape containment.

use super::task_233::group;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-245";
/// Desired invariant.
pub const NAME: &str = "timeout propagation through process groups";
/// Executes real Rust runtime seams.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "normal_group_completes",
    "normal_two_descendants_complete",
    "timeout_kills_descendant",
    "timeout_kills_sibling_descendants",
];

/// Execute all cases, retaining assertion failures separately from fixture errors.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, CASES.iter().enumerate().map(|(i, c)| (*c, case(i))))
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => group(false, 1),
        1 => group(false, 2),
        2 => group(true, 1),
        3 => group(true, 2),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn normal_group_completes() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn normal_two_descendants_complete() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn timeout_kills_descendant() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn timeout_kills_sibling_descendants() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
