//! task-241: approval actor hardness.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. `ApprovalQueue::decide` is the
//! human-decision boundary: a missing actor or a model identity must not commit a grant, and
//! reserved model identities cannot be configured as operators. Actor authentication beyond this
//! local allowlist is not claimed.

use phlow_approval::{ApprovalQueue, Error, HumanVerdict, State};

use super::task_209::{OPERATOR, pending, record, run_cases};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-241";
/// Desired invariant.
pub const NAME: &str = "approval actor hardness";
/// Drives the phlow-approval queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "human_approval",
    "human_denial",
    "missing_actor_refused",
    "model_actor_refused",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    let (mut queue, id) = pending()?;
    let (verdict, actor, expected) = match index {
        0 => (HumanVerdict::Approve, Some(OPERATOR), State::Approved),
        1 => (HumanVerdict::Deny, Some(OPERATOR), State::Denied),
        2 => (HumanVerdict::Approve, None, State::Pending),
        3 => (HumanVerdict::Approve, Some("model"), State::Pending),
        _ => return Err("case index outside fixed array".to_owned()),
    };
    let result = queue.decide(&id, verdict, actor);
    let state_held = record(&queue, &id)?.state == expected;
    Ok(match index {
        0 | 1 => result.is_ok() && state_held,
        2 => result == Err(Error::ActorRefused) && state_held,
        _ => {
            result == Err(Error::ActorRefused)
                && state_held
                && ApprovalQueue::new(&["model"]).is_err()
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn human_approval() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn human_denial() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn missing_actor_refused() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn model_actor_refused() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
