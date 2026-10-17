//! The A2A server: a bounded, loopback-first HTTP/JSON-RPC
//! surface over the task store, with the flow gate inside the task
//! lifecycle.
//!
//! Plain words: an [`A2aServer`] owns a [`TaskStore`], a map of
//! per-task flowguard trajectories, and an embedder-supplied
//! [`AgentExecutor`]. Each connection is one thread; each task
//! execution is one worker thread, counted against
//! [`WORKERS_MAX`] — past the bound, new tasks are created directly
//! in the `rejected` state rather than queued without limit. The
//! server is hand-rolled `std::net` (see the crate docs for why).
//!
//! Security posture, enforced here and not left to configuration
//! discipline:
//!
//! - A non-loopback bind with no bearer tokens configured is a
//!   startup error.
//! - Caller identity comes from the transport (the bearer token),
//!   never from message payloads. Unknown or absent tokens classify
//!   the caller as `external-unvetted`: audience `self`, lowest
//!   trust, joined into the task's trajectory on receipt.
//! - When a policy is configured, every task carries a trajectory;
//!   inbound messages join the sender's declared source delta, and
//!   executor-initiated sends go through
//!   [`ExecCx::send_message_gated`], which is decided by the gate
//!   before the outbound sink is touched.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use phlow_attention::{Focus, FocusBoard};
use phlow_flowguard::event::{JoinSpec, ProposedAction};
use phlow_flowguard::gate::{BlockInfo, BlockReason, Decision};
use phlow_flowguard::label::LabelDelta;
use phlow_flowguard::{Policy, Trajectory};

use crate::store::{StoreError, TaskStore};
use crate::types::{
    AgentCard, GetTaskParams, ListTasksParams, Message, Part, Role, SendMessageParams,
    SendStreamingResponse, Task, TaskArtifactUpdateEvent, TaskIdParams, TaskState,
    TaskStatusUpdateEvent,
};
use crate::util::{boot_seed, mint_id, rfc3339_now};
use crate::{
    AGENT_CARD_PATH, BODY_BYTES_MAX, CONNECTIONS_MAX, HEADER_BYTES_MAX, SSE_LIFETIME_SECS,
    WORKERS_MAX,
};

/// The unvetted-caller source name used when a caller has no policy
/// source declaration.
pub const UNVETTED_CALLER: &str = "external-unvetted";

/// Server configuration. The defaults are the safe ones: loopback
/// bind, no tokens (loopback-only), no policy (trajectories off),
/// no attention board, no outbound sink.
pub struct ServerConfig {
    /// Bind address. Default: `127.0.0.1:0` (loopback, ephemeral
    /// port). A non-loopback ip requires `tokens` to be non-empty.
    pub bind: SocketAddr,
    /// This agent's own id: its identity on the attention board and
    /// the `from` name recorded for its outbound messages.
    pub agent_id: String,
    /// Bearer token → caller agent name. When non-empty, requests
    /// must present a known token; the mapped name is the caller
    /// identity for flow classification.
    pub tokens: HashMap<String, String>,
    /// The flow policy. When set, every task gets a trajectory and
    /// executor sends are gated.
    pub policy: Option<Arc<Policy>>,
    /// The shared attention board, when this agent participates in
    /// joint attention.
    pub board: Option<Arc<Mutex<FocusBoard>>>,
    /// Where gated outbound messages are delivered.
    pub outbound: Option<Arc<dyn OutboundSink>>,
    /// The card returned by `GetExtendedAgentCard`, when the agent
    /// distinguishes an extended card. Defaults to the base card.
    pub extended_card: Option<AgentCard>,
    /// How long a `SendMessage` request waits for the executor
    /// before returning the task in its current (working) state.
    /// The task itself continues.
    pub request_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            agent_id: "phlow-a2a".to_string(),
            tokens: HashMap::new(),
            policy: None,
            board: None,
            outbound: None,
            extended_card: None,
            request_timeout: Duration::from_secs(120),
        }
    }
}

/// Delivery for gate-approved outbound messages. The registry's
/// A2A client implements this for external agents; embedders may
/// deliver to in-process agents instead.
pub trait OutboundSink: Send + Sync {
    /// Deliver `message` to the policy destination named
    /// `destination`. Called only after the gate allowed the send.
    fn deliver(&self, destination: &str, message: &Message) -> Result<(), String>;
}

/// The behaviour an A2A server executes per task. Implementations
/// are the agent's own logic; everything A2A-shaped (lifecycle,
/// streaming, gating) is the server's.
pub trait AgentExecutor: Send + Sync {
    /// Run one task to completion (or to an input-required pause
    /// via [`ExecCx::require_input`]). Returning artifacts completes
    /// the task with them; returning an error fails it with the
    /// message. Progress is reported through `cx`.
    fn execute(
        &self,
        cx: &mut ExecCx,
        message: &Message,
    ) -> Result<Vec<crate::types::Artifact>, String>;
}

/// A gate block, handed back to an executor that attempted a gated
/// send. The block carries the full remedy plan; a common executor
/// response is to surface it to the caller as an input-required
/// status message (the plan rides as a data part).
#[derive(Clone, Debug)]
pub struct GateRejection(pub BlockInfo);

