//! The tool-dispatch core: what a `tools/call` *does*.
//!
//! [`McpServer`] owns the agent runtime (behind [`McpRuntime`])
//! and the joint-attention board shared with the runtime's
//! dispatch gate. It is era-free: framing and era admission live
//! in [`crate::guard`], the rmcp SDK surface in [`crate::handler`].
//! What remains here is the dispatch semantics the hand-rolled
//! server's modern path implemented, preserved exactly:
//!
//! - arguments are validated against the catalog's strict
//!   schemas; failures are protocol refusals (`-32602`), never
//!   tool results
//! - runtime failures are tool results (`isError` with
//!   `{status: "error", error}`), never protocol errors
//! - the attention tools are served from the board, fail-closed:
//!   no caller identity, no focus, a stale or spoofed publish, or
//!   a cross-frame comparison is a tool error, never a fabricated
//!   success
//!
//! The agent runtime sits behind [`McpRuntime`]. Phase 3 ships the
//! synchronous trait; Phase 4 provides the async runtime and
//! reuses this dispatch unchanged.

use crate::error::RuntimeError;
use crate::protocol::{self, INTERNAL_ERROR, INVALID_PARAMS};
use crate::schema::validate_arguments;
use phlow_attention::{Focus, FocusBoard};
use serde_json::{Map, Value};
use std::sync::{Arc, Mutex};

/// The agent runtime behind the MCP server.
///
/// # stdout contract
///
/// Implementations must never write to stdout: the MCP transport
/// owns it and a single stray byte corrupts the client's frame
/// stream. Diagnostics go to stderr or a log. (`flow/mcp.py`
/// enforces this by redirecting the runtime's stdout to stderr
/// around every tool call.)
pub trait McpRuntime {
    /// Execute the bounded planner/coder/reviewer pipeline for `task`.
    fn run(&mut self, task: &str) -> Result<Value, RuntimeError>;
    /// Report workspace status.
    fn status(&mut self) -> Result<Value, RuntimeError>;
    /// Run one approved check by name, or all checks when `name` is `None`.
    fn check(&mut self, name: Option<&str>) -> Result<Value, RuntimeError>;
    /// Release runtime resources. Called exactly once when the serve
    /// loop ends, mirroring the `finally: self.runtime.close()` in
    /// `serve`.
    fn close(&mut self);
}

/// The outcome of one dispatched `tools/call`.
#[derive(Debug, Clone, PartialEq)]
pub enum CallOutcome {
    /// The tool executed. `result` is the tool's own payload
    /// object; `is_error` mirrors its `status` field (and is set
    /// for runtime failures, which arrive as
    /// `{status: "error", error}` payloads).
    Result {
        /// The tool payload (goes to `content` text +
        /// `structuredContent`).
        result: Value,
        /// Whether the call is an MCP tool-level error.
        is_error: bool,
    },
    /// A protocol-level refusal: the call never executed. Carries
    /// the JSON-RPC error code and message the hand-rolled
    /// dispatch produced for this case.
    Refusal {
        /// JSON-RPC error code (`-32602` for every current case).
        code: i32,
        /// The refusal message, verbatim from the legacy dispatch.
        message: String,
    },
}

impl CallOutcome {
    fn refusal(code: i32, message: impl Into<String>) -> CallOutcome {
        CallOutcome::Refusal {
            code,
            message: message.into(),
        }
    }
}

/// The MCP server core: tool dispatch over a runtime plus the
/// joint-attention coordination board behind the
/// `flow_attention_*` tools.
///
/// The board is shareable with the runtime's dispatch gate via
/// [`McpServer::set_attention_board`], so foci published through
/// the tools and the gate's `decide()` re-runs see ONE board
/// (FLOWGUARD-MAPPING B4).
pub struct McpServer<R: McpRuntime> {
    runtime: R,
    /// The joint-attention coordination board. Shareable with the
    /// runtime's dispatch gate via [`McpServer::set_attention_board`].
    board: Arc<Mutex<FocusBoard>>,
}

impl<R: McpRuntime> McpServer<R> {
    /// Create a server over `runtime`.
    pub fn new(runtime: R) -> McpServer<R> {
        McpServer {
            runtime,
            board: Arc::new(Mutex::new(FocusBoard::new())),
        }
    }

