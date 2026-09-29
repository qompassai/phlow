//! Append-only, in-memory ledger of tool execution events.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::Error;
use crate::scope::{SCOPE_ITEMS_MAX, check_path, check_plain};

/// Maximum events one sink holds; a full sink rejects appends (no eviction).
pub const EVENTS_MAX: usize = 4096;
/// The `source` stamped on every envelope this crate builds.
pub const EVENT_SOURCE: &str = "phlow-approval";

/// What happened to a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    ToolStarted,
    ToolCompleted,
}

/// Who ran what, where, with what outcome.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventPayload {
    /// Required, non-empty: the spawn identity.
    pub tool: String,
    pub actor: Option<String>,
    pub paths: Vec<String>,
    pub status: Option<String>,
}

/// A validated event, not yet sequenced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub run_id: String,
    pub kind: EventKind,
    /// Wall-clock nanoseconds since the Unix epoch; always positive.
    pub ts_unix_ns: u64,
    pub source: &'static str,
    pub payload: EventPayload,
}

/// An envelope with its position in the sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Strictly increasing from 1 within one sink.
    pub seq: u64,
    pub envelope: Envelope,
}

/// Validate and build an envelope stamped at `at`.
///
/// Rejects an empty or invalid run ID or tool, invalid actor/status/paths,
/// more than [`SCOPE_ITEMS_MAX`] paths, and a time at or before the Unix
/// epoch or beyond `u64` nanoseconds.
pub fn make_envelope(
    run_id: &str,
    kind: EventKind,
    payload: EventPayload,
    at: SystemTime,
) -> Result<Envelope, Error> {
    check_plain("run_id", run_id)?;
    check_plain("tool", &payload.tool)?;
    if let Some(actor) = &payload.actor {
        check_plain("actor", actor)?;
    }
    if let Some(status) = &payload.status {
        check_plain("status", status)?;
    }
    if payload.paths.len() > SCOPE_ITEMS_MAX {
        return Err(Error::TooMany {
            field: "paths",
            max: SCOPE_ITEMS_MAX,
        });
    }
    for path in &payload.paths {
        check_path("paths", path)?;
    }
    let invalid_time = Error::InvalidValue {
        field: "ts",
        reason: "must be after the Unix epoch and fit in u64 nanoseconds",
    };
    let since_epoch = at
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid_time.clone())?;
    let ts_unix_ns = u64::try_from(since_epoch.as_nanos()).map_err(|_| invalid_time.clone())?;
    if ts_unix_ns == 0 {
        return Err(invalid_time);
    }
    Ok(Envelope {
        run_id: run_id.to_owned(),
        kind,
        ts_unix_ns,
        source: EVENT_SOURCE,
        payload,
    })
}

/// Append-only event sink. Readers receive owned copies.
#[derive(Debug, Default)]
pub struct EventSink {
    events: Vec<Event>,
}

impl EventSink {
    /// An empty sink.
    pub fn new() -> Self {
        EventSink::default()
    }

    /// Validate, stamp with the current time, sequence and store an event.
    /// Returns a copy of the stored event.
    pub fn append(
        &mut self,
        run_id: &str,
        kind: EventKind,
        payload: EventPayload,
    ) -> Result<Event, Error> {
        if self.events.len() >= EVENTS_MAX {
            return Err(Error::TooMany {
                field: "events",
                max: EVENTS_MAX,
            });
        }
        let envelope = make_envelope(run_id, kind, payload, SystemTime::now())?;
        let seq = u64::try_from(self.events.len()).map_err(|_| Error::TooMany {
            field: "events",
            max: EVENTS_MAX,
        })? + 1;
        let event = Event { seq, envelope };
        self.events.push(event.clone());
        Ok(event)
    }

    /// Copies of stored events in sequence order, optionally for one run.
    pub fn events(&self, run_id: Option<&str>) -> Vec<Event> {
        self.events
            .iter()
            .filter(|event| run_id.is_none_or(|run| event.envelope.run_id == run))
            .cloned()
            .collect()
    }
}
