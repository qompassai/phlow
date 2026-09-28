//! Session multiplexing for phlow, adapted from the architecture of tuios
//! (MIT, Go/Bubble Tea terminal multiplexer).
//!
//! This crate is an adaptation, not a port: the concepts below are re-expressed
//! in Tiger Style Rust against phlow's needs. What it takes from tuios:
//!
//! - the agent state machine (`state`): none/working/needs_input/idle/done/
//!   errored/unknown, with `needs_input` carrying a reason and `needs_you`
//!   true for needs_input and errored;
//! - the agent inbox (`mailbox`): a bounded per-session ring of direct
//!   messages, session notices, and ask records, with sender rate limits,
//!   a reserved `human` address, no self-addressing, and ask-cycle refusal;
//! - the hook system (`hooks`): named events split into daemon-side and
//!   client-side, fired with a context of environment variables;
//! - the control protocol (`protocol`, `server`): line-delimited JSON over a
//!   Unix socket, one request line to one response line, the request id echoed
//!   back opaquely, stable string error codes, a 16 MiB request cap, and a
//!   socket directory held at mode 0700;
//! - the session daemon (`daemon`): sessions own windows, windows host agent
//!   panes, clients attach and detach;
//! - session tapes (`tape`): small declarative scripts that build a session.
//!
//! Deliberate differences from tuios, each documented where it applies:
//!
//! - the socket speaks JSON only. tuios sniffs the first byte to share the
//!   socket with a binary gob fast path; phlow has no binary peer, so there
//!   is nothing to sniff for;
//! - hook commands are argv arrays, never shell strings. tuios runs hooks
//!   through the shell; phlow-tuios keeps the operator's shell explicit
//!   (`["sh", "-c", ...]`) instead of implicit;
//! - tapes script agent sessions (windows, messages, agent states), not
//!   terminal keystrokes. tuios tapes drive demo playback; phlow tapes
//!   construct sessions.
//!
//! # Limits
//!
//! Every bound lives next to the code it guards as a named constant with
//! units: [`protocol::FRAME_BYTES_MAX`], [`mailbox::RING_MESSAGES_MAX`],
//! [`mailbox::RING_TEXT_BYTES_MAX`], [`mailbox::MESSAGE_TEXT_BYTES_MAX`],
//! [`tape::TAPE_BYTES_MAX`], and the hook bounds in [`hooks`].
//!
//! # Layout
//!
//! [`state`] and [`mailbox`] own agent state; [`hooks`] owns event hooks;
//! [`protocol`] owns the wire codec and verb registry; [`server`] owns the
//! Unix socket; [`daemon`] wires the verbs to sessions; [`tape`] owns the
//! scripting language. [`error`] owns the error type.

#![forbid(unsafe_code)]

pub mod daemon;
pub mod error;
pub mod hooks;
pub mod mailbox;
pub mod protocol;
pub mod server;
pub mod state;
pub mod tape;

pub use daemon::{AgentPane, SessionDaemon};
pub use error::{ErrorCode, TuiosError};
pub use hooks::{HookContext, HookEvent, HookManager, HookSide};
pub use mailbox::{AskGraph, HUMAN, Mailbox, Message, MessageKind, RateLimiter};
pub use protocol::{ProtocolError, Request, Response, VerbHint};
pub use state::{AgentActivity, AgentPaneState, AgentState};
pub use tape::{Tape, TapeCommand};

/// Truncate `s` to at most `max` bytes, backing off to the last UTF-8
/// character boundary so the result is always valid. Plain
/// `String::truncate` panics when `max` splits a multi-byte character, and
/// every truncation site here handles untrusted text.
pub(crate) fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- validation ---

    #[test]
    fn truncate_backs_off_to_a_char_boundary() {
        // "é" is two bytes in UTF-8; cutting inside it must back off to
        // "a", never panic the way String::truncate would.
        let got = truncate_bytes("aé", 2);
        assert_eq!(got, "a");
        assert!(got.is_char_boundary(got.len()), "result is valid UTF-8");
        // Exact fits and short strings pass through untouched.
        assert_eq!(truncate_bytes("aé", 3), "aé");
        assert_eq!(truncate_bytes("abc", 10), "abc");
        assert_eq!(truncate_bytes("", 0), "");
    }
}
