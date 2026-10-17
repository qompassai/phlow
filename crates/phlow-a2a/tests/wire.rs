//! End-to-end wire tests: real servers on loopback, real clients,
//! the full stack of card discovery, lifecycle, streaming, cancel,
//! error shapes, auth, and the flow gate inside the server.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use phlow_a2a::client::{A2aClient, A2aError, ClientConfig};
use phlow_a2a::registry::{ExternalAgent, ExternalRegistry};
use phlow_a2a::server::{
    A2aServer, AgentExecutor, ExecCx, OutboundSink, ServerConfig, ServerHandle,
};
use phlow_a2a::types::{
    AgentCapabilities, AgentCard, AgentInterface, AgentSkill, Artifact, ListTasksParams, Message,
    Part, Role, SendStreamingResponse, TaskState,
};
use phlow_flowguard::Policy;

// ---------------------------------------------------------------------
// Test doubles.

struct EchoExecutor;

impl AgentExecutor for EchoExecutor {
    fn execute(&self, _cx: &mut ExecCx, message: &Message) -> Result<Vec<Artifact>, String> {
        let text = message.text_contents().join(" ");
        Ok(vec![Artifact {
            artifact_id: "echo".to_string(),
            name: Some("echo".to_string()),
            description: None,
            parts: vec![Part::text(&format!("echo: {text}"))],
            extensions: Vec::new(),
            metadata: None,
        }])
    }
}

struct StreamingExecutor;

impl AgentExecutor for StreamingExecutor {
    fn execute(&self, cx: &mut ExecCx, message: &Message) -> Result<Vec<Artifact>, String> {
        // Give the request thread a beat to attach the stream
        // subscription before the artifact update is broadcast
        // (test-double pacing only; the server's snapshot + store
        // terminal check cover the race in real use).
        std::thread::sleep(Duration::from_millis(150));
        let text = message.text_contents().join(" ");
        cx.add_artifact(Artifact {
            artifact_id: "streamed".to_string(),
            name: None,
            description: None,
            parts: vec![Part::text(&format!("streamed: {text}"))],
            extensions: Vec::new(),
            metadata: None,
        })?;
        Ok(Vec::new())
    }
}

struct GatekeeperExecutor;

