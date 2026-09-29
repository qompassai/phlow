//! task-238: PATH shadowing pinned identity.
//!
//! Honest scope: A child gauntlet probe gets a private PATH. The runtime is constructed while only
//! the second directory has the named tool; adding an earlier shadow later must be detected. Child
//! environment isolation avoids unsafe global environment edits.

use super::task_233::path_probe;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-238";
/// Desired invariant.
pub const NAME: &str = "PATH shadowing pinned identity";
/// Executes real Rust runtime seams.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "unshadowed_name_runs",
    "unshadowed_same_bytes_runs",
    "earlier_path_shadow_blocked",
    "same_digest_path_shadow_blocked",
];

/// Execute all cases, retaining assertion failures separately from fixture errors.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    if let Ok(mode) = std::env::var("PHLOW_PIN_CHILD_MODE") {
        let result = std::env::var_os("PHLOW_PIN_CHILD_DIR")
            .ok_or_else(|| "PATH child directory missing".to_owned())
            .and_then(|path| super::task_233::path_child(&mode, std::path::Path::new(&path)));
        return super::task_233::collect_outcome(ID, std::iter::once(("isolated_path", result)));
    }
    super::task_233::collect_outcome(ID, CASES.iter().enumerate().map(|(i, c)| (*c, case(i))))
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => path_probe(false, false),
        1 => path_probe(false, true),
        2 => path_probe(true, false),
        3 => path_probe(true, true),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unshadowed_name_runs() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn unshadowed_same_bytes_runs() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn earlier_path_shadow_blocked() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn same_digest_path_shadow_blocked() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
