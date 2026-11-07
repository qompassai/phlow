//! D-1063: contract-variant selection seam (scripted, hermetic).
//!
//! Drives the real `Runtime` planner → coder → verification → reviewer
//! loop with a scripted model transport, on the same bounded task as
//! `closing_turn.rs` (fix `add` in `calc.py`). These tests pin the
//! mechanism, not any variant's quality (that is the live probe's
//! job):
//!
//! - Validation: a selected variant changes the system prompt actually
//!   sent on the wire for its role, and only for its role; with no
//!   selection the payloads carry the canonical prompt bytes.
//! - Adversarial: a variant cannot weaken the strict verdict parser
//!   (garbage under a variant still stays unverified), and a variant
//!   selected for the wrong role panics at the setter boundary instead
//!   of silently falling back.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use phlow_config::{FlowConfig, LoadOptions, load_config};
use phlow_editor::{EditorTransport, SCHEMAS_LUA, TransportError};
use phlow_llm::{LlmError, LlmTransport};
use phlow_runtime::prompt::{
    closing_instruction_for_role_variant, reviewer_verdict_normalized, system_prompt_for_role,
};
use phlow_runtime::{ContractVariant, Runtime};
use serde_json::{Map, Value, json};

// ---------------------------------------------------------------------------
// Test infrastructure (mirrors tests/closing_turn.rs)
// ---------------------------------------------------------------------------

fn testdir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "phlow-contract-variants-test-{}-{}-{}",
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
        ScriptLlm {
            shared: std::sync::Arc::new(std::sync::Mutex::new(ScriptState {
                script: script.into(),
                fallback: chat_text("Fallback. Done."),
                payloads: Vec::new(),
            })),
        }
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

// ---------------------------------------------------------------------------
// The frozen task: fix `add` in calc.py (as in tests/closing_turn.rs)
// ---------------------------------------------------------------------------

const STARTER: &str = "def add(a, b):\n    return a - b\n";
const CORRECT_FIX: &str = "def add(a, b):\n    return a + b\n";
const TESTS: &str = "import sys\n\nfrom calc import add\n\nfailures = []\nfor a, b, want in [(2, 3, 5), (0, 0, 0), (-1, 1, 0), (10, -4, 6)]:\n    got = add(a, b)\n    if got != want:\n        failures.append(f\"add({a}, {b}) = {got}, want {want}\")\nif failures:\n    print(\"FAIL: \" + \"; \".join(failures))\n    sys.exit(1)\nprint(\"PASS\")\n";
const TASK_TOML: &str = "\
[agent]\nmax_iterations = 3\nmax_cycles = 1\n\n[checks.tests]\ncmd = [\"python3\", \"test_calc.py\"]\nkind = \"test\"\nrequired = true\nfiletypes = [\"python\"]\n\n[checks.lint]\ncmd = [\"true\"]\nkind = \"lint\"\nrequired = true\nfiletypes = [\"python\"]\n";
const TASK: &str = "fix add in calc.py so test_calc.py passes";
const APPROVING_VERDICT: &str =
    r#"{"approved": true, "summary": "The fix is correct.", "issues": []}"#;

fn workspace(name: &str) -> PathBuf {
    let dir = testdir(name);
    std::fs::write(dir.join("calc.py"), STARTER).expect("starter");
    std::fs::write(dir.join("test_calc.py"), TESTS).expect("tests");
    dir
}

fn runtime(dir: &std::path::Path, llm: ScriptLlm) -> Runtime<ScriptLlm, FakeEditor> {
    let config = load_test_config(dir, TASK_TOML, true);
    Runtime::new(config, llm, FakeEditor::new(), None).expect("runtime builds")
}

/// The happy-path script: plan, one write, coder summary, verdict.
fn happy_script() -> Vec<Value> {
    vec![
        chat_text("Plan: fix add in calc.py so the tests pass."),
        chat_tools(json!([tool_call(
            "write_0",
            "file_write",
            json!({"path": "calc.py", "content": CORRECT_FIX}),
        )])),
        chat_text("Done. Updated calc.py."),
        chat_text(APPROVING_VERDICT),
    ]
}