impl AgentExecutor for GatekeeperExecutor {
    fn execute(&self, cx: &mut ExecCx, message: &Message) -> Result<Vec<Artifact>, String> {
        let text = message.text_contents().join(" ");
        if text.contains("need:") {
            cx.require_input(Message::agent_text(
                "question-1",
                "which city should the report cover?",
            ));
            return Ok(Vec::new());
        }
        if text.contains("city:") {
            return Ok(vec![Artifact {
                artifact_id: "report".to_string(),
                name: None,
                description: None,
                parts: vec![Part::text(&format!("report for {text}"))],
                extensions: Vec::new(),
                metadata: None,
            }]);
        }
        if text.contains("send-please") {
            return match cx.send_message_gated("external:world", vec![Part::text("the report")]) {
                Ok(_) => Ok(vec![Artifact {
                    artifact_id: "sent".to_string(),
                    name: None,
                    description: None,
                    parts: vec![Part::text("sent")],
                    extensions: Vec::new(),
                    metadata: None,
                }]),
                Err(rejection) => {
                    let plan = serde_json::json!({
                        "reasons": format!("{:?}", rejection.0.reasons),
                        "remedies": format!("{:?}", rejection.0.remedies),
                    });
                    let mut note =
                        Message::agent_text("blocked-1", "the flow gate blocked the send");
                    note.parts.push(Part::data(plan));
                    cx.require_input(note);
                    Ok(Vec::new())
                }
            };
        }
        if text.contains("slow") {
            for _ in 0..300 {
                if cx.is_cancelled() {
                    return Err("cancelled".to_string());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            return Ok(Vec::new());
        }
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct RecordingSink {
    delivered: Mutex<Vec<String>>,
}

impl OutboundSink for RecordingSink {
    fn deliver(&self, destination: &str, _message: &Message) -> Result<(), String> {
        self.delivered
            .lock()
            .map_err(|_| "sink lock poisoned")?
            .push(destination.to_string());
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Helpers.

fn test_card(url: &str) -> AgentCard {
    AgentCard {
        name: "phlow-test-agent".to_string(),
        description: "test double".to_string(),
        supported_interfaces: vec![AgentInterface {
            url: url.to_string(),
            protocol_binding: "JSONRPC".to_string(),
            protocol_version: "1.0".to_string(),
            tenant: None,
        }],
        provider: None,
        version: "0.1.0".to_string(),
        documentation_url: None,
        capabilities: AgentCapabilities {
            streaming: true,
            push_notifications: false,
            extensions: Vec::new(),
        },
        security_schemes: Default::default(),
        security: Vec::new(),
        default_input_modes: vec!["text/plain".to_string()],
        default_output_modes: vec!["text/plain".to_string()],
        skills: vec![AgentSkill {
            id: "echo".to_string(),
            name: "Echo".to_string(),
            description: "echoes".to_string(),
            tags: Vec::new(),
            examples: Vec::new(),
            input_modes: Vec::new(),
            output_modes: Vec::new(),
        }],
        icon_url: None,
        signatures: Vec::new(),
    }
}

fn start(config: ServerConfig, executor: Arc<dyn AgentExecutor>) -> (ServerHandle, A2aClient) {
    let handle = A2aServer::start(config, test_card("http://127.0.0.1:0/"), executor)
        .expect("server starts");
    let client = A2aClient::new(&handle.url(), ClientConfig::default()).expect("client");
    (handle, client)
}

fn raw_http(addr: std::net::SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream.write_all(request.as_bytes()).expect("write");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read");
    response
}

fn rpc_body(method: &str, params: serde_json::Value) -> String {
    serde_json::json!({"jsonrpc": "2.0", "id": 9, "method": method, "params": params}).to_string()
}

fn post(addr: std::net::SocketAddr, body: &str) -> serde_json::Value {
    let request = format!(
        "POST / HTTP/1.1\r\nhost: x\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    let response = raw_http(addr, &request);
    let json_start = response.find("\r\n\r\n").expect("header end") + 4;
    serde_json::from_str(&response[json_start..]).expect("json response")
}

// ---------------------------------------------------------------------
// Validation: the happy paths are real round trips.

#[test]
fn card_discovery_and_echo_round_trip() {
    let (handle, client) = start(ServerConfig::default(), Arc::new(EchoExecutor));
    let card = client.fetch_card().expect("card");
    assert_eq!(card.name, "phlow-test-agent");
    assert!(
        !card.capabilities.push_notifications,
        "card must not claim push support"
    );
    assert_eq!(card.primary_url(), Some(handle.url().as_str()));

    let task = client
        .send_message(&Message::text("m1", "hello a2a"))
        .expect("send");
    assert_eq!(task.status.state, TaskState::Completed);
    assert_eq!(task.artifacts.len(), 1);
    assert_eq!(
        task.artifacts[0].parts[0].text.as_deref(),
        Some("echo: hello a2a")
    );

    let fetched = client.get_task(&task.id, None).expect("get");
    assert_eq!(fetched.id, task.id);
    let listed = client
        .list_tasks(&ListTasksParams::default())
        .expect("list");
    assert_eq!(listed.total_size, 1);
    handle.stop();
}

#[test]
fn streaming_delivers_status_and_artifact_events() {
    let (handle, client) = start(ServerConfig::default(), Arc::new(StreamingExecutor));
    let mut events: Vec<SendStreamingResponse> = Vec::new();
    let count = client
        .send_streaming(&Message::text("m1", "stream me"), &mut |event| {
            events.push(event);
        })
        .expect("stream");
    assert_eq!(count, events.len());
    assert!(
        events.iter().any(|event| matches!(event, SendStreamingResponse::ArtifactUpdate(update) if update.artifact.artifact_id == "streamed")),
        "expected a streamed artifact update: {events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(event, SendStreamingResponse::StatusUpdate(update) if update.status.state == TaskState::Completed)),
        "expected a completed status update: {events:?}"
    );
    handle.stop();
}

#[test]
fn input_required_pauses_and_a_follow_up_resumes() {
    let (handle, client) = start(ServerConfig::default(), Arc::new(GatekeeperExecutor));
    let task = client
        .send_message(&Message::text("m1", "write the report need: details"))
        .expect("send");
    assert_eq!(task.status.state, TaskState::InputRequired);
    let mut follow_up = Message::text("m2", "city: Spokane");
    follow_up.task_id = Some(task.id.clone());
    follow_up.context_id = Some(task.context_id.clone());
    let resumed = client.send_message(&follow_up).expect("resume");
    assert_eq!(resumed.status.state, TaskState::Completed);
    assert_eq!(resumed.artifacts.len(), 1);
    handle.stop();
}

#[test]
fn cancel_a_running_task_lands_in_canceled() {
    let (handle, client) = start(ServerConfig::default(), Arc::new(GatekeeperExecutor));
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let url = handle.url();
    let streamer = std::thread::spawn(move || {
        let thread_client = A2aClient::new(&url, ClientConfig::default()).expect("client");
        let mut sent = false;
        let _ = thread_client.send_streaming(&Message::text("m1", "slow job"), &mut |event| {
            if !sent && let SendStreamingResponse::Task(task) = &event {
                let _ = tx.send(task.id.clone());
                sent = true;
            }
        });
    });
    let task_id = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("task id from stream");
    let cancelled = client.cancel_task(&task_id).expect("cancel");
    assert_eq!(cancelled.status.state, TaskState::Canceled);
    let fetched = client.get_task(&task_id, None).expect("get");
    assert_eq!(fetched.status.state, TaskState::Canceled);
    streamer.join().expect("stream thread ends");
    handle.stop();
}

// ---------------------------------------------------------------------
// Adversarial / protocol-shape: errors are the spec's errors.

#[test]
fn unknown_task_is_the_spec_error_with_error_info() {
    let (handle, client) = start(ServerConfig::default(), Arc::new(EchoExecutor));
    match client.get_task("task-does-not-exist", None) {
        Err(A2aError::Rpc { code, data, .. }) => {
            assert_eq!(code, -32001);
            let data = data.expect("error info data");
            assert!(data.to_string().contains("TASK_NOT_FOUND"), "data: {data}");
        }
        other => panic!("expected Rpc -32001, got {other:?}"),
    }
    handle.stop();
}

#[test]
fn malformed_json_and_unknown_method_are_json_rpc_errors() {
    let (handle, _client) = start(ServerConfig::default(), Arc::new(EchoExecutor));
    let bad = post(handle.addr(), "{\"jsonrpc\": ");
    assert_eq!(bad["error"]["code"], -32700);
    let unknown = post(
        handle.addr(),
        &rpc_body("ExplodeTask", serde_json::json!({})),
    );
    assert_eq!(unknown["error"]["code"], -32601);
    // The 0.3-era aliases are accepted as an interop courtesy.
    let created = post(
        handle.addr(),
        &rpc_body(
            "message/send",
            serde_json::json!({"message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}}),
        ),
    );
    let task_id = created["result"]["id"]
        .as_str()
        .expect("task id")
        .to_string();
    let fetched = post(
        handle.addr(),
        &rpc_body("tasks/get", serde_json::json!({"id": task_id})),
    );
    assert_eq!(fetched["result"]["status"]["state"], "TASK_STATE_COMPLETED");
    handle.stop();
}

#[test]
fn message_to_terminal_task_is_rejected() {
    let (handle, client) = start(ServerConfig::default(), Arc::new(EchoExecutor));
    let task = client
        .send_message(&Message::text("m1", "one shot"))
        .expect("send");
    assert_eq!(task.status.state, TaskState::Completed);
    let mut follow_up = Message::text("m2", "are you still there");
    follow_up.task_id = Some(task.id.clone());
    match client.send_message(&follow_up) {
        Err(A2aError::Rpc { code, .. }) => assert_eq!(code, -32602),
        other => panic!("expected -32602, got {other:?}"),
    }
    handle.stop();
}

#[test]
fn bearer_tokens_gate_the_surface_and_identity_comes_from_the_token() {
    let mut config = ServerConfig::default();
    config
        .tokens
        .insert("tok-alice".to_string(), "alice".to_string());
    let (handle, _client) = start(config, Arc::new(EchoExecutor));
    // No token: refused at the HTTP layer.
    let anonymous = A2aClient::new(&handle.url(), ClientConfig::default()).expect("client");
    assert!(anonymous.send_message(&Message::text("m1", "hi")).is_err());
    // With the token: served.
    let authed = A2aClient::new(
        &handle.url(),
        ClientConfig {
            token: Some("tok-alice".to_string()),
            ..ClientConfig::default()
        },
    )
    .expect("client");
    let task = authed
        .send_message(&Message::text("m1", "hi"))
        .expect("authed send");
    assert_eq!(task.status.state, TaskState::Completed);
    handle.stop();
}

#[test]
fn non_loopback_bind_without_tokens_is_refused_at_startup() {
    let config = ServerConfig {
        bind: "0.0.0.0:0".parse().expect("addr"),
        ..ServerConfig::default()
    };
    let result = A2aServer::start(
        config,
        test_card("http://0.0.0.0:0/"),
        Arc::new(EchoExecutor),
    );
    assert!(result.is_err(), "the phlow override must refuse this bind");
}

// ---------------------------------------------------------------------
// The gate inside the server.

const GATE_POLICY: &str = r#"
version = 1

[audiences]
self = ["op@example.com"]
internal = ["op@example.com"]

[[destinations]]
name = "external:world"
readers = ["public"]

[[sources]]
name = "agent:bob"
delta = {}
"#;

#[test]
fn unvetted_caller_cannot_send_onward_but_a_declared_peer_can() {
    let policy = Arc::new(Policy::from_toml(GATE_POLICY).expect("policy"));
    let sink = Arc::new(RecordingSink::default());
    let mut config = ServerConfig {
        policy: Some(policy),
        outbound: Some(sink.clone()),
        ..ServerConfig::default()
    };
    config
        .tokens
        .insert("tok-alice".to_string(), "alice".to_string());
    config
        .tokens
        .insert("tok-bob".to_string(), "bob".to_string());
    let (handle, _client) = start(config, Arc::new(GatekeeperExecutor));

    let alice = A2aClient::new(
        &handle.url(),
        ClientConfig {
            token: Some("tok-alice".to_string()),
            ..ClientConfig::default()
        },
    )
    .expect("alice client");
    let bob = A2aClient::new(
        &handle.url(),
        ClientConfig {
            token: Some("tok-bob".to_string()),
            ..ClientConfig::default()
        },
    )
    .expect("bob client");

    // Alice has no policy source declaration: her message joins the
    // unvetted delta (audience self, lowest trust) into her task's
    // trajectory, so the executor's onward send to a public
    // destination is blocked and surfaces as input-required with
    // the remedy plan attached.
    let alices = alice
        .send_message(&Message::text("m1", "send-please"))
        .expect("alice send");
    assert_eq!(alices.status.state, TaskState::InputRequired);
    let note = alices.status.message.expect("status note");
    assert!(
        note.parts.iter().any(|part| part
            .data
            .as_ref()
            .map(|data| data.to_string().contains("AudienceMissing"))
            .unwrap_or(false)),
        "the block reason rides the status message: {note:?}"
    );
    assert!(
        sink.delivered.lock().expect("sink").is_empty(),
        "nothing may be delivered for a blocked send"
    );

    // Bob's source is declared with an empty delta (no restriction):
    // the same send is allowed and delivered.
    let bobs = bob
        .send_message(&Message::text("m2", "send-please"))
        .expect("bob send");
    assert_eq!(bobs.status.state, TaskState::Completed);
    assert_eq!(
        sink.delivered.lock().expect("sink").as_slice(),
        ["external:world"]
    );
    handle.stop();
}

#[test]
fn registry_send_is_gated_by_the_callers_trajectory() {
    // A local trajectory that read private data cannot task an
    // external agent at a public destination with it; a clean one
    // (against a live echo server) can.
    let (handle, _client) = start(ServerConfig::default(), Arc::new(EchoExecutor));
    let policy = Arc::new(
        Policy::from_toml(
            r#"
version = 1
[audiences]
self = ["op@example.com"]
internal = ["op@example.com", "dev@example.com"]
[[destinations]]
name = "external:echo"
readers = ["internal"]
[[sources]]
name = "file:secret"
delta = { audience = ["self"] }
"#,
        )
        .expect("policy"),
    );
    let mut registry = ExternalRegistry::new();
    registry
        .register(ExternalAgent {
            name: "echo".to_string(),
            interface_url: handle.url(),
            destination: "external:echo".to_string(),
            token: None,
        })
        .expect("register");

    let mut clean = phlow_flowguard::Trajectory::new(policy.clone());
    let tasked = registry
        .send_task(&mut clean, "echo", &Message::text("m1", "hello out there"))
        .expect("clean send is allowed");
    assert_eq!(tasked.status.state, TaskState::Completed);

    let mut tainted = phlow_flowguard::Trajectory::new(policy);
    tainted.record_read("file:secret").expect("read");
    let blocked = registry.send_task(
        &mut tainted,
        "echo",
        &Message::text("m2", "the secret is..."),
    );
    assert!(
        matches!(blocked, Err(phlow_a2a::registry::RegistryError::Blocked(_))),
        "tainted trajectory must be blocked at the gate: {blocked:?}"
    );
    handle.stop();
}

#[test]
fn agent_role_messages_round_trip_their_role() {
    let (handle, client) = start(ServerConfig::default(), Arc::new(EchoExecutor));
    let task = client
        .send_message(&Message::text("m1", "role check"))
        .expect("send");
    let history = &task.history;
    assert_eq!(history[0].role, Role::User);
    handle.stop();
}
