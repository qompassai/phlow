//! task-222: emergency approval revocation.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. `ApprovalQueue::revoke` withdraws an
//! approval immediately while keeping the original human attribution beside the revocation; a
//! denial stays final. Executor integration remains unproven.

use phlow_approval::{Error, HumanVerdict, State};

use super::task_209::{HUMAN, OPERATOR, pending, record, run_cases};
use super::task_233::err;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-222";
/// Desired permission invariant.
pub const NAME: &str = "emergency approval revocation";
/// Drives the phlow-approval queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "pending_can_be_denied",
    "denied_cannot_be_reapproved",
    "approved_can_be_revoked",
    "revocation_visible_on_next_read",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    let (mut queue, id) = pending()?;
    match index {
        0 => {
            queue
                .decide(&id, HumanVerdict::Deny, Some(HUMAN))
                .map_err(err)?;
            Ok(record(&queue, &id)?.state == State::Denied)
        }
        1 => {
            queue
                .decide(&id, HumanVerdict::Deny, Some(HUMAN))
                .map_err(err)?;
            let again = queue.decide(&id, HumanVerdict::Approve, Some(HUMAN));
            Ok(again
                == Err(Error::WrongState {
                    state: State::Denied,
                }))
        }
        2 => {
            queue
                .decide(&id, HumanVerdict::Approve, Some(HUMAN))
                .map_err(err)?;
            let revoked = queue.revoke(&id, Some(OPERATOR));
            Ok(revoked.is_ok() && record(&queue, &id)?.state == State::Revoked)
        }
        3 => {
            queue
                .decide(&id, HumanVerdict::Approve, Some(HUMAN))
                .map_err(err)?;
            let snapshot = record(&queue, &id)?;
            queue.revoke(&id, Some(OPERATOR)).map_err(err)?;
            let now = record(&queue, &id)?;
            Ok(snapshot.state == State::Approved
                && now.state == State::Revoked
                && now.revoked_by.as_deref() == Some(OPERATOR)
                && now.decided_by.as_deref() == Some(HUMAN))
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pending_can_be_denied() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn denied_cannot_be_reapproved() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn approved_can_be_revoked() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn revocation_visible_on_next_read() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
