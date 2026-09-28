//! The agent state machine, adapted from tuios's `internal/session/agent_state.go`.
//!
//! A pane running an agent reports its own semantic state; the daemon owns the
//! record and syncs it to readers. The states and their wire spellings are
//! taken from tuios (`AgentStateNone` etc. and `AgentStateNames`); the
//! `needs_you` rule (needs_input or errored) is tuios's `AgentState.NeedsYou`.
//!
//! tuios stores `none` as the empty string so it vanishes from serialized
//! state under `omitempty`. This module keeps that mapping: [`AgentState::as_wire`]
//! returns `""` for [`AgentState::None`], and [`AgentState::parse`] accepts
//! both `""` (absent field) and `"none"` (explicit clear).

use serde::{Deserialize, Serialize};

/// Semantic state of an agent running in a window's pane.
///
/// The zero-ish default is [`AgentState::None`]: the pane is not running an
/// agent or is not reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Not running an agent, or not reporting. Wire spelling: `""`/`"none"`.
    #[default]
    None,
    /// Actively working on a task.
    Working,
    /// Blocked waiting for the person. Carries a reason in the pane's message.
    NeedsInput,
    /// Not working and not blocked.
    Idle,
    /// Finished its task.
    Done,
    /// Stopped because of an error.
    Errored,
    /// An agent is present and nothing says what it is doing.
    Unknown,
}

/// Accepted wire spellings, in stable order. Part of the protocol surface.
pub const AGENT_STATE_NAMES: &[&str] = &[
    "none",
    "working",
    "needs_input",
    "idle",
    "done",
    "errored",
    "unknown",
];

impl AgentState {
    /// Parse a wire spelling. Accepts `""` (an absent field) as `None`, like
    /// tuios's omitempty convention; rejects everything outside
    /// [`AGENT_STATE_NAMES`].
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "" | "none" => Some(Self::None),
            "working" => Some(Self::Working),
            "needs_input" => Some(Self::NeedsInput),
            "idle" => Some(Self::Idle),
            "done" => Some(Self::Done),
            "errored" => Some(Self::Errored),
            "unknown" => Some(Self::Unknown),
            _ => Option::None,
        }
    }

    /// The wire spelling, mapping `None` back to `"none"` so a reader always
    /// gets an explicit value (tuios's `AgentState.Name`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Working => "working",
            Self::NeedsInput => "needs_input",
            Self::Idle => "idle",
            Self::Done => "done",
            Self::Errored => "errored",
            Self::Unknown => "unknown",
        }
    }

    /// Whether a person has to act on the pane now: a blocked agent, or one
    /// that stopped on an error. This is the single place the question is
    /// answered so the daemon, hooks, and readers agree.
    #[must_use]
    pub fn needs_you(self) -> bool {
        matches!(self, Self::NeedsInput | Self::Errored)
    }

    /// The coarse reading: what the agent is doing when the reader wants
    /// "working, waiting, or at rest" rather than the full enum.
    #[must_use]
    pub fn activity(self) -> AgentActivity {
        match self {
            Self::Working => AgentActivity::Working,
            Self::NeedsInput => AgentActivity::Waiting,
            Self::Idle | Self::Done | Self::Errored => AgentActivity::Resting,
            Self::Unknown => AgentActivity::Unknown,
            Self::None => AgentActivity::None,
        }
    }
}

/// Coarse activity derived from [`AgentState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentActivity {
    None,
    Working,
    Waiting,
    Resting,
    Unknown,
}

/// The daemon-owned per-pane agent record: the state plus what the pane said
/// about itself. The message and kind describe a block; they are cleared when
/// the pane moves to a state that carries no block, so a reason never outlives
/// the `needs_input` it described (tuios's `clearAgentNote`).
#[derive(Debug, Clone, Default)]
pub struct AgentPaneState {
    state: AgentState,
    /// Free text the pane carried with the state; the reason while blocked.
    message: String,
    /// The kind of block (`"question"`, `"approval"`, ...), if reported.
    kind: String,
    /// The harness id the reporting source named, if any.
    harness: String,
}