impl std::fmt::Display for GateRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "flow gate blocked the send: {:?}",
            self.0.reasons
        )
    }
}

impl std::error::Error for GateRejection {}

/// The executor's handle on the running task: status, artifacts,
/// cancellation, the attention board, and gated sending.
pub struct ExecCx {
    shared: Arc<Shared>,
    /// The running task's id.
    pub task_id: String,
    /// The running task's context id.
    pub context_id: String,
    cancel_flag: Arc<AtomicBool>,
    input_required: Option<Message>,
}

impl ExecCx {
    /// Has cancellation been requested for this task?
    pub fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::SeqCst)
    }

    /// The shared attention board, when configured.
    pub fn board(&self) -> Option<Arc<Mutex<FocusBoard>>> {
        self.shared.board.clone()
    }

    /// Publish this agent's focus to the shared board. The board
    /// itself verifies that the focus's identity matches this
    /// server's agent id.
    pub fn publish_focus(&self, focus: Focus) -> Result<(), String> {
        let board = self
            .shared
            .board
            .as_ref()
            .ok_or("no attention board configured")?;
        let mut board = board.lock().map_err(|_| "attention board lock poisoned")?;
        board
            .publish(&self.shared.agent_id, focus)
            .map_err(|err| err.to_string())
    }

    /// Add an artifact to the task and stream the update.
    pub fn add_artifact(&mut self, artifact: crate::types::Artifact) -> Result<(), String> {
        let stored = {
            let mut store = self
                .shared
                .store
                .lock()
                .map_err(|_| "store lock poisoned")?;
            store
                .add_artifact(&self.task_id, artifact)
                .map_err(|err| err.to_string())?
        };
        broadcast(
            &self.shared,
            &self.task_id,
            SendStreamingResponse::ArtifactUpdate(TaskArtifactUpdateEvent {
                task_id: self.task_id.clone(),
                context_id: self.context_id.clone(),
                artifact: stored,
                append: None,
                last_chunk: None,
            }),
        );
        Ok(())
    }

    /// Pause the task for more input: after `execute` returns, the
    /// task moves to `input-required` with this message attached.
    pub fn require_input(&mut self, message: Message) {
        self.input_required = Some(message);
    }

    /// The task trajectory's current label snapshot, when a policy
    /// is configured.
    pub fn trajectory_snapshot(&self) -> Option<phlow_flowguard::label::LabelSnapshot> {
        self.shared.policy.as_ref()?;
        let trajectories = self.shared.trajectories.lock().ok()?;
        let trajectory = trajectories.get(&self.task_id)?;
        trajectory.snapshot().ok()
    }

    /// Send a message to a policy destination, gated: the task's
    /// trajectory proposes the send, the gate decides, and only an
    /// allow reaches the outbound sink.
    ///
    /// # Errors
    ///
    /// [`GateRejection`] when the gate blocks (the block is also
    /// recorded in the trajectory, so it is auditable and the remedy
    /// plan is in the rejection); `String` for delivery failures.
    pub fn send_message_gated(
        &mut self,
        destination: &str,
        parts: Vec<Part>,
    ) -> Result<Message, GateRejection> {
        let message = Message {
            message_id: self.shared.mint_id("msg"),
            context_id: Some(self.context_id.clone()),
            task_id: Some(self.task_id.clone()),
            role: Role::Agent,
            parts,
            reference_task_ids: Vec::new(),
            extensions: Vec::new(),
            metadata: None,
        };
        let Some(_policy) = self.shared.policy.as_ref() else {
            return Err(GateRejection(BlockInfo {
                digest: String::new(),
                reasons: vec![BlockReason::PolicyMismatch {
                    detail: "no flow policy configured on this server".to_string(),
                }],
                remedies: phlow_flowguard::gate::RemedyPlan::default(),
            }));
        };
        let decision = {
            let mut trajectories = self.shared.trajectories.lock().map_err(|_| {
                GateRejection(BlockInfo {
                    digest: String::new(),
                    reasons: vec![BlockReason::PolicyMismatch {
                        detail: "trajectory lock poisoned".to_string(),
                    }],
                    remedies: phlow_flowguard::gate::RemedyPlan::default(),
                })
            })?;
            let trajectory = trajectories.get_mut(&self.task_id).ok_or_else(|| {
                GateRejection(BlockInfo {
                    digest: String::new(),
                    reasons: vec![BlockReason::PolicyMismatch {
                        detail: "task has no trajectory".to_string(),
                    }],
                    remedies: phlow_flowguard::gate::RemedyPlan::default(),
                })
            })?;
            trajectory
                .propose(&ProposedAction::Message {
                    destination: destination.to_string(),
                    payload_label: None,
                })
                .map_err(|err| {
                    GateRejection(BlockInfo {
                        digest: String::new(),
                        reasons: vec![BlockReason::PolicyMismatch {
                            detail: err.to_string(),
                        }],
                        remedies: phlow_flowguard::gate::RemedyPlan::default(),
                    })
                })?
        };
        match decision {
            Decision::Allow(_) => {
                let sink = self.shared.outbound.as_ref().ok_or_else(|| {
                    GateRejection(BlockInfo {
                        digest: String::new(),
                        reasons: vec![BlockReason::PolicyMismatch {
                            detail: "gate allowed the send but no outbound sink is configured"
                                .to_string(),
                        }],
                        remedies: phlow_flowguard::gate::RemedyPlan::default(),
                    })
                })?;
                sink.deliver(destination, &message).map_err(|err| {
                    GateRejection(BlockInfo {
                        digest: String::new(),
                        reasons: vec![BlockReason::PolicyMismatch { detail: err }],
                        remedies: phlow_flowguard::gate::RemedyPlan::default(),
                    })
                })?;
                Ok(message)
            }
            Decision::Block(info) => Err(GateRejection(info)),
        }
    }
}

