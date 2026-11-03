//! Utility pre-gate integration tests (D-717).
//!
//! Covers, against scripted fakes (no network, no Neovim):
//!
//! - routing end-to-end through [`Runtime::run`]: high scores dispatch
//!   as planned, mid scores downgrade (cheap model, or narrowed cycles
//!   without one), low scores queue without any dispatch;
//! - fail-open behaviour: scorer transport errors and malformed scorer
//!   replies dispatch as planned with the failure recorded;
//! - the hard boundary: the scorer cannot affect a permission
//!   decision — a maximum score does not grant an untrusted coder its
//!   `file_write`, and a minimum score does not block a trusted one.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use phlow_config::{FlowConfig, LoadOptions, load_config};
use phlow_editor::{CALL_LUA, EditorTransport, SCHEMAS_LUA, TransportError};
use phlow_llm::{LlmError, LlmTransport};
use phlow_runtime::Runtime;
use serde_json::{Map, Value, json};

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

/// Unique scratch directory (canonicalized: `/tmp` may be a symlink).
fn testdir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "phlow-pregate-test-{}-{}-{}",
        std::process::id(),
        name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("testdir");
    std::fs::canonicalize(&dir).expect("canonical testdir")
}

fn load_test_config(dir: &std::path::Path, toml: &str, trusted: bool) -> FlowConfig {
    std::fs::write(dir.join("config.toml"), toml).expect("config.toml");
    load_config(&LoadOptions {
        config_path: Some(dir.join("config.toml")),
        workspace: Some(dir.to_path_buf()),
        trusted,
        model: None,
    })
    .expect("test config loads")
}

/// Scripted model transport: pops replies (or failures) in order and
/// records every payload and deadline, so tests can see exactly which
/// model each call named and how long it was allowed to run.
#[derive(Clone)]
struct PregLlm {
    shared: Arc<Mutex<PregState>>,
}

struct PregState {
    script: VecDeque<Result<Value, LlmError>>,
    payloads: Vec<Map<String, Value>>,
    timeouts: Vec<Duration>,
}

impl PregLlm {
    fn new(script: Vec<Result<Value, LlmError>>) -> Self {
        PregLlm {
            shared: Arc::new(Mutex::new(PregState {
                script: script.into(),
                payloads: Vec::new(),
                timeouts: Vec::new(),
            })),
        }
    }

    fn post_count(&self) -> usize {
        self.shared.lock().unwrap().payloads.len()
    }

    /// The `model` field of the `index`-th recorded payload.
    fn payload_model(&self, index: usize) -> String {
        let state = self.shared.lock().unwrap();
        state.payloads[index]["model"]
            .as_str()
            .expect("payload names a model")
            .to_owned()
    }

    /// The deadline handed to the `index`-th call.
    fn payload_timeout(&self, index: usize) -> Duration {
        self.shared.lock().unwrap().timeouts[index]
    }
}

impl LlmTransport for PregLlm {
    fn post_chat(
        &mut self,
        _base_url: &str,
        payload: &Map<String, Value>,
        timeout: Duration,
    ) -> Result<Value, LlmError> {
        let mut state = self.shared.lock().unwrap();
        state.payloads.push(payload.clone());
        state.timeouts.push(timeout);
        state
            .script
            .pop_front()
            .unwrap_or_else(|| Ok(chat_text("Fallback. Done.")))
    }

    fn get_tags(&mut self, _base_url: &str, _timeout: Duration) -> Result<Value, LlmError> {
        Ok(json!({"models": []}))
    }

    fn close(&mut self) {}
}

/// Scripted editor transport: no editor tools, every call ok.
#[derive(Clone)]
struct FakeEditor {
    shared: Arc<Mutex<HashMap<String, Value>>>,
}

