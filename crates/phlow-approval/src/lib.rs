#![forbid(unsafe_code)]

//! Fail-closed approval and policy subsystem for phlow.
//!
//! Untrusted policies and requests enter as JSON and are narrowed against
//! closed schemas: unknown fields, unknown versions, malformed scopes and
//! oversized lists are rejected, never ignored or defaulted.
//!
//! ```text
//! Policy::from_json --> decide(policy, scope) --> Decision { verdict, reason, scope }
//!                                                     |
//!                                           Decision::proposal
//!                                                     v
//! Request::from_json --> ApprovalQueue::request --> Pending --decide(operator)--> Approved --revoke--> Revoked
//!                                                     |                      \--> Denied
//!                                                     +--deadline------------> Expired
//! ```
//!
//! Everything is in memory; nothing here persists, executes tools, or
//! authenticates humans beyond an explicit operator allowlist. Returned
//! records and events are owned snapshots, and transitions require
//! `&mut` access, so read handles cannot change published state:
//!
//! ```compile_fail
//! fn reader_approves(queue: &phlow_approval::ApprovalQueue, id: &str) {
//!     let verdict = phlow_approval::HumanVerdict::Approve;
//!     let _ = queue.decide(id, verdict, Some("operator"));
//! }
//! ```
//!
//! and a parsed policy has no writable state:
//!
//! ```compile_fail
//! fn agent_widens(policy: &mut phlow_approval::Policy) {
//!     policy.rules.clear();
//! }
//! ```

mod delta;
mod error;
mod events;
mod policy;
mod queue;
mod scope;

pub use delta::{PERMISSIONS_MAX, PermissionDelta, PermissionSet};
pub use error::Error;
pub use events::{
    EVENT_SOURCE, EVENTS_MAX, Envelope, Event, EventKind, EventPayload, EventSink, make_envelope,
};
pub use policy::{Decision, POLICY_VERSION, Policy, RULES_MAX, Verdict, decide};
pub use queue::{
    ApprovalQueue, DEFAULT_TTL, HumanVerdict, OPERATORS_MAX, QUEUE_RECORDS_MAX, RESERVED_ACTORS,
    Record, State, TTL_MAX,
};
pub use scope::{Request, Risk, SCOPE_ITEMS_MAX, SUMMARY_BYTES_MAX, Scope, TEXT_BYTES_MAX};
