//! task-220: proposal expiry enforced at decision.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. Uses the queue's real monotonic
//! clock and `sweep_expired`. `decide` enforces the deadline even without a sweep; the unswept
//! case waits a bounded 10 ms past a 1 ms TTL.

use std::time::Duration;

use phlow_approval::{Error, HumanVerdict, State};

use super::task_209::{HUMAN, RUN, pending, queue, record, req, run_cases};
use super::task_233::err;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-220";
/// Desired permission invariant.
pub const NAME: &str = "proposal expiry enforced at decision";
/// Drives the phlow-approval queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "before_deadline_stays_pending",
    "deadline_sweep_expires",
    "swept_expired_cannot_approve",
    "unswept_expired_cannot_approve",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    if index == 3 {
        return unswept_expired_cannot_approve();
    }
    let (mut queue, id) = pending()?;
    let stored = record(&queue, &id)?;
    match index {
        0 => {
            let expired_count = queue.sweep_expired(stored.created_at);
            Ok(expired_count == 0 && record(&queue, &id)?.state == State::Pending)
        }
        1 => {
            let expired_count = queue.sweep_expired(stored.deadline);
            Ok(expired_count == 1 && record(&queue, &id)?.state == State::Expired)
        }
        2 => {
            queue.sweep_expired(stored.deadline);
            let late = queue.decide(&id, HumanVerdict::Approve, Some(HUMAN));
            Ok(late
                == Err(Error::WrongState {
                    state: State::Expired,
                }))
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

fn unswept_expired_cannot_approve() -> Result<bool, String> {
    const TTL: Duration = Duration::from_millis(1);
    const WAIT: Duration = Duration::from_millis(10);
    let mut queue = queue()?;
    let id = queue.request(RUN, req()?, TTL).map_err(err)?;
    std::thread::sleep(WAIT);
    let late = queue.decide(&id, HumanVerdict::Approve, Some(HUMAN));
    Ok(late == Err(Error::Expired) && record(&queue, &id)?.state == State::Expired)
}

#[cfg(test)]
mod tests {
    #[test]
    fn before_deadline_stays_pending() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn deadline_sweep_expires() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn swept_expired_cannot_approve() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn unswept_expired_cannot_approve() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
