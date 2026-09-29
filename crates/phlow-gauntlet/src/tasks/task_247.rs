//! task-247: denial audit reconstruction.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. `decide` is the denial output seam: a
//! denial must retain structured tool, risk and scope for ledger ingestion, not merely a prose
//! reason. Persistence is not covered.

use phlow_approval::{Risk, Verdict, decide};

use super::task_209::{req, run_cases};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-247";
/// Desired invariant.
pub const NAME: &str = "denial audit reconstruction";
/// Drives the phlow-approval policy seam directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "denial_has_reason",
    "denial_has_risk",
    "denial_has_tool",
    "denial_has_scope",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    let denial = decide(None, req()?.scope());
    if denial.verdict != Verdict::Deny {
        return Ok(false);
    }
    match index {
        0 => Ok(!denial.reason.is_empty()),
        1 => Ok(denial.scope.risk() == Risk::LocalReversible),
        2 => Ok(denial.scope.tool() == "fs.write"),
        3 => Ok(denial.scope.paths() == ["/work/a"] && denial.scope.endpoints().is_empty()),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn denial_has_reason() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn denial_has_risk() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn denial_has_tool() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn denial_has_scope() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
