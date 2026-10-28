//! MCP protocol constants and the tool catalog.
//!
//! Every string, version, and error code here is part of the wire
//! contract with MCP clients. Do not rename tools, renumber codes,
//! or reword messages without a named, itemized delta.
//!
//! Since the rmcp migration (fleet standardization, D-113) the
//! server speaks exactly one era — the modern one (2026-07-28),
//! selected by per-request `_meta`. The legacy `initialize`
//! handshake era is refused by the era guard ([`crate::guard`]);
//! the envelope builders that era needed (initialize/discover
//! results, `RequestId`, envelope modernization) are gone with it
//! — the rmcp SDK and the guard own those shapes now. What stays
//! here is the catalog and the constants the guard, the handler,
//! and the domain dispatch share.

use rmcp::model::{Tool, ToolAnnotations};
use serde_json::{Map, Value, json};
use std::sync::{Arc, LazyLock};

/// Modern protocol version: every request carries `_meta` naming
/// the version, and there is no `initialize`/`notifications/initialized`
/// handshake. This is the only era the server implements.
pub const MODERN_PROTOCOL_VERSION: &str = "2026-07-28";
/// `_meta` key carrying the per-request protocol version. Required
/// on every modern request; its presence is what selects the
/// modern era.
pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
/// `_meta` key carrying the client's capabilities. Required on
/// every modern request.
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
/// `_meta` key carrying the client's identity. Attention tools
/// bind publishes to this name (anti-spoofing); a request without
/// it has no identity and attention calls fail closed.
pub const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
/// `_meta` key carrying the server's identity. The server puts it
/// in every modern result's `_meta`.
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";
/// Freshness hint, in milliseconds, for cacheable modern results:
/// the tool catalog and server identity are compile-time constants.
pub const CACHE_TTL_MS: u64 = 3_600_000;
/// Cache scope for modern list/discover results: the catalog
/// carries no user data, so shared intermediaries may cache it.
pub const CACHE_SCOPE: &str = "public";
/// Largest accepted frame: 1 MiB, matching the Python server.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
/// `serverInfo.name` is `"flow"` for compatibility with existing
/// clients, even though the repository is now named `phlow`.
pub const SERVER_NAME: &str = "flow";
/// Server version reported in `serverInfo`.
pub const SERVER_VERSION: &str = "0.2.0";
/// Instructions reported on discover.
pub const INSTRUCTIONS: &str = "Trusted workspace named checks only. Missing checks never verify.";

/// JSON-RPC parse error (malformed frame, non-finite number, over-deep nesting).
pub const PARSE_ERROR: i32 = -32700;
/// JSON-RPC invalid request (not an object, bad envelope, uncarryable id).
pub const INVALID_REQUEST: i32 = -32600;
/// JSON-RPC method not found.
pub const METHOD_NOT_FOUND: i32 = -32601;
/// JSON-RPC invalid params (bad tool args, unknown tools/call parameters).
pub const INVALID_PARAMS: i32 = -32602;
/// JSON-RPC internal error (defensive; the typed core has no panics to catch).
pub const INTERNAL_ERROR: i32 = -32603;

/// One catalog tool: its wire name, description, strict input
/// schema, and MCP annotations (declared from birth — the GAP-6
/// delta taken at the rmcp migration; the hand-rolled catalog
/// carried no annotations).
#[derive(Debug, Clone)]
pub struct ToolSpec {
    /// Wire name, e.g. `"flow_run"`.
    pub name: &'static str,
    /// Human description reported by `tools/list`.
    pub description: &'static str,
    /// Strict JSON Schema (`type: object`, `additionalProperties: false`).
    pub input_schema: Value,
    /// Behavior hints for clients: read/write class, destructiveness,
    /// idempotence, world openness. All four hints are always set.
    pub annotations: ToolAnnotations,
}

/// Annotations for a closed-world, read-only, idempotent tool
/// (status reads and board reads).
fn read_annotations() -> ToolAnnotations {
    ToolAnnotations::new()
        .read_only(true)
        .destructive(false)
        .idempotent(true)
        .open_world(false)
}

static TOOL_SPECS: LazyLock<Vec<ToolSpec>> = LazyLock::new(|| {
    vec![
        ToolSpec {
            name: "flow_run",
            description: "Bounded local planner/coder/reviewer run with real verification",
            input_schema: json!({
                "type": "object",
                "properties": {"task": {"type": "string"}},
                "required": ["task"],
                "additionalProperties": false,
            }),
            // The run's coder phase writes inside the workspace:
            // destructive updates are possible, a repeat call does
            // more work, and the world is the local workspace only.
            annotations: ToolAnnotations::new()
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(false),
        },
        ToolSpec {
            name: "flow_status",
            description: "Local health, configured models/checks, editor capabilities",
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false,
            }),
            annotations: read_annotations(),
        },
        ToolSpec {
            name: "flow_check",
            description: "Run operator-configured named checks; never arbitrary argv",
            input_schema: json!({
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "additionalProperties": false,
            }),
            // Checks execute operator-configured programs (not a
            // pure read), but their contract is verification:
            // additive-only, safe to re-run, local only.
            annotations: ToolAnnotations::new()
                .read_only(false)
                .destructive(false)
                .idempotent(true)
                .open_world(false),
        },
        ToolSpec {
            name: "flow_attention_alignment",
            description: "Alignment between your published focus and a partner agent's focus (same saliency snapshot only)",
            input_schema: json!({
                "type": "object",
                "properties": {"agent_id": {"type": "string"}},
                "required": ["agent_id"],
                "additionalProperties": false,
            }),
            annotations: read_annotations(),
        },
        ToolSpec {
            name: "flow_attention_decide",
            description: "Coordination decision against a partner agent's focus: parallel, serialize, or reject (stale frame)",
            input_schema: json!({
                "type": "object",
                "properties": {"agent_id": {"type": "string"}},
                "required": ["agent_id"],
                "additionalProperties": false,
            }),
            annotations: read_annotations(),
        },
        ToolSpec {
            name: "flow_attention_focus",
            description: "Read a partner agent's currently advertised focus from the coordination board",
            input_schema: json!({
                "type": "object",
                "properties": {"agent_id": {"type": "string"}},
                "required": ["agent_id"],
                "additionalProperties": false,
            }),
            annotations: read_annotations(),
        },
        ToolSpec {
            name: "flow_attention_publish",
            description: "Publish your attention focus (ordered salient targets + saliency snapshot hash) to the coordination board",
            input_schema: json!({
                "type": "object",
                "properties": {"focus": {
                    "type": "object",
                    "properties": {
                        "agent_id": {"type": "string"},
                        "seq": {"type": "integer"},
                        "snapshot_hash": {"type": "string"},
                        "targets": {"type": "array", "items": {"type": "string"}},
                    },
                    "required": ["agent_id", "seq", "snapshot_hash", "targets"],
                    "additionalProperties": false,
                }},
                "required": ["focus"],
                "additionalProperties": false,
            }),
            // Publishing mutates the board, additively; it is not
            // idempotent — the board refuses a non-advancing seq
            // (replay protection), so a repeat call is a refusal,
            // not a no-op.
            annotations: ToolAnnotations::new()
                .read_only(false)
                .destructive(false)
                .idempotent(false)
                .open_world(false),
        },
    ]
});