/// The system-prompt content of the first payload addressed to `role`.
fn system_prompt_on_wire(payloads: &[Map<String, Value>], role: &str) -> String {
    let needle = format!("Phlow's {role}");
    payloads
        .iter()
        .filter_map(|payload| payload["messages"].as_array())
        .filter_map(|messages| messages.first())
        .filter_map(|message| message["content"].as_str())
        .find(|content| content.contains(&needle))
        .expect("a payload addressed to the role")
        .to_owned()
}

fn role_state(report: &Value, role: &str) -> Value {
    report["roles"]
        .as_array()
        .expect("roles")
        .iter()
        .find(|item| item["role"] == json!(role))
        .expect("the role ran")
        .clone()
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[test]
fn no_selection_sends_canonical_prompt_bytes() {
    let dir = workspace("canonical");
    let llm = ScriptLlm::new(happy_script());
    let mut rt = runtime(&dir, llm.clone());
    let report = rt.run(TASK);
    let payloads = llm.payloads();
    for role in ["planner", "reviewer"] {
        assert_eq!(
            system_prompt_on_wire(&payloads, role),
            system_prompt_for_role(role),
            "{role}: default payload must carry the canonical prompt bytes"
        );
        assert!(
            role_state(&report, role).get("contract_variant").is_none(),
            "{role}: canonical runs record no variant field"
        );
    }
}

#[test]
fn reviewer_serialization_variant_reaches_the_wire() {
    let dir = workspace("reviewer-serialization");
    let llm = ScriptLlm::new(happy_script());
    let mut rt = runtime(&dir, llm.clone());
    rt.set_reviewer_contract_variant(ContractVariant::Serialization);
    let report = rt.run(TASK);
    let payloads = llm.payloads();
    let reviewer_prompt = system_prompt_on_wire(&payloads, "reviewer");
    assert!(reviewer_prompt.starts_with(&system_prompt_for_role("reviewer")));
    assert!(reviewer_prompt.contains("Serialize exactly: close every brace"));
    assert!(reviewer_prompt.contains("its last character is }"));
    // The planner is untouched by a reviewer variant.
    assert_eq!(
        system_prompt_on_wire(&payloads, "planner"),
        system_prompt_for_role("planner")
    );
    assert_eq!(
        role_state(&report, "reviewer")["contract_variant"],
        json!("serialization")
    );
}

#[test]
fn planner_skeleton_variant_reaches_the_wire() {
    let dir = workspace("planner-skeleton");
    let llm = ScriptLlm::new(happy_script());
    let mut rt = runtime(&dir, llm.clone());
    rt.set_planner_contract_variant(ContractVariant::PlanSkeleton);
    let report = rt.run(TASK);
    let payloads = llm.payloads();
    let planner_prompt = system_prompt_on_wire(&payloads, "planner");
    assert!(planner_prompt.starts_with(&system_prompt_for_role("planner")));
    assert!(planner_prompt.contains("Goal / Files / Steps / Checks"));
    // The reviewer is untouched by a planner variant.
    assert_eq!(
        system_prompt_on_wire(&payloads, "reviewer"),
        system_prompt_for_role("reviewer")
    );
    assert_eq!(
        role_state(&report, "planner")["contract_variant"],
        json!("plan-skeleton")
    );
}

#[test]
fn variant_closing_instruction_reaches_the_wire() {
    // A tool-only reviewer under the serialization variant must be
    // rescued by a closing turn whose instruction is the variant's.
    let dir = workspace("variant-closing");
    let mut script = vec![
        chat_text("Plan: fix add in calc.py so the tests pass."),
        chat_tools(json!([tool_call(
            "write_0",
            "file_write",
            json!({"path": "calc.py", "content": CORRECT_FIX}),
        )])),
        chat_text("Done. Updated calc.py."),
    ];
    for index in 0..3 {
        script.push(chat_tools(json!([tool_call(
            &format!("read_{index}"),
            "file_read",
            json!({"path": "calc.py"}),
        )])));
    }
    script.push(chat_text(APPROVING_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, llm.clone());
    rt.set_reviewer_contract_variant(ContractVariant::Serialization);
    let report = rt.run(TASK);
    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["closing_turn"], json!(true));
    assert_eq!(reviewer["approved"], json!(true));
    let payloads = llm.payloads();
    let closing_payload = payloads
        .iter()
        .filter(|payload| {
            payload["messages"].as_array().is_some_and(|messages| {
                messages.iter().any(|message| {
                    message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("iteration budget is exhausted"))
                })
            })
        })
        .last()
        .expect("a closing payload was sent");
    let last_message = closing_payload["messages"]
        .as_array()
        .expect("messages")
        .last()
        .expect("last message")
        .clone();
    assert_eq!(
        last_message["content"],
        json!(closing_instruction_for_role_variant(
            "reviewer",
            ContractVariant::Serialization
        )),
        "the closing payload must carry the variant closing instruction"
    );
    assert!(
        last_message["content"]
            .as_str()
            .expect("content")
            .contains("Close every brace and bracket")
    );
}

