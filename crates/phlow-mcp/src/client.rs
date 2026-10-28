//! Bounded MCP client for stdio servers (the outbound mirror of
//! the server side: [`crate::guard`] + [`crate::handler`]).
//!
//! phlow has always been an MCP *server*; this module is the
//! client half. It spawns a configured server process and offers
//! `tools/list` / `tools/call` with hard bounds on wait time and
//! profile shape. Everything synchronous at this surface (one
//! request in flight at a time, exactly like the runtime behind
//! the server loop); the async rmcp client runs on a private
//! runtime inside.
//!
//! Server registration is config-driven: a profile TOML file
//! lists `[[servers]]` entries (name, command, args, env). The
//! loader validates explicitly — unknown keys and missing
//! commands are errors, never defaults — mirroring phlow-config's
//! posture. Nothing here reads phlow's live operator config; a
//! profile is whatever file the caller names.
//!
//! Protocol posture (fleet standardization, D-113/GAP-4): the
//! client speaks the modern era only, on rmcp's client —
//! `server/discover` at connect, per-request `_meta` after. There
//! is no legacy `initialize` mode: a server that cannot discover
//! at 2026-07-28 is a negotiation error, never a silent downgrade
//! (the fleet's servers are all modern-only; fathom, this
//! client's named consumer target, migrated first).

use std::fmt;
use std::path::Path;
use std::time::Duration;

use rmcp::RoleClient;
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, ClientCapabilities, ClientConfig, ClientRequest,
    CustomRequest, Implementation, ListToolsRequest, ProtocolVersion,
};
use rmcp::service::{ClientInitializeError, PeerRequestOptions, RunningService, ServiceError};
use rmcp::transport::TokioChildProcess;
use serde_json::{Value, json};

use crate::protocol::MODERN_PROTOCOL_VERSION;

/// Largest single result the client will hand back, serialized
/// (fathom caps its own tool output at 8 MiB; this leaves
/// headroom, not infinity). The hand-rolled client enforced this
/// on raw frames; the SDK transport owns frame bounds now, so
/// the bound is enforced here, on the values this surface
/// returns.
pub const FRAME_BYTES_MAX: usize = 16 * 1024 * 1024;
/// Servers a single profile may register.
pub const PROFILE_SERVERS_MAX: usize = 32;
/// Arguments a single server entry may carry.
pub const SERVER_ARGS_MAX: usize = 64;
/// Default per-request wait. Callers driving slow tools (a
/// Ghidra-backed fathom call can legitimately take minutes) pass
/// a larger bound per call.
pub const RESPONSE_TIMEOUT_DEFAULT: Duration = Duration::from_secs(180);

/// One registered MCP server: how to launch it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    /// Registration name (unique within a profile by convention;
    /// the loader does not police duplicates — the caller picks
    /// one entry to spawn).
    pub name: String,
    /// Executable path or PATH-resolved name.
    pub command: String,
    /// Argument vector (may be empty).
    pub args: Vec<String>,
    /// Extra environment for the child (merged over the
    /// inherited environment).
    pub env: Vec<(String, String)>,
}

/// Failures of the client: profile, transport, protocol, or the
/// server reporting an error.
#[derive(Debug)]
pub enum ClientError {
    /// The profile file could not be read.
    Io(std::io::Error),
    /// The profile is not the documented shape.
    Profile(String),
    /// The server process could not be spawned or died.
    Spawn(String),
    /// The server does not speak the modern era (client offered,
    /// server answered, both recorded).
    Negotiation {
        /// Version the client offered.
        offered: String,
        /// Version the server answered.
        answered: String,
    },
    /// A frame violated JSON-RPC shape (bad JSON, wrong id,
    /// neither result nor error, oversized).
    Protocol(String),
    /// The server answered with a JSON-RPC error object.
    Rpc {
        /// JSON-RPC error code.
        code: i64,
        /// JSON-RPC error message.
        message: String,
    },
    /// A tool call returned `isError: true`; the payload is the
    /// tool's own error text.
    Tool(String),
    /// No response arrived within the caller's bound.
    Timeout(Duration),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::Io(error) => write!(f, "profile read failed: {error}"),
            ClientError::Profile(message) => write!(f, "invalid MCP profile: {message}"),
            ClientError::Spawn(message) => write!(f, "MCP server spawn failed: {message}"),
            ClientError::Negotiation { offered, answered } => write!(
                f,
                "protocol negotiation failed: offered {offered}, server answered {answered}"
            ),
            ClientError::Protocol(message) => write!(f, "MCP protocol violation: {message}"),
            ClientError::Rpc { code, message } => {
                write!(f, "MCP server error {code}: {message}")
            }
            ClientError::Tool(message) => write!(f, "MCP tool reported an error: {message}"),
            ClientError::Timeout(bound) => {
                write!(f, "no MCP response within {}s", bound.as_secs())
            }
        }
    }
}