/// Bound: free text one pane's agent record may hold, in bytes.
pub const PANE_MESSAGE_BYTES_MAX: usize = 4 * 1024;
/// Bound: block-kind label length, in bytes.
pub const PANE_KIND_BYTES_MAX: usize = 64;
/// Bound: harness id length, in bytes.
pub const PANE_HARNESS_BYTES_MAX: usize = 64;

impl AgentPaneState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn state(&self) -> AgentState {
        self.state
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    #[must_use]
    pub fn harness(&self) -> &str {
        &self.harness
    }

    /// Record a pane's own report. Every transition is accepted: the pane is
    /// the authority on what it is doing. over-long fields are truncated to
    /// their bounds rather than rejected, since the reporter is trusted for
    /// its own state and a truncation never changes the state itself.
    pub fn set(&mut self, state: AgentState, message: &str, kind: &str, harness: &str) {
        self.state = state;
        if state.needs_you() {
            self.message = crate::truncate_bytes(message, PANE_MESSAGE_BYTES_MAX);
            self.kind = crate::truncate_bytes(kind, PANE_KIND_BYTES_MAX);
        } else {
            // A state that is not a block carries no block note.
            self.message.clear();
            self.kind.clear();
        }
        self.harness = crate::truncate_bytes(harness, PANE_HARNESS_BYTES_MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- validation ---

    #[test]
    fn all_wire_spellings_parse_and_round_trip() {
        for name in AGENT_STATE_NAMES {
            let state = AgentState::parse(name).expect("name must parse");
            assert_eq!(state.name(), *name);
        }
    }

    #[test]
    fn empty_wire_spelling_is_none() {
        assert_eq!(AgentState::parse(""), Some(AgentState::None));
    }

    #[test]
    fn needs_you_is_needs_input_and_errored_only() {
        assert!(AgentState::NeedsInput.needs_you());
        assert!(AgentState::Errored.needs_you());
        for s in [
            AgentState::None,
            AgentState::Working,
            AgentState::Idle,
            AgentState::Done,
            AgentState::Unknown,
        ] {
            assert!(!s.needs_you(), "{s:?} must not need you");
        }
    }

    #[test]
    fn block_note_survives_only_on_blocking_states() {
        let mut pane = AgentPaneState::new();
        pane.set(
            AgentState::NeedsInput,
            "approve deploy?",
            "approval",
            "claude",
        );
        assert_eq!(pane.message(), "approve deploy?");
        pane.set(AgentState::Working, "", "", "");
        assert_eq!(pane.message(), "");
        assert_eq!(pane.kind(), "");
        // The harness is identity, not a block note: it survives.
        pane.set(AgentState::NeedsInput, "q?", "question", "opencode");
        pane.set(AgentState::Done, "", "", "opencode");
        assert_eq!(pane.harness(), "opencode");
    }

    // --- adversarial ---

    #[test]
    fn unknown_spelling_is_rejected() {
        assert_eq!(AgentState::parse("frobnicating"), Option::None);
    }

    #[test]
    fn spellings_are_case_sensitive() {
        assert_eq!(AgentState::parse("WORKING"), Option::None);
        assert_eq!(AgentState::parse("Needs_Input"), Option::None);
    }

    #[test]
    fn whitespace_padded_spelling_is_rejected() {
        assert_eq!(AgentState::parse(" working"), Option::None);
        assert_eq!(AgentState::parse("working\n"), Option::None);
    }

    #[test]
    fn overlong_message_is_truncated_at_char_boundary() {
        let mut pane = AgentPaneState::new();
        // Multi-byte tail: truncation must not split a char.
        let msg = format!("{}é", "x".repeat(PANE_MESSAGE_BYTES_MAX));
        pane.set(AgentState::NeedsInput, &msg, "", "");
        assert!(pane.message().len() <= PANE_MESSAGE_BYTES_MAX);
        assert!(pane.message().is_char_boundary(pane.message().len()));
    }
}
