//! task-248: append only audit ownership.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. The real `EventSink` stores owned
//! tool.started/completed events. Mutation attacks target the caller's payload after append and
//! the copies returned by `events`; no fake ledger is involved.

use phlow_approval::{EVENT_SOURCE, EventKind, EventPayload, EventSink};

use super::task_209::{RUN, run_cases};
use super::task_233::err;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-248";
/// Desired invariant.
pub const NAME: &str = "append only audit ownership";
/// Drives the phlow-approval event sink directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "ordered_events",
    "envelope_has_time_source",
    "append_input_cannot_rewrite",
    "reader_cannot_erase_scope",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn check_payload(paths: &[&str]) -> EventPayload {
    EventPayload {
        tool: "check".to_owned(),
        paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        ..EventPayload::default()
    }
}

fn case(index: usize) -> Result<bool, String> {
    let mut sink = EventSink::new();
    match index {
        0 => {
            sink.append(RUN, EventKind::ToolStarted, check_payload(&[]))
                .map_err(err)?;
            sink.append(RUN, EventKind::ToolCompleted, check_payload(&[]))
                .map_err(err)?;
            let rows = sink.events(Some(RUN));
            Ok(rows.len() == 2 && rows[0].seq < rows[1].seq)
        }
        1 => {
            let event = sink
                .append(RUN, EventKind::ToolStarted, check_payload(&[]))
                .map_err(err)?;
            let envelope = event.envelope;
            Ok(
                envelope.run_id == RUN
                    && envelope.ts_unix_ns > 0
                    && envelope.source == EVENT_SOURCE,
            )
        }
        2 => {
            let mut payload = check_payload(&[]);
            sink.append(RUN, EventKind::ToolStarted, payload.clone())
                .map_err(err)?;
            payload.tool = "other".to_owned();
            let stored = sink
                .events(None)
                .first()
                .map(|e| e.envelope.payload.tool.clone());
            Ok(payload.tool == "other" && stored.as_deref() == Some("check"))
        }
        3 => {
            sink.append(RUN, EventKind::ToolStarted, check_payload(&["/work/a"]))
                .map_err(err)?;
            let mut rows = sink.events(None);
            let Some(path) = rows
                .first_mut()
                .and_then(|e| e.envelope.payload.paths.first_mut())
            else {
                return Ok(false);
            };
            *path = "/outside".to_owned();
            let stored = sink.events(None);
            Ok(stored.first().map(|e| e.envelope.payload.paths.as_slice())
                == Some(&["/work/a".to_owned()][..]))
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ordered_events() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn envelope_has_time_source() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn append_input_cannot_rewrite() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn reader_cannot_erase_scope() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