struct Shared {
    addr: SocketAddr,
    card: AgentCard,
    extended_card: Option<AgentCard>,
    agent_id: String,
    tokens: HashMap<String, String>,
    policy: Option<Arc<Policy>>,
    board: Option<Arc<Mutex<FocusBoard>>>,
    outbound: Option<Arc<dyn OutboundSink>>,
    request_timeout: Duration,
    executor: Arc<dyn AgentExecutor>,
    store: Mutex<TaskStore>,
    trajectories: Mutex<HashMap<String, Trajectory>>,
    subscribers: Mutex<HashMap<String, Vec<mpsc::Sender<SendStreamingResponse>>>>,
    workers_active: AtomicUsize,
    connections_active: AtomicUsize,
    seed: u64,
    id_counter: AtomicU64,
}

impl Shared {
    fn mint_id(&self, kind: &str) -> String {
        let counter = self.id_counter.fetch_add(1, Ordering::SeqCst) + 1;
        mint_id(kind, self.seed, counter)
    }
}

/// A running server. Dropping the handle does not stop the server;
/// call [`ServerHandle::stop`].
pub struct A2aServer;

impl A2aServer {
    /// Bind and start serving. Returns the handle with the actual
    /// bound address (relevant when the configured port is 0).
    ///
    /// # Errors
    ///
    /// A string describing the startup failure: a non-loopback bind
    /// without tokens, or the OS bind error.
    pub fn start(
        config: ServerConfig,
        card: AgentCard,
        executor: Arc<dyn AgentExecutor>,
    ) -> Result<ServerHandle, String> {
        if !config.bind.ip().is_loopback() && config.tokens.is_empty() {
            return Err(
                "refusing a non-loopback A2A bind without per-agent bearer tokens \
                 (phlow override: external endpoints are explicit opt-in)"
                    .to_string(),
            );
        }
        let listener =
            TcpListener::bind(config.bind).map_err(|err| format!("bind {}: {err}", config.bind))?;
        let addr = listener
            .local_addr()
            .map_err(|err| format!("local_addr: {err}"))?;
        let shared = Arc::new(Shared {
            addr,
            card,
            extended_card: config.extended_card,
            agent_id: config.agent_id,
            tokens: config.tokens,
            policy: config.policy,
            board: config.board,
            outbound: config.outbound,
            request_timeout: config.request_timeout,
            executor,
            store: Mutex::new(TaskStore::new(boot_seed("phlow-a2a-store"))),
            trajectories: Mutex::new(HashMap::new()),
            subscribers: Mutex::new(HashMap::new()),
            workers_active: AtomicUsize::new(0),
            connections_active: AtomicUsize::new(0),
            seed: boot_seed("phlow-a2a-ids"),
            id_counter: AtomicU64::new(0),
        });
        let shutdown = Arc::new(AtomicBool::new(false));
        let accept_shared = shared.clone();
        let accept_shutdown = shutdown.clone();
        let join = std::thread::spawn(move || {
            accept_loop(listener, accept_shared, accept_shutdown);
        });
        Ok(ServerHandle {
            addr,
            shutdown,
            join: Some(join),
        })
    }
}

