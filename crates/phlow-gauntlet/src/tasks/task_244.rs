//! task-244: child reaping on every exit.
//!
//! Honest scope: Actual check children print their PID; after Runtime.check returns /proc must no
//! longer contain that process. Four bounded eight-child concurrent storms cover normal, nonzero,
//! timeout and signal
//! exits. This measures direct-child reaping, not descendant adoption.

use super::task_233::reap;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-244";
/// Desired invariant.
pub const NAME: &str = "child reaping on every exit";
/// Executes real Rust runtime seams.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "normal_exit_reaped",
    "nonzero_exit_reaped",
    "timeout_children_reaped",
    "crashed_children_reaped",
];

/// Execute all cases, retaining assertion failures separately from fixture errors.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, CASES.iter().enumerate().map(|(i, c)| (*c, case(i))))
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => reap("normal"),
        1 => reap("nonzero"),
        2 => reap("timeout"),
        3 => reap("crash"),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn normal_exit_reaped() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn nonzero_exit_reaped() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn timeout_children_reaped() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn crashed_children_reaped() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