/// All catalog tools, in `tools/list` order.
pub fn tool_specs() -> &'static [ToolSpec] {
    &TOOL_SPECS
}

/// Look up a tool by wire name.
pub fn tool_spec(name: &str) -> Option<&'static ToolSpec> {
    TOOL_SPECS.iter().find(|spec| spec.name == name)
}

/// Convert one catalog entry to the rmcp [`Tool`] the handler
/// serves. Conversion cannot fail for the compiled-in catalog:
/// every `input_schema` is an object by construction.
pub fn rmcp_tool(spec: &ToolSpec) -> Tool {
    let schema = spec
        .input_schema
        .as_object()
        .expect("catalog input schemas are objects")
        .clone();
    let mut tool = Tool::new(spec.name, spec.description, Arc::new(schema));
    tool.annotations = Some(spec.annotations.clone());
    tool
}

/// The served tool surface, in catalog order.
pub fn rmcp_tools() -> Vec<Tool> {
    tool_specs().iter().map(rmcp_tool).collect()
}

/// One tool's `tools/list` entry as a plain value (the measurement
/// shape for tools/list size accounting in tests).
pub fn tool_value(spec: &ToolSpec) -> Value {
    let mut tool = Map::new();
    tool.insert("name".to_owned(), Value::String(spec.name.to_owned()));
    tool.insert(
        "description".to_owned(),
        Value::String(spec.description.to_owned()),
    );
    tool.insert("inputSchema".to_owned(), spec.input_schema.clone());
    tool.insert(
        "annotations".to_owned(),
        serde_json::to_value(&spec.annotations).expect("annotations serialize"),
    );
    Value::Object(tool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_catalog_is_the_seven_tool_surface() {
        // The first three tools mirror Python `flow serve`; the four
        // `flow_attention_*` tools are the Rust port's joint-attention
        // extension (crate `phlow-attention`) and have no Python twin.
        let specs = tool_specs();
        assert_eq!(
            specs.iter().map(|s| s.name).collect::<Vec<_>>(),
            [
                "flow_run",
                "flow_status",
                "flow_check",
                "flow_attention_alignment",
                "flow_attention_decide",
                "flow_attention_focus",
                "flow_attention_publish"
            ]
        );
        assert_eq!(
            tool_spec("flow_run").unwrap().description,
            specs[0].description
        );
        assert!(tool_spec("nope").is_none());
    }

    #[test]
    fn every_tool_carries_full_annotations() {
        // GAP-6: annotations from birth, all four hints explicit —
        // no tool leans on a client's default interpretation.
        for spec in tool_specs() {
            let a = &spec.annotations;
            assert!(a.read_only_hint.is_some(), "{}", spec.name);
            assert!(a.destructive_hint.is_some(), "{}", spec.name);
            assert!(a.idempotent_hint.is_some(), "{}", spec.name);
            assert!(a.open_world_hint.is_some(), "{}", spec.name);
            assert_eq!(a.open_world_hint, Some(false), "{}", spec.name);
        }
        let run = &tool_spec("flow_run").unwrap().annotations;
        assert_eq!(run.read_only_hint, Some(false));
        assert_eq!(run.destructive_hint, Some(true));
        let status = &tool_spec("flow_status").unwrap().annotations;
        assert_eq!(status.read_only_hint, Some(true));
        let publish = &tool_spec("flow_attention_publish").unwrap().annotations;
        assert_eq!(publish.read_only_hint, Some(false));
        assert_eq!(publish.idempotent_hint, Some(false));
    }

    #[test]
    fn rmcp_tools_match_the_catalog() {
        let tools = rmcp_tools();
        assert_eq!(tools.len(), 7);
        for (tool, spec) in tools.iter().zip(tool_specs()) {
            assert_eq!(tool.name, spec.name);
            assert_eq!(
                tool.description.as_deref(),
                Some(spec.description),
                "{}",
                spec.name
            );
            assert_eq!(
                Value::Object((*tool.input_schema).clone()),
                spec.input_schema,
                "{}",
                spec.name
            );
            assert_eq!(tool.annotations.as_ref(), Some(&spec.annotations));
        }
    }
}