/// Handle to a running server.
pub struct ServerHandle {
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl ServerHandle {
    /// The address the server actually bound.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The interface URL clients should use (loopback form).
    pub fn url(&self) -> String {
        format!("http://{}/", self.addr)
    }

    /// Stop accepting, wake the accept loop, and join it. In-flight
    /// connections and tasks finish on their own threads.
    pub fn stop(mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>, shutdown: Arc<AtomicBool>) {
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if shutdown.load(Ordering::SeqCst) {
                    return;
                }
                if shared.connections_active.load(Ordering::SeqCst) >= CONNECTIONS_MAX {
                    let mut stream = stream;
                    let _ = write_http(
                        &mut stream,
                        503,
                        "application/json",
                        b"{\"error\":\"server at connection capacity\"}",
                    );
                    continue;
                }
                shared.connections_active.fetch_add(1, Ordering::SeqCst);
                let conn_shared = shared.clone();
                std::thread::spawn(move || {
                    let _ = handle_connection(conn_shared.clone(), stream);
                    conn_shared
                        .connections_active
                        .fetch_sub(1, Ordering::SeqCst);
                });
            }
            Err(_) => {
                if shutdown.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

// ---------------------------------------------------------------------
// HTTP plumbing (deliberately minimal: one request per connection).

struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> Result<Request, i32> {
    let mut buffer = Vec::with_capacity(8192);
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = find_subslice(&buffer, b"\r\n\r\n") {
            break pos + 4;
        }
        if buffer.len() > HEADER_BYTES_MAX {
            return Err(431);
        }
        let read = stream.read(&mut chunk).map_err(|_| 400)?;
        if read == 0 {
            return Err(400);
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next().ok_or(400)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(400)?.to_string();
    let path = parts.next().ok_or(400)?.to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let mut body = buffer.split_off(header_end);
    if method == "POST" {
        let length: usize = headers
            .get("content-length")
            .and_then(|value| value.parse().ok())
            .ok_or(411)?;
        if length > BODY_BYTES_MAX {
            return Err(413);
        }
        while body.len() < length {
            let read = stream.read(&mut chunk).map_err(|_| 400)?;
            if read == 0 {
                return Err(400);
            }
            body.extend_from_slice(&chunk[..read]);
        }
        body.truncate(length);
    }
    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn write_http(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Content Too Large",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

// ---------------------------------------------------------------------
// Request handling.

fn handle_connection(shared: Arc<Shared>, mut stream: TcpStream) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let request = match read_request(&mut stream) {
        Ok(request) => request,
        Err(status) => {
            return write_http(
                &mut stream,
                u16::try_from(status).unwrap_or(400),
                "text/plain",
                b"malformed request",
            );
        }
    };
    if request.method == "GET" {
        if request.path == AGENT_CARD_PATH {
            let card = served_card(&shared);
            let body = serde_json::to_vec(&card).unwrap_or_default();
            return write_http(&mut stream, 200, "application/json", &body);
        }
        return write_http(
            &mut stream,
            404,
            "application/json",
            b"{\"error\":\"not found\"}",
        );
    }
    if request.method != "POST" {
        return write_http(
            &mut stream,
            405,
            "application/json",
            b"{\"error\":\"method not allowed\"}",
        );
    }
    // Caller identity from the transport only.
    let caller = if shared.tokens.is_empty() {
        None
    } else {
        let presented = request
            .headers
            .get("authorization")
            .and_then(|value| value.strip_prefix("Bearer "));
        match presented.and_then(|token| shared.tokens.get(token)) {
            Some(name) => Some(name.clone()),
            None => {
                let body = serde_json::json!({
                    "jsonrpc": "2.0", "id": null,
                    "error": {"code": -32099, "message": "unauthorized: missing or unknown bearer token"}
                });
                return write_http(
                    &mut stream,
                    401,
                    "application/json",
                    body.to_string().as_bytes(),
                );
            }
        }
    };
    let parsed: serde_json::Value = match serde_json::from_slice(&request.body) {
        Ok(value) => value,
        Err(err) => {
            return write_rpc_error(
                &mut stream,
                serde_json::Value::Null,
                -32700,
                &format!("parse error: {err}"),
                None,
            );
        }
    };
    let id = parsed.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let Some(method) = parsed.get("method").and_then(serde_json::Value::as_str) else {
        return write_rpc_error(
            &mut stream,
            id,
            -32600,
            "invalid request: method must be a string",
            None,
        );
    };
    let params = parsed
        .get("params")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    match method {
        "SendStreamingMessage" | "message/stream" => {
            handle_send(shared, &mut stream, &id, params, caller.as_deref(), true)
        }
        "SendMessage" | "message/send" => {
            handle_send(shared, &mut stream, &id, params, caller.as_deref(), false)
        }
        "SubscribeToTask" | "tasks/resubscribe" => {
            handle_subscribe(shared, &mut stream, &id, params)
        }
        "GetTask" | "tasks/get" => {
            let result = rpc_get_task(&shared, params);
            write_rpc_result(&mut stream, &id, result)
        }
        "ListTasks" | "tasks/list" => {
            let result = rpc_list_tasks(&shared, params);
            write_rpc_result(&mut stream, &id, result)
        }
        "CancelTask" | "tasks/cancel" => {
            let result = rpc_cancel_task(&shared, params);
            write_rpc_result(&mut stream, &id, result)
        }
        "GetExtendedAgentCard" | "agent/getAuthenticatedExtendedCard" => {
            let card = shared
                .extended_card
                .clone()
                .unwrap_or_else(|| served_card(&shared));
            let value = serde_json::to_value(&card).unwrap_or(serde_json::Value::Null);
            write_rpc_value(&mut stream, &id, Ok(value))
        }
        other => write_rpc_error(
            &mut stream,
            id,
            -32601,
            &format!("method not found: {other}"),
            None,
        ),
    }
}

/// The card as served: the declared card with any interface whose
/// authority is ephemeral (`:0` port or the unspecified address)
/// rewritten to the bound address, so discovery always yields a
/// reachable URL.
fn served_card(shared: &Shared) -> AgentCard {
    let mut card = shared.card.clone();
    for iface in &mut card.supported_interfaces {
        if let Some((scheme, rest)) = iface.url.split_once("://") {
            let (authority, path) = match rest.split_once('/') {
                Some((authority, path)) => (authority, format!("/{path}")),
                None => (rest, String::new()),
            };
            if authority.ends_with(":0") || authority.starts_with("0.0.0.0") {
                iface.url = format!("{scheme}://{}{path}", shared.addr);
            }
        }
    }
    card
}

fn write_rpc_result(
    stream: &mut TcpStream,
    id: &serde_json::Value,
    result: Result<serde_json::Value, (i32, String, Option<serde_json::Value>)>,
) -> std::io::Result<()> {
    write_rpc_value(stream, id, result)
}

fn write_rpc_value(
    stream: &mut TcpStream,
    id: &serde_json::Value,
    result: Result<serde_json::Value, (i32, String, Option<serde_json::Value>)>,
) -> std::io::Result<()> {
    match result {
        Ok(value) => {
            let body = serde_json::json!({"jsonrpc": "2.0", "id": id, "result": value});
            write_http(stream, 200, "application/json", body.to_string().as_bytes())
        }
        Err((code, message, data)) => write_rpc_error(stream, id.clone(), code, &message, data),
    }
}

fn write_rpc_error(
    stream: &mut TcpStream,
    id: serde_json::Value,
    code: i32,
    message: &str,
    data: Option<serde_json::Value>,
) -> std::io::Result<()> {
    let mut error = serde_json::json!({"code": code, "message": message});
    if let Some(data) = data {
        error["data"] = data;
    }
    let body = serde_json::json!({"jsonrpc": "2.0", "id": id, "error": error});
    write_http(stream, 200, "application/json", body.to_string().as_bytes())
}

/// (code, message, data) — the JSON-RPC error triple.
type RpcError = (i32, String, Option<serde_json::Value>);
type RpcResult = Result<serde_json::Value, RpcError>;

fn task_not_found(id: &str) -> (i32, String, Option<serde_json::Value>) {
    (
        -32001,
        format!("task not found: {id}"),
        Some(serde_json::json!([{
            "@type": "type.googleapis.com/google.rpc.ErrorInfo",
            "reason": "TASK_NOT_FOUND",
            "domain": "a2a-protocol.org"
        }])),
    )
}

fn rpc_get_task(shared: &Shared, params: serde_json::Value) -> RpcResult {
    let params: GetTaskParams = serde_json::from_value(params)
        .map_err(|err| (-32602, format!("invalid params: {err}"), None))?;
    let store = shared
        .store
        .lock()
        .map_err(|_| (-32603, "store lock poisoned".to_string(), None))?;
    match store.get(&params.id, params.history_length) {
        Some(task) => {
            Ok(serde_json::to_value(task).map_err(|err| (-32603, err.to_string(), None))?)
        }
        None => Err(task_not_found(&params.id)),
    }
}

fn rpc_list_tasks(shared: &Shared, params: serde_json::Value) -> RpcResult {
    let params: ListTasksParams = serde_json::from_value(params)
        .map_err(|err| (-32602, format!("invalid params: {err}"), None))?;
    let store = shared
        .store
        .lock()
        .map_err(|_| (-32603, "store lock poisoned".to_string(), None))?;
    serde_json::to_value(store.list(&params)).map_err(|err| (-32603, err.to_string(), None))
}

fn rpc_cancel_task(shared: &Shared, params: serde_json::Value) -> RpcResult {
    let params: TaskIdParams = serde_json::from_value(params)
        .map_err(|err| (-32602, format!("invalid params: {err}"), None))?;
    let mut store = shared
        .store
        .lock()
        .map_err(|_| (-32603, "store lock poisoned".to_string(), None))?;
    match store.cancel(&params.id, &rfc3339_now()) {
        Ok(task) => Ok(serde_json::to_value(task).map_err(|err| (-32603, err.to_string(), None))?),
        Err(StoreError::NotFound(id)) => Err(task_not_found(&id)),
        Err(StoreError::InvalidTransition { .. }) => Err((
            -32002,
            "task cannot be canceled: it is in a terminal state".to_string(),
            None,
        )),
        Err(err) => Err((-32603, err.to_string(), None)),
    }
}

// ---------------------------------------------------------------------
// SendMessage: task creation, classification, execution.

/// Classify an inbound caller: (name for the log, join spec for the
/// task trajectory). A policy source declared under the caller's
/// name (or `agent:<name>`) wins; otherwise the unvetted default —
/// audience `self`, lowest trust — applies.
fn classify(shared: &Shared, caller: Option<&str>) -> (String, JoinSpec) {
    let name = caller.unwrap_or(UNVETTED_CALLER).to_string();
    if let Some(policy) = &shared.policy {
        for candidate in [name.clone(), format!("agent:{name}")] {
            if let Some(source) = policy.source(&candidate) {
                return (name, JoinSpec::Delta(source.delta.clone()));
            }
        }
        let lowest = policy
            .trust_chain()
            .first()
            .cloned()
            .unwrap_or_else(|| "suspicious".to_string());
        return (
            name,
            JoinSpec::Delta(LabelDelta {
                audience: vec!["self".to_string()],
                trust: Some(lowest),
            }),
        );
    }
    (
        name,
        JoinSpec::Delta(LabelDelta {
            audience: vec!["self".to_string()],
            trust: Some("suspicious".to_string()),
        }),
    )
}

/// Create the task + trajectory for a new message, and (when
/// capacity allows) spawn its worker. Returns the task id and a
/// completion receiver (None if the task was capacity-rejected and
/// will never run).
fn begin_task(
    shared: &Arc<Shared>,
    message: Message,
    caller: Option<&str>,
) -> Result<(String, Option<mpsc::Receiver<Task>>), RpcError> {
    let now = rfc3339_now();
    let task = {
        let mut store = shared
            .store
            .lock()
            .map_err(|_| (-32603, "store lock poisoned".to_string(), None))?;
        store
            .create(message.clone(), &now)
            .map_err(|err| (-32603, err.to_string(), None))?
    };
    if let Some(policy) = &shared.policy {
        let mut trajectory = Trajectory::new(policy.clone());
        let (from, join) = classify(shared, caller);
        let _ = trajectory.record_message_received(&from, join);
        if let Ok(mut trajectories) = shared.trajectories.lock() {
            trajectories.insert(task.id.clone(), trajectory);
        }
    }
    if shared.workers_active.load(Ordering::SeqCst) >= WORKERS_MAX {
        let mut store = shared
            .store
            .lock()
            .map_err(|_| (-32603, "store lock poisoned".to_string(), None))?;
        let _ = store.transition(
            &task.id,
            TaskState::Rejected,
            Some(Message::agent_text(
                &shared.mint_id("msg"),
                "server at executor capacity; task rejected rather than queued without bound",
            )),
            &rfc3339_now(),
        );
        return Ok((task.id, None));
    }
    let (completion_tx, completion_rx) = mpsc::channel();
    spawn_worker(
        shared.clone(),
        task.id.clone(),
        message,
        Some(completion_tx),
    );
    Ok((task.id, Some(completion_rx)))
}

fn spawn_worker(
    shared: Arc<Shared>,
    task_id: String,
    message: Message,
    completion: Option<mpsc::Sender<Task>>,
) {
    shared.workers_active.fetch_add(1, Ordering::SeqCst);
    std::thread::spawn(move || {
        run_task(&shared, &task_id, &message);
        if let Some(completion) = completion
            && let Ok(store) = shared.store.lock()
            && let Some(task) = store.get(&task_id, None)
        {
            let _ = completion.send(task);
        }
        shared.workers_active.fetch_sub(1, Ordering::SeqCst);
    });
}

fn run_task(shared: &Arc<Shared>, task_id: &str, message: &Message) {
    let now = rfc3339_now();
    let cancel_flag = {
        let Ok(mut store) = shared.store.lock() else {
            return;
        };
        let Some(flag) = store.cancel_flag(task_id) else {
            return;
        };
        let _ = store.transition(task_id, TaskState::Working, None, &now);
        flag
    };
    broadcast_status(shared, task_id);
    let context_id = {
        let Ok(store) = shared.store.lock() else {
            return;
        };
        let Some(task) = store.get(task_id, None) else {
            return;
        };
        task.context_id
    };
    let mut cx = ExecCx {
        shared: shared.clone(),
        task_id: task_id.to_string(),
        context_id,
        cancel_flag,
        input_required: None,
    };
    let executor = shared.executor.clone();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        executor.execute(&mut cx, message)
    }));
    let now = rfc3339_now();
    let cancelled = cx.is_cancelled();
    if let Ok(mut store) = shared.store.lock() {
        if cancelled {
            let _ = store.transition(task_id, TaskState::Canceled, None, &now);
        } else if let Some(question) = cx.input_required.take() {
            let _ = store.transition(task_id, TaskState::InputRequired, Some(question), &now);
        } else {
            match outcome {
                Ok(Ok(artifacts)) => {
                    for (index, mut artifact) in artifacts.into_iter().enumerate() {
                        if artifact.artifact_id.is_empty() {
                            artifact.artifact_id = format!("{task_id}-art-{index}");
                        }
                        let _ = store.add_artifact(task_id, artifact);
                    }
                    let _ = store.transition(task_id, TaskState::Completed, None, &now);
                }
                Ok(Err(reason)) => {
                    let note = Message::agent_text(&shared.mint_id("msg"), &reason);
                    let _ = store.transition(task_id, TaskState::Failed, Some(note), &now);
                }
                Err(_) => {
                    let note = Message::agent_text(
                        &shared.mint_id("msg"),
                        "executor panicked; the task was failed rather than left working",
                    );
                    let _ = store.transition(task_id, TaskState::Failed, Some(note), &now);
                }
            }
        }
    }
    broadcast_status(shared, task_id);
    // Terminal or paused: drop subscriptions so SSE streams close.
    if let Ok(store) = shared.store.lock()
        && let Some(task) = store.get(task_id, None)
        && (task.status.state.is_terminal() || task.status.state == TaskState::InputRequired)
    {
        drop_subscribers(shared, task_id);
    }
}

fn subscribe(shared: &Arc<Shared>, task_id: &str) -> mpsc::Receiver<SendStreamingResponse> {
    let (tx, rx) = mpsc::channel();
    if let Ok(mut subscribers) = shared.subscribers.lock() {
        subscribers.entry(task_id.to_string()).or_default().push(tx);
    }
    rx
}

fn drop_subscribers(shared: &Arc<Shared>, task_id: &str) {
    if let Ok(mut subscribers) = shared.subscribers.lock() {
        subscribers.remove(task_id);
    }
}

fn broadcast(shared: &Arc<Shared>, task_id: &str, event: SendStreamingResponse) {
    if let Ok(mut subscribers) = shared.subscribers.lock()
        && let Some(senders) = subscribers.get_mut(task_id)
    {
        senders.retain(|sender| sender.send(event.clone()).is_ok());
    }
}

fn broadcast_status(shared: &Arc<Shared>, task_id: &str) {
    let event = {
        let Ok(store) = shared.store.lock() else {
            return;
        };
        let Some(task) = store.get(task_id, None) else {
            return;
        };
        SendStreamingResponse::StatusUpdate(TaskStatusUpdateEvent {
            task_id: task.id.clone(),
            context_id: task.context_id.clone(),
            status: task.status.clone(),
            metadata: None,
        })
    };
    broadcast(shared, task_id, event);
}

fn handle_send(
    shared: Arc<Shared>,
    stream: &mut TcpStream,
    id: &serde_json::Value,
    params: serde_json::Value,
    caller: Option<&str>,
    streaming: bool,
) -> std::io::Result<()> {
    let params: SendMessageParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => {
            return write_rpc_error(
                stream,
                id.clone(),
                -32602,
                &format!("invalid params: {err}"),
                None,
            );
        }
    };
    if let Err(problem) = params.message.validate() {
        return write_rpc_error(stream, id.clone(), -32602, &problem, None);
    }
    // Continuation message for an existing task.
    if let Some(existing_id) = params.message.task_id.clone() {
        return handle_continuation(shared, stream, id, params, caller, streaming, &existing_id);
    }
    // The subscription is attached right after task creation; a
    // terminal that lands in between is caught by the pump's
    // store-side terminal check, so no final state can be missed.
    match begin_task(&shared, params.message.clone(), caller) {
        Ok((task_id, completion)) => {
            if streaming {
                let receiver = subscribe(&shared, &task_id);
                write_sse_head(stream)?;
                pump_sse(&shared, stream, id, &task_id, receiver, true)
            } else {
                let task = await_task(&shared, &task_id, completion);
                match task {
                    Some(task) => {
                        let value = serde_json::to_value(task).unwrap_or(serde_json::Value::Null);
                        write_rpc_value(stream, id, Ok(value))
                    }
                    None => write_rpc_error(stream, id.clone(), -32001, "task vanished", None),
                }
            }
        }
        Err((code, message, data)) => write_rpc_error(stream, id.clone(), code, &message, data),
    }
}

fn handle_continuation(
    shared: Arc<Shared>,
    stream: &mut TcpStream,
    id: &serde_json::Value,
    params: SendMessageParams,
    caller: Option<&str>,
    streaming: bool,
    existing_id: &str,
) -> std::io::Result<()> {
    let existing = {
        let Ok(store) = shared.store.lock() else {
            return write_rpc_error(stream, id.clone(), -32603, "store lock poisoned", None);
        };
        store.get(existing_id, None)
    };
    let Some(existing) = existing else {
        let (code, message, data) = task_not_found(existing_id);
        return write_rpc_error(stream, id.clone(), code, &message, data);
    };
    if existing.status.state.is_terminal() {
        return write_rpc_error(
            stream,
            id.clone(),
            -32602,
            "messages to a terminal task are rejected",
            None,
        );
    }
    // Join the follow-up's classification and append it to history.
    if shared.policy.is_some() {
        let (from, join) = classify(&shared, caller);
        if let Ok(mut trajectories) = shared.trajectories.lock()
            && let Some(trajectory) = trajectories.get_mut(existing_id)
        {
            let _ = trajectory.record_message_received(&from, join);
        }
    }
    {
        let Ok(mut store) = shared.store.lock() else {
            return write_rpc_error(stream, id.clone(), -32603, "store lock poisoned", None);
        };
        let _ = store.append_message(existing_id, params.message.clone());
    }
    let resumable = matches!(
        existing.status.state,
        TaskState::InputRequired | TaskState::AuthRequired
    );
    if !resumable {
        // Work is already in flight; the message is recorded and
        // the current task is the answer.
        let value = serde_json::to_value(existing).unwrap_or(serde_json::Value::Null);
        return write_rpc_value(stream, id, Ok(value));
    }
    if shared.workers_active.load(Ordering::SeqCst) >= WORKERS_MAX {
        let value = serde_json::to_value(existing).unwrap_or(serde_json::Value::Null);
        return write_rpc_value(stream, id, Ok(value));
    }
    let (completion_tx, completion_rx) = mpsc::channel();
    if streaming {
        let receiver = subscribe(&shared, existing_id);
        spawn_worker(
            shared.clone(),
            existing_id.to_string(),
            params.message.clone(),
            Some(completion_tx),
        );
        write_sse_head(stream)?;
        pump_sse(&shared, stream, id, existing_id, receiver, true)
    } else {
        spawn_worker(
            shared.clone(),
            existing_id.to_string(),
            params.message.clone(),
            Some(completion_tx),
        );
        let task = await_task(&shared, existing_id, Some(completion_rx));
        match task {
            Some(task) => {
                let value = serde_json::to_value(task).unwrap_or(serde_json::Value::Null);
                write_rpc_value(stream, id, Ok(value))
            }
            None => write_rpc_error(stream, id.clone(), -32001, "task vanished", None),
        }
    }
}

/// Wait for a worker's completion up to the configured request
/// timeout; on timeout, return the task's current snapshot.
fn await_task(
    shared: &Arc<Shared>,
    task_id: &str,
    completion: Option<mpsc::Receiver<Task>>,
) -> Option<Task> {
    if let Some(completion) = completion {
        match completion.recv_timeout(shared.request_timeout) {
            Ok(task) => return Some(task),
            Err(_) => {
                let store = shared.store.lock().ok()?;
                return store.get(task_id, None);
            }
        }
    }
    let store = shared.store.lock().ok()?;
    store.get(task_id, None)
}

fn handle_subscribe(
    shared: Arc<Shared>,
    stream: &mut TcpStream,
    id: &serde_json::Value,
    params: serde_json::Value,
) -> std::io::Result<()> {
    let params: TaskIdParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => {
            return write_rpc_error(
                stream,
                id.clone(),
                -32602,
                &format!("invalid params: {err}"),
                None,
            );
        }
    };
    let exists = {
        let Ok(store) = shared.store.lock() else {
            return write_rpc_error(stream, id.clone(), -32603, "store lock poisoned", None);
        };
        store.get(&params.id, None).is_some()
    };
    if !exists {
        let (code, message, data) = task_not_found(&params.id);
        return write_rpc_error(stream, id.clone(), code, &message, data);
    }
    let receiver = subscribe(&shared, &params.id);
    write_sse_head(stream)?;
    pump_sse(&shared, stream, id, &params.id, receiver, true)
}

