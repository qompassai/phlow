//! task-211: review token integrity.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. Approval IDs are the token seam:
//! unknown IDs, replay of a consumed ID, and a copied ID presented to another queue must all be
//! refused. IDs are unique and queue-bound, not cryptographically unguessable; authority comes
//! from the operator allowlist, not ID secrecy. The queue is not `Clone`, so it cannot be forked.

use phlow_approval::{DEFAULT_TTL, Error, HumanVerdict, State};

use super::task_209::{HUMAN, RUN, pending, queue, record, req, run_cases};
use super::task_233::err;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-211";
/// Desired permission invariant.
pub const NAME: &str = "review token integrity";
/// Drives the phlow-approval queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "human_decision_recorded",
    "distinct_requests_have_distinct_ids",
    "forged_id_rejected",
    "copied_consumed_token_rejected",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    let approve = HumanVerdict::Approve;
    match index {
        0 => {
            let (mut queue, id) = pending()?;
            let decided = queue.decide(&id, approve, Some(HUMAN));
            Ok(decided.is_ok() && record(&queue, &id)?.state == State::Approved)
        }
        1 => {
            let (mut queue, id) = pending()?;
            let other = queue.request(RUN, req()?, DEFAULT_TTL).map_err(err)?;
            Ok(other != id)
        }
        2 => {
            let mut queue = queue()?;
            let forged = queue.decide("forged-review-id", approve, Some(HUMAN));
            Ok(forged == Err(Error::UnknownId) && queue.pending().is_empty())
        }
        3 => {
            let (mut queue, id) = pending()?;
            let copied = id.clone();
            let first = queue.decide(&id, approve, Some(HUMAN));
            let replay = queue.decide(&copied, approve, Some(HUMAN));
            let (mut fork, fork_id) = pending()?;
            let forked = fork.decide(&copied, approve, Some(HUMAN));
            Ok(first.is_ok()
                && replay
                    == Err(Error::WrongState {
                        state: State::Approved,
                    })
                && forked == Err(Error::UnknownId)
                && record(&fork, &fork_id)?.state == State::Pending)
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn human_decision_recorded() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn distinct_requests_have_distinct_ids() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn forged_id_rejected() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn copied_consumed_token_rejected() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
