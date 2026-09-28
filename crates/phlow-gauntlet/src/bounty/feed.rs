//! Scope feeds. `ScopeFeed` is the trait; `ScriptedFeed` serves canned
//! snapshots and can be told to go malformed or hostile on cue.
//!
//! Wire-format note: this is "bbscope-style" (periodic scope polling),
//! not "bbscope-compatible" — the wave-23 worker must verify bbscope's
//! real semantics against its repo before claiming compatibility.

use crate::bounty::types::{ScopeSnapshot, Target, TargetId, TargetKind};

#[derive(Debug, PartialEq, Eq)]
pub enum FeedError {
    Malformed(String),
    HostileTarget { value: String, reason: String },
    Transport(String),
}

/// A source of scope snapshots.
pub trait ScopeFeed {
    fn poll(&mut self) -> Result<ScopeSnapshot, FeedError>;
}

/// Scripted feed for deterministic drivers. `snapshots` are served in
/// order; when exhausted, the last one repeats. `malformed_at` /
/// `hostile_at` (poll indices) inject failures on cue.
pub struct ScriptedFeed {
    snapshots: Vec<ScopeSnapshot>,
    polls: usize,
    malformed_at: Option<usize>,
    hostile_at: Option<usize>,
    hostile_target: Option<Target>,
}

impl ScriptedFeed {
    pub fn new(snapshots: Vec<ScopeSnapshot>) -> Self {
        ScriptedFeed {
            snapshots,
            polls: 0,
            malformed_at: None,
            hostile_at: None,
            hostile_target: None,
        }
    }

    pub fn with_malformed_at(mut self, poll_idx: usize) -> Self {
        self.malformed_at = Some(poll_idx);
        self
    }

    pub fn with_hostile_at(mut self, poll_idx: usize, target: Target) -> Self {
        self.hostile_at = Some(poll_idx);
        self.hostile_target = Some(target);
        self
    }

    pub fn polls_done(&self) -> usize {
        self.polls
    }
}

impl ScopeFeed for ScriptedFeed {
    fn poll(&mut self) -> Result<ScopeSnapshot, FeedError> {
        let idx = self.polls;
        self.polls += 1;
        if self.malformed_at == Some(idx) {
            return Err(FeedError::Malformed(format!(
                "scripted malformed snapshot at poll {idx}"
            )));
        }
        if self.hostile_at == Some(idx)
            && let Some(t) = self.hostile_target.clone()
        {
            return Err(FeedError::HostileTarget {
                value: t.value.clone(),
                reason: "scripted hostile target injected by fixture".to_string(),
            });
        }
        let snap = self
            .snapshots
            .get(idx)
            .or_else(|| self.snapshots.last())
            .cloned();
        snap.ok_or_else(|| FeedError::Transport("no snapshots scripted".to_string()))
    }
}

/// Build a target without fuss in drivers.
pub fn target(id: &str, kind: TargetKind, value: &str) -> Target {
    Target {
        id: TargetId(id.to_string()),
        kind,
        value: value.to_string(),
    }
}

/// Build a snapshot without fuss in drivers.
pub fn snapshot(version: u64, fetched_at: u64, targets: Vec<Target>) -> ScopeSnapshot {
    ScopeSnapshot {
        version,
        targets,
        fetched_at,
    }
}
