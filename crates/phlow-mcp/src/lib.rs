//! Newline-delimited JSON-RPC server for the phlow agent runtime.
//!
//! This crate is a byte-faithful port of `flow/mcp.py`, extended to speak
//! the modern protocol alongside it. It is a dual-era server:
//!
//! * **Modern era** (`2026-07-28`): every request carries `_meta` with the
//!   protocol version and client capabilities. Requests are stateless — no
//!   `initialize` handshake. `server/discover` reports the supported
//!   versions, capabilities, and caching hints; every result carries
//!   `resultType` (`"complete"` here; this server never returns
//!   `"input_required"`) and `_meta` with `serverInfo`.
//! * **Legacy era** (`2025-11-25`, negotiating down from older supported
//!   versions): the classic `initialize` / `notifications/initialized`
//!   handshake gates tool calls, byte-identical to the Python server.
//!
//! The era is selected by how the client opens: a request whose params
//! carry modern `_meta` is served statelessly; `initialize` (or any
//! request without `_meta`) keeps legacy handshake semantics. An unknown
//! modern version gets `UnsupportedProtocolVersionError` (-32022) naming
//! the versions the server does implement.
//!
//! The same three compatibility tools (`flow_run`, `flow_status`,
//! `flow_check`), the same JSON-RPC error codes, and the same 1 MiB frame
//! cap apply in both eras.
//!
//! Wire compatibility notes:
//!
//! * Frames are serialized with [`json_ascii::dumps`], which replicates
//!   Python's `json.dumps(payload, ensure_ascii=True, allow_nan=False)`:
//!   `", "`/`": "` separators, ASCII-only escaping, insertion-ordered
//!   keys. Golden tests replay Python-produced frames byte-for-byte.
//! * Stdout carries *only* protocol JSON. The [`McpRuntime`] trait documents
//!   that implementations must not write to stdout; the server itself never
//!   logs there.
//! * The runtime behind the server is synchronous here ([`McpRuntime`]); the
//!   async runtime arrives in Phase 4 and reuses this framing core unchanged.
//!
//! Wire compatibility notes:
//!
//! * Frames are serialized with [`json_ascii::dumps`], which replicates
//!   Python's `json.dumps(payload, ensure_ascii=True, allow_nan=False)`:
//!   `", "`/`": "` separators, ASCII-only escaping, insertion-ordered
//!   keys. Golden tests replay Python-produced frames byte-for-byte.
//! * Stdout carries *only* protocol JSON. The [`McpRuntime`] trait documents
//!   that implementations must not write to stdout; the server itself never
//!   logs there.
//! * The runtime behind the server is synchronous here ([`McpRuntime`]); the
//!   async runtime arrives in Phase 4 and reuses this framing core unchanged.

#![forbid(unsafe_code)]

pub mod error;
pub mod json_ascii;
pub mod protocol;
pub mod schema;
pub mod server;

pub use error::{McpError, RuntimeError};
pub use json_ascii::{DumpsError, dumps};
pub use protocol::{
    CACHE_SCOPE, CACHE_TTL_MS, INSTRUCTIONS, INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST,
    MAX_FRAME_BYTES, META_CLIENT_CAPABILITIES, META_CLIENT_INFO, META_PROTOCOL_VERSION,
    META_SERVER_INFO, METHOD_NOT_FOUND, MODERN_PROTOCOL_VERSION, MODERN_SUPPORTED_VERSIONS,
    NOT_INITIALIZED, PARSE_ERROR, PROTOCOL_VERSION, SERVER_NAME, SERVER_VERSION,
    SUPPORTED_PROTOCOL_VERSIONS, ToolSpec, UNSUPPORTED_PROTOCOL_VERSION, tool_spec, tool_specs,
};
pub use schema::{SCHEMA_DEPTH_MAX, SCHEMA_NODES_MAX, SchemaError, validate_arguments};
pub use server::{FakeRuntime, FrameRead, McpRuntime, McpServer, ServeEnd};
