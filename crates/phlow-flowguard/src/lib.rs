//! Flow security for phlow, in the OpenAPPA style.
//!
//! Plain words: every unit of work (a *trajectory*) carries a security
//! label — who may receive what it has touched (its **audience**) and
//! how much that material can be relied on (its **trust**). Reading
//! restricted data narrows the audience; reading unvetted data lowers
//! trust. Labels only ever get *more* restrictive: the label is defined
//! as a fold over an append-only event log, and the only combining
//! operation is the join (intersect audiences, take the lower trust),
//! so a downgrade is not an operation this crate can express.
//!
//! Before every tool call and every inter-agent or external message, a
//! **deterministic gate** answers one question from the policy and the
//! log alone: is this data allowed to go to this destination? No model
//! is consulted, no clock is read, no ambient state leaks in — the same
//! log and the same action always produce the same decision, which is
//! what makes the decision replayable and auditable. The gate sits
//! outside the agent loop; nothing model-visible can address it.
//!
//! A block is not a dead end: it carries a machine-readable
//! [`RemedyPlan`] — sanitize and retry, ask an authority to approve
//! this one action (approvals never change the label), or run the step
//! in a disposable sub-agent trajectory so the parent's label stays
//! clean.
//!
//! # Bounds
//!
//! - [`EVENTS_MAX`]: most events in one trajectory log.
//! - [`READER_TOKENS_MAX`]: most tokens in one audience declaration.
//! - [`POLICY_TOOLS_MAX`]: most tool contracts in one policy.
//! - [`TOKEN_BYTES_MAX`]: longest accepted reader/group/token string.
//!
//! # Unsafe policy
//!
//! This crate forbids unsafe code outright.

#![forbid(unsafe_code)]

pub mod error;
pub mod event;
pub mod gate;
pub mod label;
pub mod policy;
pub mod trajectory;

pub use error::FlowError;
pub use event::{Event, JoinSpec, ProposedAction};
pub use gate::{Decision, RemedyOption, RemedyPlan, action_digest, apply_sanitizer, decide};
pub use label::{Label, LabelDelta, LabelSnapshot, ReaderSet};
pub use policy::Policy;
pub use trajectory::Trajectory;

/// Most events in one trajectory log. A trajectory that needs more
/// history than this is a new trajectory, not a longer log.
pub const EVENTS_MAX: usize = 10_000;

/// Most tokens in one audience declaration (a delta, a destination's
/// reader list, a group's member list).
pub const READER_TOKENS_MAX: usize = 256;

/// Most tool contracts in one policy.
pub const POLICY_TOOLS_MAX: usize = 512;

/// Longest accepted reader id, group name, or token string, in bytes.
pub const TOKEN_BYTES_MAX: usize = 128;

/// The reserved attention mark that denies a call outright. No
/// authority may permit it and no remedy is offered for a contract
/// that requires it (OpenAPPA's rule, adopted verbatim).
pub const ATTENTION_BLOCKED: &str = "blocked";
