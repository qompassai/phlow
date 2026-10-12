//! Grounded joint-attention coordination for phlow agents.
//!
//! Plain words: agents that were never trained together — different
//! models swapped into the planner/coder/reviewer roles, external agents
//! arriving over MCP or A2A — cannot rely on shared habits to decide who
//! works on what. This crate gives them a coordination signal computed
//! from the shared workspace instead: a saliency snapshot any agent can
//! reproduce exactly, a focus advertisement derived from that snapshot,
//! an alignment measurement between two foci, and a deterministic rule
//! that turns a pair of foci into a coordination decision.
//!
//! The design is the LLM-stack analog of MATE (Benintendi et al.,
//! arXiv:2610.06025): attention grounded in environment objects, shared
//! between partners during the interaction, compared only within a
//! common frame. Here the "frame" is the snapshot hash: foci computed
//! from different snapshots are never compared — alignment is undefined
//! across frames and the coordination rule fails closed.
//!
//! # Bounds
//!
//! - [`SNAPSHOT_TARGETS_MAX`]: most targets one snapshot may carry.
//! - [`FOCUS_TARGETS_MAX`]: most targets one focus may advertise.
//! - [`TARGET_ID_MAX_BYTES`]: longest accepted target identifier.
//! - [`BOARD_AGENTS_MAX`]: most agents one focus board tracks.
//!
//! # Unsafe policy
//!
//! This crate forbids unsafe code outright.

#![forbid(unsafe_code)]

pub mod alignment;
pub mod board;
pub mod error;
pub mod focus;
pub mod saliency;
pub mod snapshot;

pub use alignment::{Alignment, Decision, alignment, decide};
pub use board::FocusBoard;
pub use error::AttentionError;
pub use focus::Focus;
pub use saliency::{SaliencyInput, saliency};
pub use snapshot::{Snapshot, TargetId};

/// Most targets one saliency snapshot may carry. A workspace larger
/// than this is indexed in pages by the caller; the snapshot itself
/// stays a bounded, hashable object.
pub const SNAPSHOT_TARGETS_MAX: usize = 4096;

/// Most targets one focus may advertise. A focus is an attention
/// headline, not a work plan: eight is enough to coordinate over and
/// small enough that flooding the channel is structurally useless.
pub const FOCUS_TARGETS_MAX: usize = 8;

/// Longest accepted target identifier, in bytes. Target ids are file
/// paths or `path#symbol` strings; anything longer is malformed or
/// hostile, never a real workspace object.
pub const TARGET_ID_MAX_BYTES: usize = 512;

/// Most distinct agents one focus board tracks. Beyond this the board
/// rejects new agents rather than evicting silently: a coordination
/// rendezvous that forgets participants is worse than one that refuses.
pub const BOARD_AGENTS_MAX: usize = 64;

/// Longest accepted agent identifier, in bytes.
pub const AGENT_ID_MAX_BYTES: usize = 128;
