//! Intake tests: the Mandate, the Eval layer, and resume revalidation,
//! exercised through the runtime against scripted fakes (no network, no
//! Neovim). Mirrors the harness shape of `tests/runtime.rs`.
//!
//! Half validation (the layers behave as specified on well-formed
//! input), half adversarial (a rejected task must never reach the model;
//! an Eval must never widen the Mandate's bounds; a mutated spec or a
//! changed config must halt a resume).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use phlow_config::{FlowConfig, LoadOptions, load_config};
use phlow_editor::{EditorTransport, SCHEMAS_LUA, TransportError};
use phlow_llm::{LlmError, LlmTransport};
use phlow_runtime::Runtime;
use phlow_runtime::intake::{TaskBounds, TaskScope, TaskSpec};
use serde_json::{Map, Value, json};

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

fn testdir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "phlow-intake-test-{}-{}-{}",
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

fn chat_text(content: &str) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": content}}]})
}

/// Scripted model transport: pops chat replies in order and records
/// every payload, so tests can prove whether (and how) the model was
/// consulted.
#[derive(Clone)]
struct ScriptLlm {
    shared: Arc<Mutex<ScriptState>>,
}

struct ScriptState {
    script: VecDeque<Value>,
    payloads: Vec<Map<String, Value>>,
}

impl ScriptLlm {
    fn new(script: Vec<Value>) -> Self {
        ScriptLlm {
            shared: Arc::new(Mutex::new(ScriptState {
                script: script.into(),
                payloads: Vec::new(),
            })),
        }
    }

    fn post_count(&self) -> usize {
        self.shared.lock().unwrap().payloads.len()
    }

    fn payloads(&self) -> Vec<Map<String, Value>> {
        self.shared.lock().unwrap().payloads.clone()
    }
}

impl LlmTransport for ScriptLlm {
    fn post_chat(
        &mut self,
        _base_url: &str,
        payload: &Map<String, Value>,
        _timeout: Duration,
    ) -> Result<Value, LlmError> {
        let mut state = self.shared.lock().unwrap();
        state.payloads.push(payload.clone());
        state
            .script
            .pop_front()
            .ok_or_else(|| LlmError::Transport("script exhausted".to_owned()))
    }

    fn get_tags(&mut self, _base_url: &str, _timeout: Duration) -> Result<Value, LlmError> {
        Ok(json!({"models": []}))
    }

    fn close(&mut self) {}
}

/// Editor transport serving no schemas and no calls; intake never
/// touches the editor, it only has to not interfere with construction.
struct FakeEditor {
    schemas: Value,
    calls: HashMap<String, Value>,
}

impl FakeEditor {
    fn new() -> Self {
        FakeEditor {
            schemas: json!([]),
            calls: HashMap::new(),
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
            Ok(self.schemas.clone())
        } else {
            let name = args.first().and_then(|arg| arg.as_str()).unwrap_or("");
            Ok(self
                .calls
                .get(name)
                .cloned()
                .unwrap_or(json!({"status": "ok"})))
        }
    }

    fn close(&mut self) {}
}

type TestRuntime = Runtime<ScriptLlm, FakeEditor>;

fn runtime(dir: &std::path::Path, toml: &str, trusted: bool, llm: ScriptLlm) -> TestRuntime {
    let config = load_test_config(dir, toml, trusted);
    Runtime::new(config, llm, FakeEditor::new(), None).expect("runtime builds")
}

