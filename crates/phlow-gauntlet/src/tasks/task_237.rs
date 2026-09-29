//! task-237: pin removal and restoration.
//!
//! Honest scope: Runtime.check must fail closed on a removed pinned file or a changed file restored
//! at the same pathname. Positive controls establish that the executable fixture really runs.

use super::task_233::{Fixture, err, ok, tamper};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-237";
/// Desired invariant.
pub const NAME: &str = "pin removal and restoration";
/// Executes real Rust runtime seams.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "original_binary_runs",
    "identical_restoration_runs",
    "missing_executable_hard_error",
    "restored_changed_binary_refused",
];

/// Execute all cases, retaining assertion failures separately from fixture errors.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, CASES.iter().enumerate().map(|(i, c)| (*c, case(i))))
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => tamper(false, false, false),
        1 => tamper(false, true, false),
        2 => {
            let f = Fixture::new()?;
            f.executable(
                "tool",
                "#!/bin/sh
exit 0
",
            )?;
            let mut r = f.runtime(&["./tool"], 1000, true)?;
            std::fs::remove_file(f.path.join("tool")).map_err(err)?;
            let report = r.check(Some("probe"));
            Ok(report["checks"][0]["status"] == "error" && !ok(&report))
        }
        3 => tamper(true, true, false),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn original_binary_runs() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn identical_restoration_runs() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn missing_executable_hard_error() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn restored_changed_binary_refused() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
