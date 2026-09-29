//! task-243: bounded hook spawn storm.
//!
//! Honest scope: Real HookManager children hold a filesystem lock to update live/peak counts. At
//! most 96 firings fit the observable 128-record outcome ring. Every firing must finish or be
//! explicitly dropped; each child self-expires.

use super::task_233::storm;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-243";
/// Desired invariant.
pub const NAME: &str = "bounded hook spawn storm";
/// Executes real Rust runtime seams.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "single_spawn",
    "small_fanout",
    "storm_shed_and_bounded",
    "crashing_storm_bounded",
];

/// Execute all cases, retaining assertion failures separately from fixture errors.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, CASES.iter().enumerate().map(|(i, c)| (*c, case(i))))
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => storm(1, false),
        1 => storm(4, false),
        2 => storm(96, false),
        3 => storm(96, true),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn single_spawn() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn small_fanout() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn storm_shed_and_bounded() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn crashing_storm_bounded() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
