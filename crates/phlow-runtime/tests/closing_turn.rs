//! D-1060/D-1061: forced closing turn + artifact-first role contracts.
//!
//! Drives the real `Runtime` planner → coder → verification → reviewer
//! loop with a scripted model transport, on the same bounded task as
//! `best_of_k.rs` (fix `add` in `calc.py`). The defect under repair: a
//! planner or reviewer that spends every iteration on tool calls reaches
//! budget end as `unverified` ("Role iteration budget exhausted") and
//! emits no artifact at all — the NCM S1 diagnostic (planner died
//! upstream of the coder) and the NCM S2 live measurement (13/20 reviewer
//! runs reached no verdict; 7/7 correct when one arrived).
//!
//! The fix under test: exactly one forced closing turn (empty tool list,
//! artifact demanded now) routed through the normal text-turn finish
//! path, marked `"closing_turn": true` in the role state. Validation
//! tests pin artifact recovery; adversarial tests pin that garbage stays
//! unverified, that a spontaneous text answer makes no extra call, and
//! that the closing turn cannot fire twice.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use phlow_config::{FlowConfig, LoadOptions, load_config};
use phlow_editor::{EditorTransport, SCHEMAS_LUA, TransportError};
use phlow_llm::{LlmError, LlmTransport};
use phlow_runtime::Runtime;
use serde_json::{Map, Value, json};

// ---------------------------------------------------------------------------
// Test infrastructure (mirrors tests/best_of_k.rs)
// ---------------------------------------------------------------------------

fn testdir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "phlow-closing-turn-test-{}-{}-{}",
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

#[derive(Clone)]
struct ScriptLlm {
    shared: std::sync::Arc<std::sync::Mutex<ScriptState>>,
}

struct ScriptState {
    script: VecDeque<Value>,
    fallback: Value,
    payloads: Vec<Map<String, Value>>,
}

impl ScriptLlm {
    fn new(script: Vec<Value>) -> Self {
        Self::with_fallback(script, chat_text("Fallback. Done."))
    }

