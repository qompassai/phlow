//! Bounded MCP client for stdio servers (the outbound mirror of
//! [`crate::server`]).
//!
//! phlow has always been an MCP *server*; this module is the
//! client half. It spawns a configured server process, speaks
//! newline-delimited JSON-RPC (the same framing the server side
//! uses), performs the legacy `initialize` handshake, and offers
//! `tools/list` / `tools/call` with hard bounds on frame size,
//! wait time, and profile shape. Everything synchronous: one
//! request in flight at a time, exactly like the runtime behind
//! the server loop.
//!
//! Server registration is config-driven: a profile TOML file
//! lists `[[servers]]` entries (name, command, args, env). The
//! loader validates explicitly — unknown keys and missing
//! commands are errors, never defaults — mirroring phlow-config's
//! posture. Nothing here reads phlow's live operator config; a
//! profile is whatever file the caller names.
//!
//! Protocol posture: the client offers
//! [`crate::protocol::PROTOCOL_VERSION`] and accepts any answer
//! in [`crate::protocol::SUPPORTED_PROTOCOL_VERSIONS`], recording
//! the negotiated version for the caller to inspect. A server
//! answering outside the supported set is a negotiation error,
//! never a silent downgrade.

use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::protocol::{PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS};

/// Largest single frame the client will read (fathom caps its
/// own tool output at 8 MiB; this leaves headroom, not infinity).
pub const FRAME_BYTES_MAX: usize = 16 * 1024 * 1024;
/// Servers a single profile may register.
pub const PROFILE_SERVERS_MAX: usize = 32;
/// Arguments a single server entry may carry.
pub const SERVER_ARGS_MAX: usize = 64;
/// Notifications/interleaved frames skipped while awaiting one
/// response before the wait is declared off the rails.
pub const SKIPPED_FRAMES_MAX: usize = 1024;
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
    /// The negotiated protocol version is outside the supported
    /// set (client offered, server answered, both recorded).
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

/// A live stdio session with one MCP server.
pub struct McpClient {
    child: Child,
    lines: mpsc::Receiver<std::io::Result<String>>,
    next_id: u64,
    /// The protocol version in force (the server's answer to
    /// `initialize`, validated against the supported set).
    pub negotiated_version: String,
    /// `serverInfo.name` from the initialize result, if reported.
    pub server_name: Option<String>,
    stdin: ChildStdin,
}

