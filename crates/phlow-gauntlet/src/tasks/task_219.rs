//! task-219: append only approval history.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. `ApprovalQueue::get` returns owned
//! snapshots: a second decision cannot overwrite the first, and editing a returned record cannot
//! change state or erase attribution. The queue is in memory; no durable ledger is claimed.

use phlow_approval::{Error, HumanVerdict, State};

use super::task_209::{HUMAN, pending, record, run_cases};
use super::task_233::err;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-219";
/// Desired permission invariant.
pub const NAME: &str = "append only approval history";
/// Drives the phlow-approval queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "decision_remains_readable",
    "second_decision_cannot_overwrite",
    "reader_cannot_edit_history",
    "reader_cannot_delete_attribution",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    let (mut queue, id) = pending()?;
    queue
        .decide(&id, HumanVerdict::Approve, Some(HUMAN))
        .map_err(err)?;
    match index {
        0 => Ok(record(&queue, &id)?.state == State::Approved),
        1 => {
            let second = queue.decide(&id, HumanVerdict::Deny, Some(HUMAN));
            Ok(second
                == Err(Error::WrongState {
                    state: State::Approved,
                })
                && record(&queue, &id)?.state == State::Approved)
        }
        2 => {
            let mut copy = record(&queue, &id)?;
            copy.state = State::Pending;
            Ok(record(&queue, &id)?.state == State::Approved)
        }
        3 => {
            let mut copy = record(&queue, &id)?;
            copy.decided_by = None;
            Ok(record(&queue, &id)?.decided_by.as_deref() == Some(HUMAN))
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn decision_remains_readable() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn second_decision_cannot_overwrite() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn reader_cannot_edit_history() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn reader_cannot_delete_attribution() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