// ---------------------------------------------------------------------
// SSE.

fn write_sse_head(stream: &mut TcpStream) -> std::io::Result<()> {
    stream.write_all(
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n",
    )?;
    stream.flush()
}

/// Stream events for one task: first the current snapshot (when
/// `snapshot_first`), then live updates, until the task's terminal
/// (or input-required) event has been delivered, the stream's
/// lifetime bound is reached, or the client goes away.
fn pump_sse(
    shared: &Arc<Shared>,
    stream: &mut TcpStream,
    id: &serde_json::Value,
    task_id: &str,
    receiver: mpsc::Receiver<SendStreamingResponse>,
    snapshot_first: bool,
) -> std::io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(SSE_LIFETIME_SECS);
    if snapshot_first
        && let Ok(store) = shared.store.lock()
        && let Some(task) = store.get(task_id, None)
    {
        let done = task.status.state.is_terminal() || task.status.state == TaskState::InputRequired;
        if write_sse_event(stream, id, &SendStreamingResponse::Task(task)).is_err() {
            return Ok(());
        }
        if done {
            return Ok(());
        }
    }
    loop {
        match receiver.recv_timeout(Duration::from_millis(500)) {
            Ok(event) => {
                let terminal = match &event {
                    SendStreamingResponse::Task(task) => {
                        task.status.state.is_terminal()
                            || task.status.state == TaskState::InputRequired
                    }
                    SendStreamingResponse::StatusUpdate(update) => {
                        update.status.state.is_terminal()
                            || update.status.state == TaskState::InputRequired
                    }
                    SendStreamingResponse::ArtifactUpdate(_) => false,
                };
                if write_sse_event(stream, id, &event).is_err() {
                    return Ok(());
                }
                if terminal {
                    return Ok(());
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if Instant::now() >= deadline {
                    return Ok(());
                }
                // A terminal state reached without a delivered
                // event (e.g. the worker finished before this
                // subscription attached) still ends the stream:
                // check the store and deliver the final snapshot.
                if let Ok(store) = shared.store.lock()
                    && let Some(task) = store.get(task_id, None)
                    && (task.status.state.is_terminal()
                        || task.status.state == TaskState::InputRequired)
                {
                    let _ = write_sse_event(stream, id, &SendStreamingResponse::Task(task));
                    return Ok(());
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

fn write_sse_event(
    stream: &mut TcpStream,
    id: &serde_json::Value,
    event: &SendStreamingResponse,
) -> std::io::Result<()> {
    let body = serde_json::json!({"jsonrpc": "2.0", "id": id, "result": event});
    stream.write_all(format!("data: {body}\n\n").as_bytes())?;
    stream.flush()
}