    /// Replace the attention board with a shared one, so the
    /// runtime's dispatch gate and the `flow_attention_*` tools
    /// see the same coordination state.
    pub fn set_attention_board(&mut self, board: Arc<Mutex<FocusBoard>>) {
        self.board = board;
    }

    /// The server's attention board (share it with a dispatch gate).
    pub fn attention_board(&self) -> Arc<Mutex<FocusBoard>> {
        Arc::clone(&self.board)
    }

    /// Access the wrapped runtime (for tests).
    pub fn runtime(&self) -> &R {
        &self.runtime
    }

    /// Access the wrapped runtime mutably (for tests).
    pub fn runtime_mut(&mut self) -> &mut R {
        &mut self.runtime
    }

    /// Close the wrapped runtime. The serve path calls this
    /// exactly once when serving ends, on every exit path.
    pub fn close(&mut self) {
        self.runtime.close();
    }

    /// Dispatch one `tools/call`: schema-validated arguments, then
    /// the tool itself.
    ///
    /// `caller` is the caller's attention identity — the modern
    /// `_meta` clientInfo name, extracted by the handler. `None`
    /// means no identity was established, and identity-bound
    /// attention calls fail closed.
    ///
    /// The strict-params refusal (unknown `tools/call` params
    /// keys) is *not* here: the SDK's typed dispatch drops unknown
    /// keys before any handler runs, so that refusal lives in the
    /// era guard, where the raw frame is still visible (GAP-2).
    pub fn call_tool(&mut self, name: &str, args: &Value, caller: Option<&str>) -> CallOutcome {
        let Some(spec) = protocol::tool_spec(name) else {
            return CallOutcome::refusal(INVALID_PARAMS, "Unknown Phlow tool");
        };
        if let Err(reason) = validate_arguments(args, &spec.input_schema) {
            return CallOutcome::refusal(INVALID_PARAMS, reason.to_string());
        }
        let args_obj = match phlow_json::object_map(args) {
            Ok(args_obj) => args_obj,
            Err(_) => {
                return CallOutcome::refusal(INVALID_PARAMS, "Arguments must be an object");
            }
        };
        self.execute_tool(name, args_obj, caller)
    }