impl McpClient {
    /// Spawn the server and complete the `initialize` handshake.
    pub fn connect(config: &McpServerConfig, client_name: &str) -> Result<McpClient, ClientError> {
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (key, value) in &config.env {
            command.env(key, value);
        }
        let mut child = command
            .spawn()
            .map_err(|e| ClientError::Spawn(format!("{}: {e}", config.command)))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ClientError::Spawn("child stdin was not piped".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ClientError::Spawn("child stdout was not piped".to_string()))?;
        let (sender, lines) = mpsc::channel::<std::io::Result<String>>();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if sender.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
        });
        let mut client = McpClient {
            child,
            lines,
            next_id: 1,
            negotiated_version: String::new(),
            server_name: None,
            stdin,
        };
        let result = client.request(
            "initialize",
            json!({
                "capabilities": {},
                "clientInfo": {"name": client_name, "version": env!("CARGO_PKG_VERSION")},
                "protocolVersion": PROTOCOL_VERSION,
            }),
            RESPONSE_TIMEOUT_DEFAULT,
        )?;
        let answered = result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ClientError::Protocol("initialize result carries no protocolVersion".to_string())
            })?
            .to_string();
        if !SUPPORTED_PROTOCOL_VERSIONS.contains(&answered.as_str()) {
            return Err(ClientError::Negotiation {
                offered: PROTOCOL_VERSION.to_string(),
                answered,
            });
        }
        client.negotiated_version = answered;
        client.server_name = result
            .get("serverInfo")
            .and_then(|info| info.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string);
        client.notify("notifications/initialized", json!({}))?;
        Ok(client)
    }

    /// Names of the tools the server offers.
    pub fn list_tools(&mut self) -> Result<Vec<String>, ClientError> {
        let result = self.request("tools/list", json!({}), RESPONSE_TIMEOUT_DEFAULT)?;
        let tools = result
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ClientError::Protocol("tools/list result has no tools array".to_string())
            })?;
        let mut names = Vec::new();
        for tool in tools {
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| ClientError::Protocol("tool entry has no name".to_string()))?;
            names.push(name.to_string());
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
        let result = self.request(
            "tools/call",
            json!({"arguments": arguments, "name": name}),
            timeout,
        )?;
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let text = result
            .get("content")
            .and_then(Value::as_array)
            .and_then(|content| content.first())
            .and_then(|item| item.get("text"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ClientError::Protocol(format!("tool '{name}' result has no text content"))
            })?;
        if is_error {
            return Err(ClientError::Tool(text.to_string()));
        }
        match serde_json::from_str::<Value>(text) {
            Ok(value) => Ok(value),
            Err(_) => Ok(json!({"text": text})),
        }
    }

    /// One request/response round trip, bounded by `timeout`.
    pub fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, ClientError> {
        let id = self.next_id;
        self.next_id += 1;
        let frame = json!({
            "id": id,
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.send_frame(&frame)?;
        let mut skipped = 0_usize;
        loop {
            let line = match self.lines.recv_timeout(timeout) {
                Ok(Ok(line)) => line,
                Ok(Err(error)) => {
                    return Err(ClientError::Spawn(format!("server stdout failed: {error}")));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(ClientError::Timeout(timeout));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(ClientError::Spawn("server closed its stdout".to_string()));
                }
            };
            if line.len() > FRAME_BYTES_MAX {
                return Err(ClientError::Protocol(format!(
                    "frame exceeds the {FRAME_BYTES_MAX}-byte bound"
                )));
            }
            let message: Value = serde_json::from_str(&line)
                .map_err(|e| ClientError::Protocol(format!("frame is not valid JSON: {e}")))?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                // A notification or an interleaved frame: bounded
                // skipping, never an unbounded wait.
                skipped += 1;
                if skipped > SKIPPED_FRAMES_MAX {
                    return Err(ClientError::Protocol(format!(
                        "more than {SKIPPED_FRAMES_MAX} frames skipped awaiting id {id}"
                    )));
                }
                continue;
            }
            if let Some(error) = message.get("error") {
                let code = error.get("code").and_then(Value::as_i64).unwrap_or(-1);
                let message_text = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("no message")
                    .to_string();
                return Err(ClientError::Rpc {
                    code,
                    message: message_text,
                });
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), ClientError> {
        let frame = json!({"jsonrpc": "2.0", "method": method, "params": params});
        self.send_frame(&frame)
    }

    fn send_frame(&mut self, frame: &Value) -> Result<(), ClientError> {
        let mut text = serde_json::to_string(frame)
            .map_err(|e| ClientError::Protocol(format!("frame does not serialize: {e}")))?;
        text.push('\n');
        self.stdin
            .write_all(text.as_bytes())
            .and_then(|()| self.stdin.flush())
            .map_err(|e| ClientError::Spawn(format!("server stdin failed: {e}")))?;
        Ok(())
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // The session is over; the child must not outlive it.
        let _ = self.child.kill();
        let _ = self.child.wait();
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
    /// workspace tests run) answering the legacy handshake for
    /// protocol 2025-06-18 — the version fathom negotiates.
    fn fake_server_config(dir: &Path) -> McpServerConfig {
        let script = dir.join("fake_mcp.py");
        std::fs::write(
            &script,
            r#"
import json, sys
for line in sys.stdin:
    req = json.loads(line)
    method = req.get("method")
    if method == "initialize":
        result = {"protocolVersion": "2025-06-18",
                  "capabilities": {},
                  "serverInfo": {"name": "fake", "version": "0"}}
    elif method == "tools/list":
        result = {"tools": [{"name": "fake_tool"}]}
    elif method == "tools/call":
        result = {"content": [{"type": "text", "text": "{\"ok\": true}"}],
                  "isError": False}
    else:
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
    fn validation_client_negotiates_and_calls() {
        let dir = std::env::temp_dir().join(format!("phlow-mcp-client-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = fake_server_config(&dir);
        let mut client = McpClient::connect(&config, "phlow-test").unwrap();
        assert_eq!(client.negotiated_version, "2025-06-18");
        assert_eq!(client.server_name.as_deref(), Some("fake"));
        assert_eq!(client.list_tools().unwrap(), vec!["fake_tool".to_string()]);
        let value = client
            .call_tool("fake_tool", json!({}), Duration::from_secs(10))
            .unwrap();
        assert_eq!(value, json!({"ok": true}));
        std::fs::remove_dir_all(&dir).ok();
    }
}
