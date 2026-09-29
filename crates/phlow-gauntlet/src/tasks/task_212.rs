//! task-212: human bound approval.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. `ApprovalQueue::decide` accepts only
//! actors on the queue's explicit operator allowlist; an agent identity or a missing actor is
//! refused and leaves the record pending. Operator names are configuration, not identity proof.

use phlow_approval::{Error, HumanVerdict, State};

use super::task_209::{HUMAN, pending, record, run_cases};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-212";
/// Desired permission invariant.
pub const NAME: &str = "human bound approval";
/// Drives the phlow-approval queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "human_approval_recorded",
    "human_denial_recorded",
    "agent_self_approval_rejected",
    "anonymous_approval_rejected",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    let (mut queue, id) = pending()?;
    match index {
        0 => {
            let decided = queue.decide(&id, HumanVerdict::Approve, Some(HUMAN));
            Ok(decided.is_ok() && record(&queue, &id)?.decided_by.as_deref() == Some(HUMAN))
        }
        1 => {
            let decided = queue.decide(&id, HumanVerdict::Deny, Some(HUMAN));
            Ok(decided.is_ok() && record(&queue, &id)?.state == State::Denied)
        }
        2 => {
            let refused = queue.decide(&id, HumanVerdict::Approve, Some("agent"));
            Ok(refused == Err(Error::ActorRefused) && record(&queue, &id)?.state == State::Pending)
        }
        3 => {
            let refused = queue.decide(&id, HumanVerdict::Approve, None);
            Ok(refused == Err(Error::ActorRefused) && record(&queue, &id)?.state == State::Pending)
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn human_approval_recorded() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn human_denial_recorded() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn agent_self_approval_rejected() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn anonymous_approval_rejected() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