impl std::error::Error for ClientError {}

impl From<std::io::Error> for ClientError {
    fn from(error: std::io::Error) -> ClientError {
        ClientError::Io(error)
    }
}

/// Load and validate a profile file. Shape:
///
/// ```toml
/// [[servers]]
/// name = "fathom"
/// command = "/path/to/fathom"
/// args = ["--config", "fathom.toml", "serve"]
/// [servers.env]
/// KEY = "value"
/// ```
///
/// (`[servers.env]` is per-entry: write it after each entry's
/// scalar keys, as TOML table order requires.)
pub fn load_profile(path: &Path) -> Result<Vec<McpServerConfig>, ClientError> {
    let text = std::fs::read_to_string(path)?;
    parse_profile(&text)
}

/// The parser behind [`load_profile`], split out for tests.
pub fn parse_profile(text: &str) -> Result<Vec<McpServerConfig>, ClientError> {
    let root: toml::Value = text
        .parse()
        .map_err(|e| ClientError::Profile(format!("not valid TOML: {e}")))?;
    let table = root
        .as_table()
        .ok_or_else(|| ClientError::Profile("profile root must be a table".to_string()))?;
    for key in table.keys() {
        if key != "servers" {
            return Err(ClientError::Profile(format!(
                "unknown profile key '{key}' (only 'servers' is defined)"
            )));
        }
    }
    let entries = table
        .get("servers")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| ClientError::Profile("profile must list [[servers]] entries".to_string()))?;
    if entries.len() > PROFILE_SERVERS_MAX {
        return Err(ClientError::Profile(format!(
            "profile lists {} servers; the bound is {PROFILE_SERVERS_MAX}",
            entries.len()
        )));
    }
    let mut servers = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        servers.push(parse_server_entry(index, entry)?);
    }
    Ok(servers)
}

fn parse_server_entry(index: usize, entry: &toml::Value) -> Result<McpServerConfig, ClientError> {
    let where_ = format!("servers[{index}]");
    let table = entry
        .as_table()
        .ok_or_else(|| ClientError::Profile(format!("{where_} must be a table")))?;
    for key in table.keys() {
        if !matches!(key.as_str(), "args" | "command" | "env" | "name") {
            return Err(ClientError::Profile(format!(
                "{where_} has unknown key '{key}'"
            )));
        }
    }
    let name = required_string(table, "name", &where_)?.to_string();
    if name.is_empty() {
        return Err(ClientError::Profile(format!(
            "{where_}.name must not be empty"
        )));
    }
    let command = required_string(table, "command", &where_)?.to_string();
    if command.is_empty() {
        return Err(ClientError::Profile(format!(
            "{where_}.command must not be empty"
        )));
    }
    let mut args = Vec::new();
    if let Some(raw_args) = table.get("args") {
        let list = raw_args.as_array().ok_or_else(|| {
            ClientError::Profile(format!("{where_}.args must be an array of strings"))
        })?;
        if list.len() > SERVER_ARGS_MAX {
            return Err(ClientError::Profile(format!(
                "{where_}.args lists {} items; the bound is {SERVER_ARGS_MAX}",
                list.len()
            )));
        }
        for item in list {
            let arg = item.as_str().ok_or_else(|| {
                ClientError::Profile(format!("{where_}.args must be an array of strings"))
            })?;
            args.push(arg.to_string());
        }
    }
    let mut env = Vec::new();
    if let Some(raw_env) = table.get("env") {
        let map = raw_env.as_table().ok_or_else(|| {
            ClientError::Profile(format!("{where_}.env must be a table of strings"))
        })?;
        for (key, value) in map {
            let value = value.as_str().ok_or_else(|| {
                ClientError::Profile(format!("{where_}.env.{key} must be a string"))
            })?;
            env.push((key.clone(), value.to_string()));
        }
    }
    Ok(McpServerConfig {
        name,
        command,
        args,
        env,
    })
}

fn required_string<'a>(
    table: &'a toml::Table,
    key: &str,
    where_: &str,
) -> Result<&'a str, ClientError> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .ok_or_else(|| ClientError::Profile(format!("{where_}.{key} must be a string")))
}