const INTAKE_TOML: &str = "\
[ollama]\nmodel = \"default-model\"\n
[agent]\nmax_iterations = 16\nmax_cycles = 3\nmax_tool_calls = 64\nmax_context_chars = 100000\nmax_task_chars = 10000\n
[checks.smoke]\ncmd = [\"true\"]\nkind = \"lint\"\nrequired = true\nfiletypes = [\"*\"]\n";

/// Same host, with the tool-call ceiling tightened below the spec's
/// declared bound — the config change a resume must not survive.
const TIGHTENED_TOML: &str = "\
[ollama]\nmodel = \"default-model\"\n
[agent]\nmax_iterations = 16\nmax_cycles = 3\nmax_tool_calls = 16\nmax_context_chars = 100000\nmax_task_chars = 10000\n
[checks.smoke]\ncmd = [\"true\"]\nkind = \"lint\"\nrequired = true\nfiletypes = [\"*\"]\n";

fn spec() -> TaskSpec {
    TaskSpec {
        id: "intake-1".to_owned(),
        prompt: "Write a greeting file.".to_owned(),
        scope: TaskScope {
            paths: vec![PathBuf::from("src")],
            tools: vec!["file_read".to_owned(), "file_write".to_owned()],
            write: true,
        },
        bounds: TaskBounds {
            max_iterations: 8,
            max_cycles: 2,
            max_tool_calls: 32,
            max_context_chars: 65_536,
        },
        required_checks: vec!["smoke".to_owned()],
    }
}

fn eval_reply(narrowed_tool_calls: u32) -> Value {
    chat_text(&format!(
        "{{\"clarity\":\"clear\",\"fit\":\"fits\",\"risk_notes\":[\"small\"],\
         \"narrowed_bounds\":{{\"max_iterations\":8,\"max_cycles\":2,\
         \"max_tool_calls\":{narrowed_tool_calls},\"max_context_chars\":65536}},\
         \"recommendation\":\"proceed\"}}"
    ))
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[test]
fn intake_admits_and_stores_the_approval() {
    let dir = testdir("admits");
    let llm = ScriptLlm::new(vec![eval_reply(16)]);
    let mut rt = runtime(&dir, INTAKE_TOML, true, llm.clone());
    let task = spec();
    let report = rt.intake_task(&task);
    assert_eq!(report["status"], json!("admitted"));
    assert_eq!(
        report["admission"]["effective_bounds"]["max_tool_calls"],
        json!(16)
    );
    assert_eq!(
        report["admission"]["spec_fingerprint"],
        json!(task.fingerprint())
    );
    assert!(rt.last_approval().is_some());
    assert!(rt.last_admission().is_some());
    // Exactly one model call, carrying no tools: the Eval layer assesses,
    // it never acts.
    assert_eq!(llm.post_count(), 1);
    let payloads = llm.payloads();
    if let Some(tools) = payloads[0].get("tools") {
        assert_eq!(tools, &json!([]));
    }
    let messages = payloads[0]["messages"].as_array().expect("messages");
    assert_eq!(messages[0]["role"], json!("system"));
    let user = messages[1]["content"].as_str().expect("user content");
    assert!(user.contains(&task.fingerprint()));
}

#[test]
fn resume_task_resumes_an_unchanged_spec() {
    let dir = testdir("resume-ok");
    let llm = ScriptLlm::new(vec![eval_reply(32)]);
    let mut rt = runtime(&dir, INTAKE_TOML, true, llm);
    let task = spec();
    assert_eq!(rt.intake_task(&task)["status"], json!("admitted"));
    let report = rt.resume_task(&task);
    assert_eq!(report["status"], json!("resumed"));
    assert_eq!(report["limits_changed"], json!(false));
}

// ---------------------------------------------------------------------------
// Adversarial
// ---------------------------------------------------------------------------

#[test]
fn intake_rejection_never_calls_the_model() {
    let dir = testdir("rejected");
    let llm = ScriptLlm::new(vec![]);
    let mut rt = runtime(&dir, INTAKE_TOML, true, llm.clone());
    let mut hostile = spec();
    hostile.scope.tools.push("shell_exec".to_owned());
    let report = rt.intake_task(&hostile);
    assert_eq!(report["status"], json!("rejected"));
    assert_eq!(
        report["mandate"]["reasons"][0]["kind"],
        json!("unknown_tool")
    );
    // The Mandate is the first layer: a rejected task is never shown to
    // a model, so there is nothing for a prompt to talk its way past.
    assert_eq!(llm.post_count(), 0);
    assert!(rt.last_approval().is_none());
}

#[test]
fn intake_refuses_an_eval_that_widens_the_approved_bounds() {
    let dir = testdir("widening");
    // The assessor suggests a tool-call bound far above the approved 32.
    let llm = ScriptLlm::new(vec![eval_reply(999)]);
    let mut rt = runtime(&dir, INTAKE_TOML, true, llm);
    let report = rt.intake_task(&spec());
    assert_eq!(report["status"], json!("refused"));
    let error = report["error"].as_str().expect("error text");
    assert!(error.contains("wider"), "unexpected error: {error}");
    // The Mandate approval stands (it was honest); no admission exists.
    assert!(rt.last_approval().is_some());
    assert!(rt.last_admission().is_none());
}

#[test]
fn intake_reports_a_malformed_eval_without_dying() {
    let dir = testdir("eval-garbage");
    let llm = ScriptLlm::new(vec![chat_text("Sure, looks fine to me.")]);
    let mut rt = runtime(&dir, INTAKE_TOML, true, llm);
    let report = rt.intake_task(&spec());
    assert_eq!(report["status"], json!("eval_failed"));
    assert!(rt.last_admission().is_none());
}

#[test]
fn resume_task_halts_a_scope_mutated_after_approval() {
    let dir = testdir("resume-mutated");
    let llm = ScriptLlm::new(vec![eval_reply(32)]);
    let mut rt = runtime(&dir, INTAKE_TOML, true, llm);
    let task = spec();
    assert_eq!(rt.intake_task(&task)["status"], json!("admitted"));
    // The same task, quietly grown a second path while it waited.
    let mut mutated = task.clone();
    mutated.scope.paths.push(PathBuf::from("other"));
    let report = rt.resume_task(&mutated);
    assert_eq!(report["status"], json!("halted"));
    assert_eq!(report["revalidation"]["spec_mutated"], json!(true));
}

#[test]
fn resume_with_halts_after_a_config_change_lowers_the_ceiling() {
    let dir = testdir("resume-config");
    let llm = ScriptLlm::new(vec![eval_reply(32)]);
    let mut rt = runtime(&dir, INTAKE_TOML, true, llm);
    let task = spec();
    assert_eq!(rt.intake_task(&task)["status"], json!("admitted"));
    let approval = rt.last_approval().expect("approval").clone();
    drop(rt);
    // A new process on the same workspace, under a tightened config:
    // the approval record is presented for resume and must not carry.
    let llm2 = ScriptLlm::new(vec![]);
    let mut rt2 = runtime(&dir, TIGHTENED_TOML, true, llm2);
    let report = rt2.resume_with(&approval, &task);
    assert_eq!(report["status"], json!("halted"));
    assert_eq!(report["revalidation"]["spec_mutated"], json!(false));
    assert_eq!(
        report["revalidation"]["reasons"][0]["kind"],
        json!("bound_exceeds_ceiling")
    );
}

#[test]
fn resume_task_without_an_approval_halts() {
    let dir = testdir("resume-none");
    let llm = ScriptLlm::new(vec![]);
    let mut rt = runtime(&dir, INTAKE_TOML, true, llm);
    let report = rt.resume_task(&spec());
    assert_eq!(report["status"], json!("halted"));
}
