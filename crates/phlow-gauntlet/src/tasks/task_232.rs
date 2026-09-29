//! task-232: approval surface delta fidelity.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. `ApprovalQueue::pending` is the
//! approval-surface data source; it must expose the same computed delta as `get`, including a
//! revocation a summary tries to hide. A claimed empty delta is rejected at admission and cannot
//! mask the honest request's rename.

use phlow_approval::{Error, Request};
use serde_json::json;

use super::task_209::{pending_from, record, req_with, run_cases};
use super::task_225::{delta_with, is_delta};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-232";
/// Desired invariant.
pub const NAME: &str = "approval surface delta fidelity";
/// Drives the phlow-approval queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "pending_exposes_delta",
    "get_and_pending_agree",
    "summary_cannot_hide_revoke",
    "empty_claim_cannot_hide_rename",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => {
            let input = req_with(json!({ "permissions_before": [], "permissions_after": ["a"] }))?;
            let (queue, _id) = pending_from(&input)?;
            let rows = queue.pending();
            Ok(rows.len() == 1 && is_delta(&rows[0].permission_delta, &["a"], &[]))
        }
        1 => {
            let input = req_with(json!({ "permissions_before": ["a"], "permissions_after": [] }))?;
            let (queue, id) = pending_from(&input)?;
            let from_get = record(&queue, &id)?.permission_delta;
            let rows = queue.pending();
            Ok(rows.len() == 1
                && rows[0].permission_delta == from_get
                && is_delta(&from_get, &[], &["a"]))
        }
        2 => {
            let delta = delta_with(json!({
                "permissions_before": ["admin"],
                "permissions_after": [],
                "summary": "Read-only operation, no change",
            }))?;
            Ok(is_delta(&delta, &[], &["admin"]))
        }
        3 => {
            let rename = json!({ "permissions_before": ["a"], "permissions_after": ["b"] });
            let mut claimed = req_with(rename.clone())?;
            claimed["permission_delta"] = json!({ "added": [], "removed": [] });
            let rejected = matches!(
                Request::from_json(&claimed),
                Err(Error::UnknownField { field, .. }) if field == "permission_delta"
            );
            Ok(rejected && is_delta(&delta_with(rename)?, &["b"], &["a"]))
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pending_exposes_delta() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn get_and_pending_agree() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn summary_cannot_hide_revoke() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn empty_claim_cannot_hide_rename() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