/// Map an rmcp service error onto the client's error surface,
/// preserving the distinctions callers could see before the
/// migration: RPC errors keep code+message, timeouts keep the
/// bound, a dead transport is a spawn-class failure.
fn service_error(error: ServiceError, bound: Duration) -> ClientError {
    match error {
        ServiceError::McpError(error) => ClientError::Rpc {
            code: i64::from(error.code.0),
            message: error.message.into_owned(),
        },
        ServiceError::Timeout { .. } => ClientError::Timeout(bound),
        ServiceError::TransportClosed | ServiceError::TransportSend(_) => {
            ClientError::Spawn(format!("server transport failed: {error}"))
        }
        other => ClientError::Protocol(other.to_string()),
    }
}

/// Enforce [`FRAME_BYTES_MAX`] on a value this surface hands back.
fn check_result_size(value: &Value) -> Result<(), ClientError> {
    let size = serde_json::to_string(value)
        .map(|text| text.len())
        .unwrap_or(0);
    if size > FRAME_BYTES_MAX {
        return Err(ClientError::Protocol(format!(
            "result exceeds the {FRAME_BYTES_MAX}-byte bound"
        )));
    }
    Ok(())
}

/// A live stdio session with one MCP server.
pub struct McpClient {
    runtime: Option<tokio::runtime::Runtime>,
    service: Option<RunningService<RoleClient, ClientConfig>>,
    /// The protocol version in force (the modern era this client
    /// discovered the server at).
    pub negotiated_version: String,
    /// `serverInfo.name` from the discover result, if reported.
    pub server_name: Option<String>,
}

