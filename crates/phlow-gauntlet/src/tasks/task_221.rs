//! task-221: denial proposal round trip.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. Feeds a real `decide` denial through
//! `Decision::proposal` into the queue. The denial must carry enough to build the exact proposal,
//! and the proposal must carry only that scope: no endpoints, permissions or summary smuggled in
//! from the original request.

use phlow_approval::{DEFAULT_TTL, PermissionDelta, Request, decide};
use serde_json::json;

use super::task_209::{RUN, pending, queue, record, req, req_with, run_cases};
use super::task_233::err;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-221";
/// Desired permission invariant.
pub const NAME: &str = "denial proposal round trip";
/// Drives the phlow-approval policy seam and queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "request_scope_survives_queue",
    "request_risk_survives_queue",
    "denial_is_sufficient_for_proposal",
    "denial_round_trip_preserves_only_scope",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    let original = req()?;
    match index {
        0 | 1 => {
            let (queue, id) = pending()?;
            let stored = record(&queue, &id)?;
            Ok(match index {
                0 => stored.scope.paths() == original.scope().paths(),
                _ => stored.scope.risk() == original.scope().risk(),
            })
        }
        2 => {
            let denial = decide(None, original.scope());
            let mut queue = queue()?;
            let id = queue
                .request(RUN, denial.proposal(), DEFAULT_TTL)
                .map_err(err)?;
            Ok(record(&queue, &id)?.scope.tool() == original.scope().tool())
        }
        3 => preserves_only_scope(),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

fn preserves_only_scope() -> Result<bool, String> {
    let widened = req_with(json!({ "permissions_after": ["root"], "summary": "also grant root" }))?;
    let source = Request::from_json(&widened).map_err(err)?;
    let denial = decide(None, source.scope());
    let mut queue = queue()?;
    let id = queue
        .request(RUN, denial.proposal(), DEFAULT_TTL)
        .map_err(err)?;
    let proposal = record(&queue, &id)?;
    Ok(proposal.scope == *source.scope()
        && proposal.scope.endpoints().is_empty()
        && proposal.permissions_after.is_empty()
        && proposal.permission_delta == PermissionDelta::default()
        && proposal.summary.is_none())
}

#[cfg(test)]
mod tests {
    #[test]
    fn request_scope_survives_queue() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn request_risk_survives_queue() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn denial_is_sufficient_for_proposal() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn denial_round_trip_preserves_only_scope() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