    /// A script with a distinctive fallback: tests that must prove no
    /// extra model call happened give the fallback a payload (an
    /// approving verdict) whose arrival would flip their assertions.
    fn with_fallback(script: Vec<Value>, fallback: Value) -> Self {
        ScriptLlm {
            shared: std::sync::Arc::new(std::sync::Mutex::new(ScriptState {
                script: script.into(),
                fallback,
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
        Ok(state
            .script
            .pop_front()
            .unwrap_or_else(|| state.fallback.clone()))
    }

    fn get_tags(&mut self, _base_url: &str, _timeout: Duration) -> Result<Value, LlmError> {
        Ok(json!({"models": []}))
    }

    fn close(&mut self) {}
}

fn chat_text(content: &str) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": content}}]})
}

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

/// `count` consecutive tool-only turns reading `calc.py`: the shape of
/// the NCM defect (a role that inspects forever and never speaks).
fn tool_only_turns(count: usize, prefix: &str) -> Vec<Value> {
    (0..count)
        .map(|index| {
            chat_tools(json!([tool_call(
                &format!("{prefix}_{index}"),
                "file_read",
                json!({"path": "calc.py"}),
            )]))
        })
        .collect()
}

#[derive(Clone)]
struct FakeEditor {
    shared: std::sync::Arc<std::sync::Mutex<usize>>,
}

impl FakeEditor {
    fn new() -> Self {
        FakeEditor {
            shared: std::sync::Arc::new(std::sync::Mutex::new(0)),
        }
    }
}

impl EditorTransport for FakeEditor {
    fn exec(
        &mut self,
        expression: &str,
        _args: &[Value],
        _timeout: Duration,
    ) -> Result<Value, TransportError> {
        *self.shared.lock().unwrap() += 1;
        if expression == SCHEMAS_LUA {
            Ok(json!([]))
        } else {
            Err(TransportError::Failed("unexpected expression".to_owned()))
        }
    }

    fn close(&mut self) {}
}

type TestRuntime = Runtime<ScriptLlm, FakeEditor>;

fn runtime(dir: &std::path::Path, toml: &str, llm: ScriptLlm) -> TestRuntime {
    let config = load_test_config(dir, toml, true);
    Runtime::new(config, llm, FakeEditor::new(), None).expect("runtime builds")
}

// ---------------------------------------------------------------------------
// The frozen task: fix `add` in calc.py (as in tests/best_of_k.rs)
// ---------------------------------------------------------------------------

const STARTER: &str = "def add(a, b):\n    return a - b\n";
const TESTS: &str = "\
import sys\n\nfrom calc import add\n\nfailures = []\nfor a, b, want in [(2, 3, 5), (0, 0, 0), (-1, 1, 0), (10, -4, 6)]:\n    got = add(a, b)\n    if got != want:\n        failures.append(f\"add({a}, {b}) = {got}, want {want}\")\nif failures:\n    print(\"FAIL: \" + \"; \".join(failures))\n    sys.exit(1)\nprint(\"PASS\")\n";
const CORRECT_FIX: &str = "def add(a, b):\n    return a + b\n";

/// Three iterations per role keep tool-only scripts short; one cycle
/// keeps every run single-pass so post counts are exact.
const TASK_TOML: &str = "\
[agent]\nmax_iterations = 3\nmax_cycles = 1\n\n[checks.tests]\ncmd = [\"python3\", \"test_calc.py\"]\nkind = \"test\"\nrequired = true\nfiletypes = [\"python\"]\n\n[checks.lint]\ncmd = [\"true\"]\nkind = \"lint\"\nrequired = true\nfiletypes = [\"python\"]\n";

const PLAN_TEXT: &str = "Plan: fix add in calc.py so the tests pass.";
const APPROVE_VERDICT: &str = r#"{"approved": true, "summary": "Tests pass.", "issues": []}"#;
const TASK: &str = "fix add in calc.py so test_calc.py passes";

fn seed_workspace(dir: &std::path::Path) {
    std::fs::write(dir.join("calc.py"), STARTER).expect("starter");
    std::fs::write(dir.join("test_calc.py"), TESTS).expect("tests");
}

/// The coder's two turns: write the correct fix, then report done.
fn coder_turns() -> Vec<Value> {
    vec![
        chat_tools(json!([tool_call(
            "coder_write",
            "file_write",
            json!({"path": "calc.py", "content": CORRECT_FIX}),
        )])),
        chat_text("Done. Updated calc.py."),
    ]
}

fn role_state<'a>(report: &'a Value, role: &str) -> &'a Value {
    report["roles"]
        .as_array()
        .expect("roles array")
        .iter()
        .find(|entry| entry["role"] == json!(role))
        .unwrap_or_else(|| panic!("role {role} ran"))
}

/// True when a recorded payload offered the model no tools: the payload
/// builder omits the `tools` key entirely for an empty tool list.
fn payload_is_tools_off(payload: &Map<String, Value>) -> bool {
    !payload.contains_key("tools")
}

/// The text of the last message in a recorded payload.
fn payload_last_message_text(payload: &Map<String, Value>) -> String {
    payload["messages"]
        .as_array()
        .expect("payload messages")
        .last()
        .and_then(|message| message["content"].as_str())
        .expect("last message content")
        .to_owned()
}

// ---------------------------------------------------------------------------
// Validation: the closing turn recovers the artifact
// ---------------------------------------------------------------------------

#[test]
fn reviewer_closing_turn_recovers_the_verdict() {
    let dir = testdir("reviewer-closing");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.extend(coder_turns());
    script.extend(tool_only_turns(3, "reviewer_look"));
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    assert_eq!(report["status"], json!("ok"));
    assert_eq!(report["verified"], json!(true));
    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["closing_turn"], json!(true));
    assert_eq!(reviewer["status"], json!("ok"));
    assert_eq!(reviewer["approved"], json!(true));
    assert_eq!(reviewer["summary"], json!("Tests pass."));
    assert_eq!(reviewer["turns"], json!(4), "3 tool turns + 1 closing turn");
    assert_eq!(role_state(&report, "planner").get("closing_turn"), None);
    assert_eq!(role_state(&report, "coder").get("closing_turn"), None);

    // Exactly one extra model call, made with tools off and the closing
    // instruction as the final message.
    assert_eq!(llm.post_count(), 7);
    assert_eq!(report["model_calls"], json!(7));
    let payloads = llm.payloads();
    assert!(
        !payload_is_tools_off(&payloads[5]),
        "budget turns offer tools"
    );
    let closing = payloads.last().expect("closing payload");
    assert!(
        payload_is_tools_off(closing),
        "closing turn offers no tools"
    );
    assert!(
        payload_last_message_text(closing).contains("exactly the verdict JSON"),
        "closing payload demands the verdict"
    );
}