impl McpClient {
    /// Spawn the server and discover it at the modern era.
    pub fn connect(config: &McpServerConfig, client_name: &str) -> Result<McpClient, ClientError> {
        let mut command = tokio::process::Command::new(&config.command);
        command.args(&config.args);
        for (key, value) in &config.env {
            command.env(key, value);
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| ClientError::Spawn(format!("client runtime: {e}")))?;
        // Spawning a tokio child process needs the reactor: enter
        // the runtime for the spawn (the runtime itself lives in
        // the returned client, keeping the child driver alive).
        let (transport, _stderr) = {
            let _enter = runtime.enter();
            TokioChildProcess::builder(command)
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| ClientError::Spawn(format!("{}: {e}", config.command)))?
        };
        let handler = ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new(client_name, env!("CARGO_PKG_VERSION")),
        );
        let service = runtime.block_on(async {
            use rmcp::service::ClientServiceExt;
            let connect = handler.serve_with_lifecycle(
                transport,
                rmcp::service::ClientLifecycleMode::Discover {
                    preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                },
            );
            match tokio::time::timeout(RESPONSE_TIMEOUT_DEFAULT, connect).await {
                Ok(result) => result.map_err(|e| {
                    use ClientInitializeError as InitError;
                    match e {
                        // The server named its versions and none
                        // is the modern era: a negotiation failure
                        // with both sides recorded.
                        InitError::NoCompatibleProtocolVersion {
                            server_supported, ..
                        } => ClientError::Negotiation {
                            offered: MODERN_PROTOCOL_VERSION.to_string(),
                            answered: server_supported
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join(", "),
                        },
                        // Discover itself was refused (a legacy
                        // server's method-not-found is the common
                        // case): also a negotiation failure — the
                        // refusal is the server's answer.
                        InitError::JsonRpcError(error) => ClientError::Negotiation {
                            offered: MODERN_PROTOCOL_VERSION.to_string(),
                            answered: format!(
                                "server/discover refused: {} {}",
                                error.code.0, error.message
                            ),
                        },
                        InitError::TransportError { .. } => {
                            ClientError::Spawn("server transport failed during connect".to_string())
                        }
                        other => ClientError::Protocol(format!("connect failed: {other}")),
                    }
                }),
                Err(_) => Err(ClientError::Timeout(RESPONSE_TIMEOUT_DEFAULT)),
            }
        })?;
        let info = service.peer().peer_info();
        let (negotiated_version, server_name) = match info.as_deref() {
            Some(info) => {
                let version = info.protocol_version.to_string();
                if version != MODERN_PROTOCOL_VERSION {
                    return Err(ClientError::Negotiation {
                        offered: MODERN_PROTOCOL_VERSION.to_string(),
                        answered: version,
                    });
                }
                (
                    version,
                    info.server_info.as_ref().map(|info| info.name.clone()),
                )
            }
            None => (MODERN_PROTOCOL_VERSION.to_string(), None),
        };
        Ok(McpClient {
            runtime: Some(runtime),
            service: Some(service),
            negotiated_version,
            server_name,
        })
    }

    /// Names of the tools the server offers.
    pub fn list_tools(&mut self) -> Result<Vec<String>, ClientError> {
        let result = self.send(
            ClientRequest::ListToolsRequest(ListToolsRequest {
                method: Default::default(),
                params: None,
                extensions: Default::default(),
            }),
            RESPONSE_TIMEOUT_DEFAULT,
        )?;
        let rmcp::model::ServerResult::ListToolsResult(result) = result else {
            return Err(ClientError::Protocol(
                "tools/list answered with an unexpected result type".to_string(),
            ));
        };
        let mut names = Vec::new();
        for tool in &result.tools {
            names.push(tool.name.clone().into_owned());
        }
        Ok(names)
    }

    /// Call one tool and return its first text content parsed as
    /// JSON — the shape evidence-envelope servers (fathom) use.
    /// A tool that answers non-JSON text yields that text wrapped
    /// as `{"text": ...}` rather than an error.
    pub fn call_tool(
        &mut self,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> Result<Value, ClientError> {
        let arguments = match arguments {
            Value::Object(map) => Some(map),
            Value::Null => None,
            _ => {
                return Err(ClientError::Protocol(
                    "tool arguments must be a JSON object".to_string(),
                ));
            }
        };
        let mut params = CallToolRequestParams::new(name.to_string());
        if let Some(arguments) = arguments {
            params = params.with_arguments(arguments);
        }
        let result = self.send(
            ClientRequest::CallToolRequest(CallToolRequest::new(params)),
            timeout,
        )?;
        let rmcp::model::ServerResult::CallToolResult(result) = result else {
            return Err(ClientError::Protocol(format!(
                "tool '{name}' answered with an unexpected result type"
            )));
        };
        let is_error = result.is_error.unwrap_or(false);
        let text = result
            .content
            .first()
            .and_then(|item| item.as_text())
            .map(|text| text.text.as_str())
            .ok_or_else(|| {
                ClientError::Protocol(format!("tool '{name}' result has no text content"))
            })?;
        if is_error {
            return Err(ClientError::Tool(text.to_string()));
        }
        let value = match serde_json::from_str::<Value>(text) {
            Ok(value) => value,
            Err(_) => json!({"text": text}),
        };
        check_result_size(&value)?;
        Ok(value)
    }

    /// One request/response round trip, bounded by `timeout`, for
    /// methods outside the typed tools surface. Returns the raw
    /// `result` value.
    pub fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, ClientError> {
        let result = self.send(
            ClientRequest::CustomRequest(CustomRequest::new(method, Some(params))),
            timeout,
        )?;
        let value = match result {
            rmcp::model::ServerResult::CustomResult(custom) => custom.0,
            other => serde_json::to_value(&other).unwrap_or(Value::Null),
        };
        check_result_size(&value)?;
        Ok(value)
    }

    /// Send one typed request through the running service.
    fn send(
        &mut self,
        request: ClientRequest,
        timeout: Duration,
    ) -> Result<rmcp::model::ServerResult, ClientError> {
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| ClientError::Spawn("client session is closed".to_string()))?;
        let service = self
            .service
            .as_ref()
            .ok_or_else(|| ClientError::Spawn("client session is closed".to_string()))?;
        let peer = service.peer().clone();
        runtime.block_on(async move {
            let handle = peer
                .send_request_with_option(request, PeerRequestOptions::with_timeout(timeout))
                .await
                .map_err(|e| service_error(e, timeout))?;
            handle
                .await_response()
                .await
                .map_err(|e| service_error(e, timeout))
        })
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // The session is over; the child must not outlive it. The
        // transport's own cleanup kills the child process when the
        // service drops; closing first gives it the chance to end
        // cleanly.
        if let (Some(runtime), Some(mut service)) = (self.runtime.take(), self.service.take()) {
            runtime.block_on(async move {
                let _ = service.close().await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE_OK: &str = r#"
[[servers]]
name = "fathom"
command = "/opt/fathom/fathom"
args = ["--config", "fathom.toml", "serve"]

[[servers]]
name = "other"
command = "other-server"
"#;

    #[test]
    fn validation_profile_parses_servers() {
        let servers = parse_profile(PROFILE_OK).unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name, "fathom");
        assert_eq!(servers[0].args.len(), 3);
        assert_eq!(servers[1].args.len(), 0);
        assert!(servers[1].env.is_empty());
    }

    #[test]
    fn adversarial_profile_rejects_missing_command() {
        let text = "[[servers]]\nname = \"x\"\n";
        assert!(parse_profile(text).is_err());
    }

    #[test]
    fn adversarial_profile_rejects_unknown_keys() {
        let root_key = "bogus = 1\n[[servers]]\nname = \"x\"\ncommand = \"y\"\n";
        assert!(parse_profile(root_key).is_err());
        let entry_key = "[[servers]]\nname = \"x\"\ncommand = \"y\"\nsurpise = 1\n";
        assert!(parse_profile(entry_key).is_err());
    }

    /// A scripted fake MCP server (python3, present wherever the
    /// workspace tests run) speaking the modern era: it answers
    /// `server/discover` at 2026-07-28 and serves tools/list +
    /// tools/call statelessly, the way migrated fleet servers do.
    fn fake_server_config(dir: &Path) -> McpServerConfig {
        let script = dir.join("fake_mcp.py");
        std::fs::write(
            &script,
            r#"
import json, sys
for line in sys.stdin:
    req = json.loads(line)
    method = req.get("method")
    if "id" not in req:
        continue
    if method == "server/discover":
        result = {"resultType": "complete",
                  "supportedVersions": ["2026-07-28"],
                  "capabilities": {"tools": {}},
                  "instructions": "fake",
                  "ttlMs": 3600000, "cacheScope": "public",
                  "_meta": {"io.modelcontextprotocol/serverInfo":
                            {"name": "fake", "version": "0"}}}
    elif method == "tools/list":
        result = {"resultType": "complete",
                  "tools": [{"name": "fake_tool", "description": "fake",
                             "inputSchema": {"type": "object"}}]}
    elif method == "tools/call":
        result = {"resultType": "complete",
                  "content": [{"type": "text", "text": "{\"ok\": true}"}],
                  "isError": False}
    else:
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": req["id"],
            "error": {"code": -32601, "message": "Method not found"}}) + "\n")
        sys.stdout.flush()
        continue
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": req["id"],
                                 "result": result}) + "\n")
    sys.stdout.flush()
