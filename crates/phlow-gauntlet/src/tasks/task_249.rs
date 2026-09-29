//! task-249: audit semantic completeness.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. `make_envelope` is the ledger
//! admission seam: tool execution records must reject an empty spawn identity and timestamps at
//! or before the Unix epoch. This tests validation, not automatic emission.

use std::time::{Duration, SystemTime};

use phlow_approval::{EventKind, EventPayload, make_envelope};

use super::task_209::{OPERATOR, RUN, run_cases};
use super::task_233::err;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-249";
/// Desired invariant.
pub const NAME: &str = "audit semantic completeness";
/// Drives the phlow-approval event admission seam directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "valid_spawn_envelope",
    "valid_completion_envelope",
    "empty_spawn_rejected",
    "invalid_time_rejected",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn spawn_payload(status: Option<&str>) -> EventPayload {
    EventPayload {
        tool: "check".to_owned(),
        actor: Some(OPERATOR.to_owned()),
        paths: vec!["/work/a".to_owned()],
        status: status.map(str::to_owned),
    }
}

fn case(index: usize) -> Result<bool, String> {
    let now = SystemTime::now();
    match index {
        0 => {
            let envelope = make_envelope(RUN, EventKind::ToolStarted, spawn_payload(None), now);
            Ok(envelope.map_err(err)?.kind == EventKind::ToolStarted)
        }
        1 => {
            let payload = spawn_payload(Some("denied"));
            let envelope =
                make_envelope(RUN, EventKind::ToolCompleted, payload, now).map_err(err)?;
            Ok(envelope.payload.status.as_deref() == Some("denied"))
        }
        2 => {
            let empty = EventPayload::default();
            Ok(make_envelope(RUN, EventKind::ToolStarted, empty, now).is_err())
        }
        3 => {
            let before_epoch = SystemTime::UNIX_EPOCH
                .checked_sub(Duration::from_nanos(1))
                .ok_or("platform cannot represent a pre-epoch time")?;
            let completed =
                |at| make_envelope(RUN, EventKind::ToolCompleted, spawn_payload(None), at);
            Ok(completed(before_epoch).is_err() && completed(SystemTime::UNIX_EPOCH).is_err())
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn valid_spawn_envelope() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn valid_completion_envelope() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn empty_spawn_rejected() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn invalid_time_rejected() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