#[test]
fn planner_closing_turn_recovers_the_plan_and_the_run_proceeds() {
    let dir = testdir("planner-closing");
    seed_workspace(&dir);
    let mut script = tool_only_turns(3, "planner_look");
    script.push(chat_text(PLAN_TEXT));
    script.extend(coder_turns());
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    let planner = role_state(&report, "planner");
    assert_eq!(planner["closing_turn"], json!(true));
    assert_eq!(planner["status"], json!("ok"));
    assert_eq!(planner["summary"], json!(PLAN_TEXT));
    assert_eq!(planner["turns"], json!(4), "3 tool turns + 1 closing turn");
    // The run proceeded past planning to a verified completion.
    assert_eq!(report["status"], json!("ok"));
    assert_eq!(report["verified"], json!(true));
    assert_eq!(llm.post_count(), 7);
    let payloads = llm.payloads();
    let closing = &payloads[3];
    assert!(
        payload_is_tools_off(closing),
        "closing turn offers no tools"
    );
    assert!(
        payload_last_message_text(closing).contains("concrete plan as plain text"),
        "closing payload demands the plan"
    );
}

#[test]
fn immediate_answers_make_no_closing_turn() {
    let dir = testdir("immediate");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.extend(coder_turns());
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    assert_eq!(report["status"], json!("ok"));
    // No extra model call for any role: 1 planner + 2 coder + 1 reviewer.
    assert_eq!(llm.post_count(), 4);
    assert_eq!(report["model_calls"], json!(4));
    for role in ["planner", "coder", "reviewer"] {
        assert_eq!(
            role_state(&report, role).get("closing_turn"),
            None,
            "{role} answered in text; no closing turn may fire"
        );
    }
    assert_eq!(role_state(&report, "planner")["turns"], json!(1));
    assert_eq!(role_state(&report, "reviewer")["turns"], json!(1));
}

// ---------------------------------------------------------------------------
// Adversarial: the closing turn cannot manufacture an artifact
// ---------------------------------------------------------------------------

#[test]
fn reviewer_closing_turn_garbage_stays_unverified() {
    let dir = testdir("reviewer-garbage");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.extend(coder_turns());
    script.extend(tool_only_turns(3, "reviewer_look"));
    script.push(chat_text("I looked at everything and it seems fine to me."));
    // The fallback is an approving verdict: if the runtime made any
    // further call, the reviewer would flip to approved and fail here.
    let llm = ScriptLlm::with_fallback(script, chat_text(APPROVE_VERDICT));
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["closing_turn"], json!(true));
    assert_eq!(reviewer["status"], json!("unverified"));
    assert_eq!(reviewer["approved"], json!(false), "never a false approval");
    assert_ne!(report["status"], json!("ok"));
    assert_ne!(report["verified"], json!(true));
    assert_eq!(llm.post_count(), 7, "no call beyond the closing turn");
}

#[test]
fn closing_turn_fires_at_most_once() {
    let dir = testdir("closing-once");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.extend(coder_turns());
    script.extend(tool_only_turns(3, "reviewer_look"));
    // The closing reply itself tries to call a tool again, with no text.
    script.push(chat_tools(json!([tool_call(
        "one_more_look",
        "file_read",
        json!({"path": "calc.py"}),
    )])));
    // Any second closing turn (or a resumed loop) would consume this
    // approving fallback and flip the verdict.
    let llm = ScriptLlm::with_fallback(script, chat_text(APPROVE_VERDICT));
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["closing_turn"], json!(true));
    assert_eq!(reviewer["turns"], json!(4), "budget + exactly one closing");
    assert_eq!(reviewer["approved"], json!(false));
    assert_eq!(reviewer["status"], json!("unverified"));
    assert_eq!(llm.post_count(), 7, "the closing turn cannot re-fire");
    assert_ne!(report["status"], json!("ok"));
}

#[test]
fn planner_closing_turn_garbage_records_like_a_text_turn() {
    // Parity invariant: a garbage closing reply degrades exactly like a
    // garbage spontaneous text reply — it becomes the plan text and the
    // run proceeds; nothing is repaired or reinterpreted.
    let dir = testdir("planner-garbage");
    seed_workspace(&dir);
    let mut script = tool_only_turns(3, "planner_look");
    script.push(chat_text("lorem ipsum, not a plan"));
    script.extend(coder_turns());
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    let planner = role_state(&report, "planner");
    assert_eq!(planner["closing_turn"], json!(true));
    assert_eq!(planner["status"], json!("ok"));
    assert_eq!(planner["summary"], json!("lorem ipsum, not a plan"));
    assert_eq!(report["status"], json!("ok"));
    assert_eq!(llm.post_count(), 7);
}