// ---------------------------------------------------------------------------
// Adversarial
// ---------------------------------------------------------------------------

#[test]
fn variant_garbage_verdict_stays_unverified() {
    // The serialization variant strengthens wording only: a garbage
    // closing reply still goes through the one strict parse path and
    // stays unverified with approved == false.
    let dir = workspace("variant-garbage");
    let mut script = vec![
        chat_text("Plan: fix add in calc.py so the tests pass."),
        chat_tools(json!([tool_call(
            "write_0",
            "file_write",
            json!({"path": "calc.py", "content": CORRECT_FIX}),
        )])),
        chat_text("Done. Updated calc.py."),
    ];
    for index in 0..3 {
        script.push(chat_tools(json!([tool_call(
            &format!("read_{index}"),
            "file_read",
            json!({"path": "calc.py"}),
        )])));
    }
    script.push(chat_text("approved, probably — trust me"));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, llm.clone());
    rt.set_reviewer_contract_variant(ContractVariant::Serialization);
    let report = rt.run(TASK);
    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["status"], json!("unverified"));
    assert_eq!(reviewer["approved"], json!(false));
    assert_ne!(report["status"], json!("ok"));
}

#[test]
fn shipped_variant_set_keeps_the_single_strict_parse_path() {
    // reviewer_verdict_normalized is the only parse path and no
    // variant changes it: near-JSON (the rung-1/2 residual class — a
    // correct verdict missing its final brace) is unverified, and
    // only the exact object parses.
    let near = r#"{"approved": false, "summary": "defect", "issues": ["x"]"#;
    let verdict = reviewer_verdict_normalized(near);
    assert_eq!(verdict["status"], json!("unverified"));
    assert_eq!(verdict["approved"], json!(false));
    let exact = r#"{"approved": false, "summary": "defect", "issues": ["x"]}"#;
    let verdict = reviewer_verdict_normalized(exact);
    assert!(verdict.get("status").is_none());
    assert_eq!(verdict["approved"], json!(false));
    // Every shipped reviewer variant's closing instruction still
    // demands exactly the verdict JSON and nothing else.
    for variant in [
        ContractVariant::Canonical,
        ContractVariant::Serialization,
        ContractVariant::WorkedExample,
        ContractVariant::DecisionChecklist,
    ] {
        let closing = closing_instruction_for_role_variant("reviewer", variant);
        assert!(
            closing.contains("exactly the verdict JSON and nothing else"),
            "{variant:?} closing weakened the exact-JSON demand"
        );
    }
}

#[test]
#[should_panic(expected = "does not apply to role: reviewer")]
fn planner_variant_rejected_for_the_reviewer() {
    let dir = workspace("mismatch-reviewer");
    let llm = ScriptLlm::new(happy_script());
    let mut rt = runtime(&dir, llm);
    rt.set_reviewer_contract_variant(ContractVariant::PlanSkeleton);
}

#[test]
#[should_panic(expected = "does not apply to role: planner")]
fn reviewer_variant_rejected_for_the_planner() {
    let dir = workspace("mismatch-planner");
    let llm = ScriptLlm::new(happy_script());
    let mut rt = runtime(&dir, llm);
    rt.set_planner_contract_variant(ContractVariant::WorkedExample);
}