impl FakeEditor {
    fn new() -> Self {
        FakeEditor {
            shared: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl EditorTransport for FakeEditor {
    fn exec(
        &mut self,
        expression: &str,
        args: &[Value],
        _timeout: Duration,
    ) -> Result<Value, TransportError> {
        if expression == SCHEMAS_LUA {
            Ok(json!([]))
        } else if expression == CALL_LUA {
            let name = args.first().and_then(|arg| arg.as_str()).unwrap_or("");
            Ok(self
                .shared
                .lock()
                .unwrap()
                .get(name)
                .cloned()
                .unwrap_or(json!({"status": "ok"})))
        } else {
            Err(TransportError::Failed("unexpected expression".to_owned()))
        }
    }

    fn close(&mut self) {}
}

/// A chat completion with plain text content.
fn chat_text(content: &str) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": content}}]})
}

/// A chat completion whose message carries tool calls.
fn chat_tools(calls: Value) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": "", "tool_calls": calls}}]})
}

fn tool_call(id: &str, name: &str, arguments: Value) -> Value {
    json!({
        "id": id,
        "type": "function",
        "function": {"name": name, "arguments": arguments},
    })
}

/// The golden run's replies after any scorer reply: planner plans,
/// coder writes `hello.txt`, reviewer approves.
fn golden_replies() -> Vec<Result<Value, LlmError>> {
    vec![
        Ok(chat_text(
            "Plan: write a greeting file.\nI will create hello.txt.",
        )),
        Ok(chat_tools(json!([tool_call(
            "call_1",
            "file_write",
            json!({"path": "hello.txt", "content": "hello, world\n"})
        )]))),
        Ok(chat_text("Done. Wrote hello.txt.")),
        Ok(chat_text(
            r#"{"approved": true, "summary": "Looks good.", "issues": []}"#,
        )),
        Ok(chat_text("Done. Wrote hello.txt.")),
        Ok(chat_text(
            r#"{"approved": true, "summary": "Looks good.", "issues": []}"#,
        )),
    ]
}

/// A scorer reply carrying `score`.
fn scorer_reply(score: u32) -> Result<Value, LlmError> {
    Ok(chat_text(&format!(
        r#"{{"score": {score}, "reasons": ["scripted"]}}"#
    )))
}

const SMOKE_CHECK_TOML: &str =
    "[checks.smoke]\ncmd = [\"true\"]\nkind = \"lint\"\nrequired = true\nfiletypes = [\"*\"]\n";

const PREGATE_TOML: &str =
    "[pregate]\nenabled = true\nscorer_model = \"scorer:small\"\ntimeout_ms = 5000\n";

const PREGATE_CHEAP_TOML: &str = "[pregate]\nenabled = true\nscorer_model = \"scorer:small\"\ncheap_model = \"cheap:3b\"\ntimeout_ms = 5000\n";

/// The configured default model (`OllamaConfig::default`).
const DEFAULT_MODEL: &str = "qwen2.5-coder:7b";

fn runtime(
    dir: &std::path::Path,
    toml: &str,
    trusted: bool,
    llm: PregLlm,
) -> Runtime<PregLlm, FakeEditor> {
    let config = load_test_config(dir, toml, trusted);
    Runtime::new(config, llm, FakeEditor::new(), None).expect("runtime builds")
}

fn gated_toml(pregate: &str) -> String {
    format!("{pregate}{SMOKE_CHECK_TOML}")
}

// ---------------------------------------------------------------------------
// Validation half: routing
// ---------------------------------------------------------------------------

#[test]
fn pregate_disabled_by_default_records_nothing() {
    let dir = testdir("disabled");
    let llm = PregLlm::new(golden_replies());
    let mut rt = runtime(&dir, SMOKE_CHECK_TOML, true, llm.clone());
    let report = rt.run("write a greeting file");
    assert_eq!(report["status"], json!("ok"));
    assert!(
        report.get("utility_pregate").is_none(),
        "a disabled gate leaves no record: {report}"
    );
    // The first dispatch is the planner on the configured model: no
    // scorer call happened at all.
    assert_eq!(llm.payload_model(0), DEFAULT_MODEL);
}

#[test]
fn high_score_dispatches_as_planned() {
    let dir = testdir("high");
    let mut script = vec![scorer_reply(95)];
    script.extend(golden_replies());
    let llm = PregLlm::new(script);
    let mut rt = runtime(&dir, &gated_toml(PREGATE_TOML), true, llm.clone());
    let report = rt.run("write a greeting file");
    assert_eq!(report["status"], json!("ok"));
    let record = &report["utility_pregate"];
    assert_eq!(record["route"], json!("as_planned"));
    assert_eq!(record["score"], json!(95));
    assert_eq!(record["scorer_model"], json!("scorer:small"));
    // Call 0 is the scorer (on the scorer model, under the configured
    // 5 s deadline — not the backend's 120 s); call 1 is the planner
    // on the planned model.
    assert_eq!(llm.payload_model(0), "scorer:small");
    assert_eq!(llm.payload_timeout(0), Duration::from_millis(5_000));
    assert_eq!(llm.payload_model(1), DEFAULT_MODEL);
}

#[test]
fn mid_score_with_cheap_model_downgrades_every_role() {
    let dir = testdir("mid-cheap");
    let mut script = vec![scorer_reply(50)];
    script.extend(golden_replies());
    let llm = PregLlm::new(script);
    let mut rt = runtime(&dir, &gated_toml(PREGATE_CHEAP_TOML), true, llm.clone());
    let report = rt.run("write a greeting file");
    assert_eq!(report["status"], json!("ok"));
    let record = &report["utility_pregate"];
    assert_eq!(record["route"], json!("downgraded"));
    assert_eq!(record["downgrade_model"], json!("cheap:3b"));
    // Planner, coder, and reviewer all ran on the cheap model.
    for index in 1..llm.post_count() {
        assert_eq!(llm.payload_model(index), "cheap:3b", "payload {index}");
    }
    let roles = report["roles"].as_array().expect("roles ran");
    assert_eq!(roles.len(), 3);
    for role in roles {
        assert_eq!(role["model"], json!("cheap:3b"));
    }
}

#[test]
fn mid_score_without_cheap_model_narrows_the_cycle_budget() {
    let dir = testdir("mid-narrow");
    // Cycle 1: planner, coder writes, reviewer rejects. A second cycle
    // exists in the script (golden tail) and in the default budget
    // (max_cycles = 2) — the narrowed cap of 1 must stop the run
    // after the first failed cycle.
    let mut script = vec![scorer_reply(50)];
    script.extend([
        Ok(chat_text("Plan: write a greeting file.")),
        Ok(chat_tools(json!([tool_call(
            "call_1",
            "file_write",
            json!({"path": "hello.txt", "content": "hello, world\n"})
        )]))),
        Ok(chat_text("Done. Wrote hello.txt.")),
        Ok(chat_text(
            r#"{"approved": false, "summary": "Not good.", "issues": ["weak"]}"#,
        )),
    ]);
    script.extend(golden_replies());
    let llm = PregLlm::new(script);
    let mut rt = runtime(&dir, &gated_toml(PREGATE_TOML), true, llm.clone());
    let report = rt.run("write a greeting file");
    let record = &report["utility_pregate"];
    assert_eq!(record["route"], json!("downgraded"));
    assert_eq!(record["narrow_max_cycles"], json!(1));
    assert_eq!(
        report["cycles"],
        json!(1),
        "narrowed run stops at one cycle"
    );
    assert_eq!(report["status"], json!("failed"));
    // The planned model still ran (no cheap model configured).
    assert_eq!(llm.payload_model(1), DEFAULT_MODEL);
}

#[test]
fn low_score_queues_without_any_dispatch() {
    let dir = testdir("low");
    let llm = PregLlm::new(vec![scorer_reply(5)]);
    let mut rt = runtime(&dir, &gated_toml(PREGATE_TOML), true, llm.clone());
    let report = rt.run("write a greeting file");
    assert_eq!(report["status"], json!("queued"));
    let record = &report["utility_pregate"];
    assert_eq!(record["route"], json!("queued"));
    assert_eq!(record["score"], json!(5));
    assert_eq!(record["reasons"], json!(["scripted"]));
    // Exactly one model call happened: the scorer's. No planner, no
    // coder, no file.
    assert_eq!(llm.post_count(), 1);
    assert_eq!(report["roles"], json!([]));
    assert!(!dir.join("hello.txt").exists());
}

// ---------------------------------------------------------------------------
// Adversarial half: fail-open and the permission boundary
// ---------------------------------------------------------------------------

#[test]
fn scorer_transport_error_fails_open() {
    let dir = testdir("fail-open-transport");
    let mut script = vec![Err(LlmError::Transport("connection refused".to_owned()))];
    script.extend(golden_replies());
    let llm = PregLlm::new(script);
    let mut rt = runtime(&dir, &gated_toml(PREGATE_TOML), true, llm.clone());
    let report = rt.run("write a greeting file");
    assert_eq!(report["status"], json!("ok"));
    let record = &report["utility_pregate"];
    assert_eq!(record["available"], json!(false));
    assert_eq!(record["error_kind"], json!("transport"));
    assert_eq!(record["route"], json!("as_planned"));
}

#[test]
fn scorer_malformed_reply_fails_open() {
    let dir = testdir("fail-open-malformed");
    let mut script = vec![Ok(chat_text("This task is clearly excellent."))];
    script.extend(golden_replies());
    let llm = PregLlm::new(script);
    let mut rt = runtime(&dir, &gated_toml(PREGATE_TOML), true, llm.clone());
    let report = rt.run("write a greeting file");
    assert_eq!(report["status"], json!("ok"));
    let record = &report["utility_pregate"];
    assert_eq!(record["available"], json!(false));
    assert_eq!(record["error_kind"], json!("malformed"));
    assert_eq!(record["route"], json!("as_planned"));
}

/// The boundary, both directions: the scorer's verdict must not be
/// able to move a permission decision. A maximum score on an
/// untrusted runtime leaves the coder's `file_write` refused; a
/// minimum score on a trusted runtime leaves it allowed.
#[test]
fn scorer_cannot_affect_a_permission_decision() {
    // High score, untrusted: the run dispatches as planned, and the
    // permission gate still refuses the write afterwards.
    let dir = testdir("boundary-high-untrusted");
    let mut script = vec![scorer_reply(100)];
    script.extend(golden_replies());
    let llm = PregLlm::new(script);
    let mut rt = runtime(&dir, &gated_toml(PREGATE_TOML), false, llm.clone());
    let report = rt.run("write a greeting file");
    assert_eq!(report["utility_pregate"]["route"], json!("as_planned"));
    let result = rt.call_tool(
        "file_write",
        &json!({"path": "x.txt", "content": "hi"}),
        "coder",
    );
    assert_eq!(result["status"], json!("error"));
    assert!(!dir.join("x.txt").exists());

    // Low score, trusted: the run is queued, and the permission gate
    // still allows the direct write afterwards — the queue verdict
    // reroutes dispatch, it does not deny permitted work.
    let dir = testdir("boundary-low-trusted");
    let llm = PregLlm::new(vec![scorer_reply(0)]);
    let mut rt = runtime(&dir, &gated_toml(PREGATE_TOML), true, llm.clone());
    let report = rt.run("write a greeting file");
    assert_eq!(report["status"], json!("queued"));
    let result = rt.call_tool(
        "file_write",
        &json!({"path": "x.txt", "content": "hi"}),
        "coder",
    );
    assert_eq!(result["status"], json!("ok"));
    assert_eq!(std::fs::read_to_string(dir.join("x.txt")).unwrap(), "hi");
}
