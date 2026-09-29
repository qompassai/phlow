//! task-235: digest refusal cannot become warning.
//!
//! Honest scope: The trusted and optional-check paths exercise the real runtime. A changed
//! executable must have an error status and no marker side effect even when optional; a failed exit
//! alone is insufficient.

use super::task_233::tamper;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-235";
/// Desired invariant.
pub const NAME: &str = "digest refusal cannot become warning";
/// Executes real Rust runtime seams.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "trusted_unchanged_runs",
    "optional_unchanged_runs",
    "trusted_tamper_hard_error",
    "optional_tamper_hard_error",
];

/// Execute all cases, retaining assertion failures separately from fixture errors.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, CASES.iter().enumerate().map(|(i, c)| (*c, case(i))))
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => tamper(false, false, false),
        1 => tamper(false, false, true),
        2 => tamper(true, false, false),
        3 => tamper(true, false, true),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn trusted_unchanged_runs() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn optional_unchanged_runs() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn trusted_tamper_hard_error() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn optional_tamper_hard_error() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
