//! Agent2Agent (A2A) protocol interop for phlow, implementing
//! **A2A specification version 1.0.0** (a2a-protocol.org, read at the
//! primary source; see the program SPEC for the section map).
//!
//! Plain words: this crate lets phlow agents be tasked by other
//! agents and task other agents in return, in both directions over
//! the A2A JSON-RPC binding. It provides:
//!
//! - [`types`]: the wire model — Agent Card, Task, Message, Part,
//!   Artifact, and the streaming event shapes.
//! - [`store`]: the task lifecycle state machine, in-memory and
//!   bounded, with explicit handles (no hidden global state).
//! - [`server`]: a loopback-first HTTP/JSON-RPC server that serves
//!   the card at `/.well-known/agent-card.json` and the task
//!   operations, running an embedder-supplied executor per task.
//! - [`client`]: a synchronous client for tasking external agents.
//! - [`registry`]: external agents as policy destinations — sending
//!   to one is a flow-gated action, decided by `phlow-flowguard`
//!   before any bytes move.
//!
//! The server is a hand-rolled `std::net` listener (thread per
//! connection, bounded) rather than an async framework: phlow's core
//! is synchronous, the one async-shaped surface (SSE) is a bounded
//! blocking stream, and this keeps the whole A2A surface usable
//! without a runtime. The client posture mirrors phlow-runtime's
//! transport rules: proxies off, redirects off, TLS on, timeouts and
//! a streaming response cap.
//!
//! # The phlow overrides (where phlow's contracts beat the spec)
//!
//! - The server binds **loopback by default**; a non-loopback bind
//!   requires explicit configuration and per-agent bearer tokens.
//! - An A2A task grants **no** tool rights: executors reach tools
//!   only through the runtime's existing gated dispatch.
//! - Every inbound message joins its sender's declared label delta
//!   into the task trajectory, and every outbound send is decided by
//!   the flow gate *before* transport.
//! - Push notifications are not implemented; the card says so
//!   (`pushNotifications: false`) rather than implying support.
//!
//! # Bounds (Tiger Style: everything is capped, in advance)
//!
//! - [`TASKS_MAX`] tasks per store; [`HISTORY_MAX`] messages per
//!   task; [`ARTIFACTS_MAX`] artifacts per task.
//! - [`BODY_BYTES_MAX`] request bodies; [`PART_TEXT_BYTES_MAX`] per
//!   text part; [`PARTS_MAX`] parts per message.
//! - [`CONNECTIONS_MAX`] concurrent server connections;
//!   [`WORKERS_MAX`] concurrent task executions;
//!   [`SSE_LIFETIME_SECS`] per stream; [`RESPONSE_BYTES_MAX`] per
//!   client response.
//!
//! # Unsafe policy
//!
//! This crate forbids unsafe code outright.

#![forbid(unsafe_code)]

pub mod client;
pub mod registry;
pub mod server;
pub mod store;
pub mod types;
pub mod util;

pub use client::{A2aClient, A2aError};
pub use registry::{ExternalAgent, ExternalRegistry, RegistryError};
pub use server::{A2aServer, ExecCx, GateRejection, OutboundSink, ServerConfig, ServerHandle};
pub use store::{StoreError, TaskStore};
pub use types::{
    AgentCard, ListTasksParams, ListTasksResult, Message, Part, SendStreamingResponse, Task,
    TaskState,
};

/// Most tasks held by one store. A task is cheap; a flood is not.
pub const TASKS_MAX: usize = 1024;

/// Most messages retained in one task's history.
pub const HISTORY_MAX: usize = 128;

/// Most artifacts on one task.
pub const ARTIFACTS_MAX: usize = 64;

/// Most parts in one message.
pub const PARTS_MAX: usize = 64;

/// Largest accepted request body, in bytes (1 MiB).
pub const BODY_BYTES_MAX: usize = 1_048_576;

/// Largest accepted single text part, in bytes (256 KiB).
pub const PART_TEXT_BYTES_MAX: usize = 262_144;

/// Largest accepted header block, in bytes (16 KiB).
pub const HEADER_BYTES_MAX: usize = 16_384;

/// Most concurrent connections the server will hold.
pub const CONNECTIONS_MAX: usize = 32;

/// Most task executions running at once; further tasks are created
/// in the `rejected` state rather than queued without bound.
pub const WORKERS_MAX: usize = 16;

/// Longest lifetime of one server-sent-events stream, in seconds.
pub const SSE_LIFETIME_SECS: u64 = 300;

/// Largest client response body, in bytes (4 MiB).
pub const RESPONSE_BYTES_MAX: u64 = 4_194_304;

/// Most external agents in one registry.
pub const AGENTS_MAX: usize = 64;

/// The JSON-RPC path suffix the well-known card lives at.
pub const AGENT_CARD_PATH: &str = "/.well-known/agent-card.json";

/// A2A protocol version implemented by this crate.
pub const PROTOCOL_VERSION: &str = "1.0";