"#,
        )
        .unwrap();
        McpServerConfig {
            name: "fake".to_string(),
            command: "python3".to_string(),
            args: vec![script.to_string_lossy().into_owned()],
            env: Vec::new(),
        }
    }

    #[test]
    fn validation_client_discovers_and_calls() {
        let dir = std::env::temp_dir().join(format!("phlow-mcp-client-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = fake_server_config(&dir);
        let mut client = McpClient::connect(&config, "phlow-test").unwrap();
        assert_eq!(client.negotiated_version, "2026-07-28");
        assert_eq!(client.server_name.as_deref(), Some("fake"));
        assert_eq!(client.list_tools().unwrap(), vec!["fake_tool".to_string()]);
        let value = client
            .call_tool("fake_tool", json!({}), Duration::from_secs(10))
            .unwrap();
        assert_eq!(value, json!({"ok": true}));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A legacy-only server (it knows `initialize`, nothing else)
    /// must be refused at connect: the modern-only client has no
    /// downgrade path (GAP-4).
    #[test]
    fn adversarial_client_refuses_a_legacy_only_server() {
        let dir =
            std::env::temp_dir().join(format!("phlow-mcp-client-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("legacy_mcp.py");
        std::fs::write(
            &script,
            r#"
import json, sys
for line in sys.stdin:
    req = json.loads(line)
    if "id" not in req:
        continue
    if req.get("method") == "initialize":
        result = {"protocolVersion": "2025-06-18", "capabilities": {},
                  "serverInfo": {"name": "legacy", "version": "0"}}
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": req["id"],
                                     "result": result}) + "\n")
    else:
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": req["id"],
            "error": {"code": -32601, "message": "Method not found"}}) + "\n")
    sys.stdout.flush()
"#,
        )
        .unwrap();
        let config = McpServerConfig {
            name: "legacy".to_string(),
            command: "python3".to_string(),
            args: vec![script.to_string_lossy().into_owned()],
            env: Vec::new(),
        };
        match McpClient::connect(&config, "phlow-test") {
            Err(ClientError::Negotiation { offered, .. }) => {
                assert_eq!(offered, "2026-07-28");
            }
            Err(other) => panic!("expected a negotiation refusal, got: {other}"),
            Ok(_) => panic!("a legacy-only server must not connect"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
