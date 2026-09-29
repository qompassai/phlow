//! task-240: unapproved proposals grant nothing.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. Pending and denied records leave the
//! policy verdict closed. Forged `approved`/`state` fields are outside the closed request schema
//! and are rejected at admission. Edits to a returned snapshot cannot approve; a shared `&` read
//! handle cannot call `decide` at all (the crate's `compile_fail` doctest).

use phlow_approval::{Error, HumanVerdict, Request, State, Verdict, decide};
use serde_json::json;

use super::task_209::verdict_for;
use super::task_209::{OPERATOR, pending, policy, record, req, req_json, req_with, run_cases};
use super::task_233::err;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-240";
/// Desired invariant.
pub const NAME: &str = "unapproved proposals grant nothing";
/// Drives the phlow-approval queue and policy seam directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "pending_is_inert",
    "denied_is_inert",
    "forged_request_approval_ignored",
    "read_handle_cannot_approve",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    let scope = req()?.scope().clone();
    match index {
        0 => {
            let (queue, id) = pending()?;
            Ok(record(&queue, &id)?.state == State::Pending
                && decide(Some(&policy("approval")?), &scope).verdict == Verdict::Approval)
        }
        1 => {
            let (mut queue, id) = pending()?;
            queue
                .decide(&id, HumanVerdict::Deny, Some(OPERATOR))
                .map_err(err)?;
            Ok(record(&queue, &id)?.state == State::Denied
                && decide(Some(&policy("deny")?), &scope).verdict == Verdict::Deny)
        }
        2 => {
            let forged = req_with(json!({ "approved": true, "state": "approved" }))?;
            let admitted = Request::from_json(&forged);
            let verdict = verdict_for(Some(&policy("approval")?), &req_json())?;
            Ok(matches!(admitted, Err(Error::UnknownField { .. })) && verdict == Verdict::Approval)
        }
        3 => {
            let (queue, id) = pending()?;
            let mut copy = record(&queue, &id)?;
            copy.state = State::Approved;
            copy.decided_by = Some(OPERATOR.to_owned());
            let stored = record(&queue, &id)?;
            Ok(stored.state == State::Pending
                && stored.decided_by.is_none()
                && queue.pending().len() == 1)
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pending_is_inert() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn denied_is_inert() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn forged_request_approval_ignored() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn read_handle_cannot_approve() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