    /// Dispatch one `flow_attention_*` tool against the shared
    /// board. Every handler is fail-closed (see the module docs).
    fn execute_attention(
        &mut self,
        name: &str,
        args: &Map<String, Value>,
        caller: Option<&str>,
    ) -> Result<Value, String> {
        let caller =
            caller.ok_or_else(|| "attention tools require an identified client".to_owned())?;
        let mut board = self.board.lock().expect("attention board lock");
        match name {
            "flow_attention_publish" => {
                let raw = args
                    .get("focus")
                    .ok_or_else(|| "focus argument is required".to_owned())?;
                let focus: Focus = serde_json::from_value(raw.clone())
                    .map_err(|error| format!("invalid focus: {error}"))?;
                board
                    .publish(caller, focus)
                    .map_err(|error| error.to_string())?;
                Ok(serde_json::json!({
                    "status": "ok",
                    "agent_id": caller,
                    "seq": raw.get("seq").cloned().unwrap_or(Value::Null),
                    "snapshot_hash": raw
                        .get("snapshot_hash")
                        .cloned()
                        .unwrap_or(Value::Null),
                }))
            }
            "flow_attention_focus" => {
                let agent_id = args
                    .get("agent_id")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| "agent_id argument is required".to_owned())?;
                let focus = board.get(agent_id).map(|focus| {
                    serde_json::to_value(focus).expect("focus serialization cannot fail")
                });
                Ok(serde_json::json!({"status": "ok", "focus": focus}))
            }
            "flow_attention_alignment" => {
                let partner_id = args
                    .get("agent_id")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| "agent_id argument is required".to_owned())?;
                let score = board
                    .alignment_between(caller, partner_id)
                    .ok_or_else(|| "both agents must publish a focus before alignment".to_owned())?
                    .map_err(|error| error.to_string())?;
                Ok(serde_json::json!({
                    "status": "ok",
                    "alignment": serde_json::to_value(score)
                        .expect("alignment serialization cannot fail"),
                }))
            }
            "flow_attention_decide" => {
                let partner_id = args
                    .get("agent_id")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| "agent_id argument is required".to_owned())?;
                let decision = board.decide_between(caller, partner_id).ok_or_else(|| {
                    "both agents must publish a focus before a decision".to_owned()
                })?;
                Ok(serde_json::json!({
                    "status": "ok",
                    "decision": serde_json::to_value(decision)
                        .expect("decision serialization cannot fail"),
                }))
            }
            _ => Err(format!("unknown attention tool: {name}")),
        }
    }

    /// Dispatch one validated tool call to the runtime (or the
    /// board) and wrap the outcome. `name` is a known tool and
    /// `args` passed schema validation.
    fn execute_tool(
        &mut self,
        name: &str,
        args: &Map<String, Value>,
        caller: Option<&str>,
    ) -> CallOutcome {
        if name.starts_with("flow_attention_") {
            // Attention tools are served from the coordination
            // board, not the runtime.
            return match self.execute_attention(name, args, caller) {
                Ok(result) => CallOutcome::Result {
                    result,
                    is_error: false,
                },
                Err(reason) => {
                    let failed = serde_json::json!({"status": "error", "error": reason});
                    CallOutcome::Result {
                        result: failed,
                        is_error: true,
                    }
                }
            };
        }
        // Defense in depth: the schema already enforces these
        // shapes, but the explicit checks preserve the Python
        // error precedence.
        let outcome = match name {
            "flow_run" => match args.get("task").and_then(|task| task.as_str()) {
                Some(task) => self.runtime.run(task),
                None => {
                    return CallOutcome::refusal(INVALID_PARAMS, "Task must be a string");
                }
            },
            "flow_check" => {
                let check_name = args.get("name").and_then(|name| name.as_str());
                if args.get("name").is_some() && check_name.is_none() {
                    return CallOutcome::refusal(INVALID_PARAMS, "Check name must be a string");
                }
                self.runtime.check(check_name)
            }
            "flow_status" => self.runtime.status(),
            _ => {
                return CallOutcome::refusal(INVALID_PARAMS, "Unknown Phlow tool");
            }
        };
        match outcome {
            Err(reason) => {
                // A runtime failure is a failed tool, not a
                // protocol error: isError with
                // {status: "error", error: text}.
                let failed = serde_json::json!({"status": "error", "error": reason.to_string()});
                CallOutcome::Result {
                    result: failed,
                    is_error: true,
                }
            }
            Ok(result) => {
                if !result.is_object() {
                    return CallOutcome::refusal(
                        INTERNAL_ERROR,
                        "Runtime returned a non-object result",
                    );
                }
                let is_error =
                    result.get("status").and_then(|status| status.as_str()) != Some("ok");
                CallOutcome::Result { result, is_error }
            }
        }
    }
}

/// A scripted [`McpRuntime`] for tests: canned results, recorded
/// calls, and an optional injected failure.
#[derive(Debug, Default)]
pub struct FakeRuntime {
    /// Result returned by `run`.
    pub run_result: Value,
    /// Result returned by `status`.
    pub status_result: Value,
    /// Result returned by `check`.
    pub check_result: Value,
    /// When set, every runtime call fails with this message.
    pub fail_with: Option<String>,
    /// `check` calls received (each `Option<String>` is the name argument).
    pub check_calls: Vec<Option<String>>,
    /// `run` calls received.
    pub run_calls: Vec<String>,
    /// Whether `close` was called.
    pub closed: bool,
}

impl FakeRuntime {
    /// A fake whose `run`/`status`/`check` all report
    /// `{"status": "ok", ...}`, matching the Python contract-test
    /// stub.
    pub fn ok() -> FakeRuntime {
        FakeRuntime {
            run_result: serde_json::json!({"status": "ok"}),
            status_result: serde_json::json!({"status": "ok"}),
            check_result: serde_json::json!({"status": "ok", "checks": []}),
            ..FakeRuntime::default()
        }
    }
}

impl McpRuntime for FakeRuntime {
    fn run(&mut self, task: &str) -> Result<Value, RuntimeError> {
        self.run_calls.push(task.to_owned());
        match &self.fail_with {
            Some(message) => Err(RuntimeError(message.clone())),
            None => Ok(self.run_result.clone()),
        }
    }

