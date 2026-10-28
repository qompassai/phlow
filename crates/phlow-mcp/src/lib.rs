//! MCP (Model Context Protocol) server and client for phlow.
//!
//! Two halves, one shape of rigour:
//!
//! * **Server** — [`McpServer`] is the era-free domain dispatch
//!   over an [`McpRuntime`]; [`PhlowHandler`] adapts it to rmcp;
//!   [`serve_stdio`] runs the modern era (2026-07-28 only) behind
//!   the byte-level admission guard in [`guard`]. The SDK types
//!   every message, so malformed input fails in the guard, never
//!   inside a handler.
//! * **Client** — [`McpClient`] spawns a configured MCP server
//!   over stdio and calls its tools, likewise modern-only, on
//!   rmcp's client.
//!
//! The shared rigour: one frame-size bound enforced at both
//! edges, errors that name what failed and nothing more, and a
//! per-tool JSON-Schema dialect ([`schema`]) whose strictness is
//! documented where it lives. Tool result *text* still uses the
//! phlow-canonical JSON spelling ([`dumps`]) so era-independent
//! bytes survive the migration unchanged. Stdout carries *only*
//! protocol JSON on the server side.

pub mod client;
pub mod error;
pub mod guard;
pub mod handler;
pub mod json_ascii;
pub mod protocol;
pub mod schema;
pub mod server;

pub use client::{
    ClientError, FRAME_BYTES_MAX, McpClient, McpServerConfig, load_profile, parse_profile,
};
pub use error::{McpError, RuntimeError};
pub use guard::{GuardAction, classify};
pub use handler::{PhlowHandler, serve_stdio};
pub use json_ascii::{DumpsError, dumps};
pub use protocol::{
    CACHE_SCOPE, CACHE_TTL_MS, INSTRUCTIONS, INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST,
    MAX_FRAME_BYTES, META_CLIENT_CAPABILITIES, META_CLIENT_INFO, META_PROTOCOL_VERSION,
    META_SERVER_INFO, METHOD_NOT_FOUND, MODERN_PROTOCOL_VERSION, PARSE_ERROR, SERVER_NAME,
    SERVER_VERSION, ToolSpec, rmcp_tool, rmcp_tools, tool_spec, tool_specs,
};
pub use schema::validate_arguments;
pub use server::{CallOutcome, FakeRuntime, McpRuntime, McpServer};
