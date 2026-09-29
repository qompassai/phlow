//! task-246: approval audit completeness.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. The queue record is the approval
//! ledger: it retains identity, run, actor, monotonic creation/decision times and immutable scope.
//! Times are process-monotonic `Instant`s; no persistent ledger is fabricated.

use std::time::Instant;

use phlow_approval::{HumanVerdict, State};
use serde_json::json;

use super::task_209::{OPERATOR, RUN, pending, pending_from, record, req_json, run_cases};
use super::task_233::err;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-246";
/// Desired invariant.
pub const NAME: &str = "approval audit completeness";
/// Drives the phlow-approval queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "request_has_identity_time",
    "decision_retains_actor",
    "decision_has_timestamp",
    "scope_cannot_be_rewritten",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => {
            let (queue, id) = pending()?;
            let stored = record(&queue, &id)?;
            Ok(stored.id == id
                && stored.run_id == RUN
                && stored.created_at <= Instant::now()
                && stored.deadline > stored.created_at)
        }
        1 => {
            let (mut queue, id) = pending()?;
            queue
                .decide(&id, HumanVerdict::Approve, Some(OPERATOR))
                .map_err(err)?;
            Ok(record(&queue, &id)?.decided_by.as_deref() == Some(OPERATOR))
        }
        2 => {
            let (mut queue, id) = pending()?;
            queue
                .decide(&id, HumanVerdict::Deny, Some(OPERATOR))
                .map_err(err)?;
            let stored = record(&queue, &id)?;
            Ok(stored.state == State::Denied
                && stored.decided_at.is_some_and(|at| at >= stored.created_at))
        }
        3 => {
            let mut input = req_json();
            let (queue, id) = pending_from(&input)?;
            input["paths"][0] = json!("/work/other");
            Ok(record(&queue, &id)?.scope.paths() == ["/work/a"])
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn request_has_identity_time() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn decision_retains_actor() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn decision_has_timestamp() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn scope_cannot_be_rewritten() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