    fn status(&mut self) -> Result<Value, RuntimeError> {
        match &self.fail_with {
            Some(message) => Err(RuntimeError(message.clone())),
            None => Ok(self.status_result.clone()),
        }
    }

    fn check(&mut self, name: Option<&str>) -> Result<Value, RuntimeError> {
        self.check_calls.push(name.map(str::to_owned));
        match &self.fail_with {
            Some(message) => Err(RuntimeError(message.clone())),
            None => Ok(self.check_result.clone()),
        }
    }

    fn close(&mut self) {
        self.closed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> McpServer<FakeRuntime> {
        McpServer::new(FakeRuntime::ok())
    }

    fn focus(agent: &str, seq: u64, hash: &str, targets: &[&str]) -> Value {
        serde_json::json!({
            "agent_id": agent,
            "seq": seq,
            "snapshot_hash": hash,
            "targets": targets,
        })
    }

    fn result_of(outcome: CallOutcome) -> (Value, bool) {
        match outcome {
            CallOutcome::Result { result, is_error } => (result, is_error),
            other => panic!("expected a tool result, got {other:?}"),
        }
    }

    // --- validation: the runtime tools dispatch and wrap ---

    #[test]
    fn parity_a8_run_ok_result_is_not_error() {
        let mut server = server();
        let (result, is_error) =
            result_of(server.call_tool("flow_run", &serde_json::json!({"task": "do it"}), None));
        assert_eq!(result["status"], "ok");
        assert!(!is_error);
        assert_eq!(server.runtime().run_calls, ["do it"]);
    }

    #[test]
    fn parity_a8_non_ok_status_marks_error() {
        let mut fake = FakeRuntime::ok();
        fake.status_result = serde_json::json!({"status": "unavailable"});
        let mut server = McpServer::new(fake);
        let (_, is_error) =
            result_of(server.call_tool("flow_status", &serde_json::json!({}), None));
        assert!(is_error);
    }

    #[test]
    fn parity_a8_runtime_failure_is_a_tool_result_not_a_refusal() {
        let mut fake = FakeRuntime::ok();
        fake.fail_with = Some("boom".to_owned());
        let mut server = McpServer::new(fake);
        let (result, is_error) =
            result_of(server.call_tool("flow_status", &serde_json::json!({}), None));
        assert!(is_error);
        assert_eq!(result["status"], "error");
        assert_eq!(result["error"], "boom");
    }

    #[test]
    fn parity_c1_check_name_passes_through_to_the_runtime() {
        // C1: named-checks-only lives in the runtime; the dispatch
        // layer's job is to carry the name (and the result shape)
        // untouched.
        let mut server = server();
        let (result, _) =
            result_of(server.call_tool("flow_check", &serde_json::json!({"name": "unit"}), None));
        assert_eq!(result["status"], "ok");
        assert_eq!(server.runtime().check_calls, [Some("unit".to_owned())]);
        let mut server2 = self::server();
        let _ = result_of(server2.call_tool("flow_check", &serde_json::json!({}), None));
        assert_eq!(server2.runtime().check_calls, [None]);
    }

    // --- adversarial: refusals ---

    #[test]
    fn parity_a7_bad_args_are_refusals() {
        let mut server = server();
        for (name, args) in [
            // missing required
            ("flow_run", serde_json::json!({})),
            // wrong type
            ("flow_run", serde_json::json!({"task": 3})),
            // extra property against additionalProperties: false
            ("flow_status", serde_json::json!({"bogus": 1})),
            // arguments not an object at all
            ("flow_status", serde_json::json!([1, 2])),
        ] {
            match server.call_tool(name, &args, None) {
                CallOutcome::Refusal { code, .. } => {
                    assert_eq!(code, INVALID_PARAMS, "{name} {args}")
                }
                other => panic!("expected refusal for {name} {args}, got {other:?}"),
            }
        }
    }

    #[test]
    fn unknown_tool_is_a_refusal() {
        let mut server = server();
        match server.call_tool("shell_exec", &serde_json::json!({}), None) {
            CallOutcome::Refusal { code, message } => {
                assert_eq!(code, INVALID_PARAMS);
                assert_eq!(message, "Unknown Phlow tool");
            }
            other => panic!("expected refusal, got {other:?}"),
        }
    }

    // --- attention (B5): fail-closed, board-served ---

    #[test]
    fn parity_b5_attention_requires_an_identity() {
        let mut server = server();
        let (result, is_error) = result_of(server.call_tool(
            "flow_attention_publish",
            &serde_json::json!({"focus": focus("alice", 1, "h1", &["a"])}),
            None,
        ));
        assert!(is_error);
        assert_eq!(
            result["error"],
            "attention tools require an identified client"
        );
    }

    #[test]
    fn parity_b5_publish_focus_decide_alignment_flow() {
        let mut server = server();
        let (result, is_error) = result_of(server.call_tool(
            "flow_attention_publish",
            &serde_json::json!({"focus": focus("alice", 1, "h1", &["a", "b"])}),
            Some("alice"),
        ));
        assert!(!is_error);
        assert_eq!(result["status"], "ok");
        assert_eq!(result["agent_id"], "alice");

        let (_result, is_error) = result_of(server.call_tool(
            "flow_attention_publish",
            &serde_json::json!({"focus": focus("bob", 1, "h1", &["b", "c"])}),
            Some("bob"),
        ));
        assert!(!is_error);

        let (result, is_error) = result_of(server.call_tool(
            "flow_attention_focus",
            &serde_json::json!({"agent_id": "bob"}),
            Some("alice"),
        ));
        assert!(!is_error);
        assert_eq!(result["focus"]["agent_id"], "bob");

        let (result, is_error) = result_of(server.call_tool(
            "flow_attention_alignment",
            &serde_json::json!({"agent_id": "bob"}),
            Some("alice"),
        ));
        assert!(!is_error, "{result}");
        assert_eq!(result["status"], "ok");

        let (result, is_error) = result_of(server.call_tool(
            "flow_attention_decide",
            &serde_json::json!({"agent_id": "bob"}),
            Some("alice"),
        ));
        assert!(!is_error, "{result}");
        assert_eq!(result["status"], "ok");
        assert!(result["decision"].is_object());
    }

    #[test]
    fn parity_b5_spoofed_publish_is_a_tool_error() {
        // The payload's agent_id must equal the authenticated
        // caller; the board refuses the mismatch (anti-spoof).
        let mut server = server();
        let (result, is_error) = result_of(server.call_tool(
            "flow_attention_publish",
            &serde_json::json!({"focus": focus("mallory", 1, "h1", &["a"])}),
            Some("alice"),
        ));
        assert!(is_error);
        assert_eq!(result["status"], "error");
    }

    #[test]
    fn parity_b5_stale_sequence_replay_is_refused() {
        let mut server = server();
        let args = serde_json::json!({"focus": focus("alice", 1, "h1", &["a"])});
        let (_, is_error) =
            result_of(server.call_tool("flow_attention_publish", &args, Some("alice")));
        assert!(!is_error);
        let (result, is_error) =
            result_of(server.call_tool("flow_attention_publish", &args, Some("alice")));
        assert!(is_error, "{result}");
    }

    #[test]
    fn parity_b5_alignment_without_partner_focus_fails_closed() {
        let mut server = server();
        let _ = result_of(server.call_tool(
            "flow_attention_publish",
            &serde_json::json!({"focus": focus("alice", 1, "h1", &["a"])}),
            Some("alice"),
        ));
        let (result, is_error) = result_of(server.call_tool(
            "flow_attention_alignment",
            &serde_json::json!({"agent_id": "ghost"}),
            Some("alice"),
        ));
        assert!(is_error);
        assert!(
            result["error"]
                .as_str()
                .unwrap_or_default()
                .contains("both agents must publish"),
            "{result}"
        );
    }

    #[test]
    fn parity_b4_board_is_shared_with_the_gate() {
        // B4: a board handed in from outside (the runtime gate's)
        // is the same board the tools serve — publish through the
        // tool surface, read the focus back off the shared board.
        let shared = Arc::new(Mutex::new(FocusBoard::new()));
        let mut server = server();
        server.set_attention_board(Arc::clone(&shared));
        let _ = result_of(server.call_tool(
            "flow_attention_publish",
            &serde_json::json!({"focus": focus("alice", 1, "h1", &["a"])}),
            Some("alice"),
        ));
        let board = shared.lock().expect("board lock");
        assert!(board.get("alice").is_some());
    }
}
