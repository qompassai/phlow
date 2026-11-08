//! D-1063: contract-variant selection seam + rung-3 adoption
//! (scripted, hermetic).
//!
//! Drives the real `Runtime` planner → coder → verification → reviewer
//! loop with a scripted model transport, on the same bounded task as
//! `closing_turn.rs` (fix `add` in `calc.py`). These tests pin the
//! mechanism and its compose outcome, not any variant's quality (that
//! was the live probe's job):
//!
//! - Validation: with no selection the payloads carry the canonical
//!   prompt bytes; the clauses adopted at rung 3 (reviewer
//!   serialization, planner skeleton) are part of those canonical
//!   bytes and reach the wire by default, including in the closing
//!   instruction.
//! - Adversarial: the adopted wording cannot weaken the strict
//!   verdict parser (garbage still stays unverified), near-JSON is
//!   still unverified through the single parse path, and the retired
//!   pool's ids panic instead of silently resolving.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use phlow_config::{FlowConfig, LoadOptions, load_config};
use phlow_editor::{EditorTransport, SCHEMAS_LUA, TransportError};
use phlow_llm::{LlmError, LlmTransport};
use phlow_runtime::prompt::{
    closing_instruction_for_role, closing_instruction_for_role_variant,
    reviewer_verdict_normalized, system_prompt_for_role, system_prompt_for_role_variant,
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

/// A script whose reviewer reads calc.py on every turn (tool-only),
/// then answers the closing turn with `closing_reply`.
fn tool_only_reviewer_script(closing_reply: &str) -> Vec<Value> {
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
    script.push(chat_text(closing_reply));
    script
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
fn adopted_reviewer_clause_reaches_the_wire_by_default() {
    // D-1063 adoption: the serialization clause is canonical now, so
    // the default reviewer payload carries it with no selection.
    let dir = workspace("adopted-reviewer");
    let llm = ScriptLlm::new(happy_script());
    let mut rt = runtime(&dir, llm.clone());
    let report = rt.run(TASK);
    let payloads = llm.payloads();
    let reviewer_prompt = system_prompt_on_wire(&payloads, "reviewer");
    assert!(reviewer_prompt.contains("Serialize exactly: close every brace"));
    assert!(reviewer_prompt.contains("its last character is }"));
    assert!(
        role_state(&report, "reviewer")
            .get("contract_variant")
            .is_none(),
        "the adopted clause is canonical, not a variant selection"
    );
}

#[test]
fn adopted_planner_clause_reaches_the_wire_by_default() {
    let dir = workspace("adopted-planner");
    let llm = ScriptLlm::new(happy_script());
    let mut rt = runtime(&dir, llm.clone());
    rt.run(TASK);
    let payloads = llm.payloads();
    let planner_prompt = system_prompt_on_wire(&payloads, "planner");
    assert!(planner_prompt.contains("Goal / Files / Steps / Checks"));
    assert!(planner_prompt.contains("name the exact test or command"));
}

#[test]
fn adopted_closing_instruction_reaches_the_wire() {
    // A tool-only reviewer must be rescued by a closing turn whose
    // instruction is the canonical (adopted) closing text, verbatim.
    let dir = workspace("adopted-closing");
    let llm = ScriptLlm::new(tool_only_reviewer_script(APPROVING_VERDICT));
    let mut rt = runtime(&dir, llm.clone());
    let report = rt.run(TASK);
    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["closing_turn"], json!(true));
    assert_eq!(reviewer["approved"], json!(true));
    let payloads = llm.payloads();
    let closing_payload = payloads
        .iter()
        .rev()
        .find(|payload| {
            payload["messages"].as_array().is_some_and(|messages| {
                messages.iter().any(|message| {
                    message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("iteration budget is exhausted"))
                })
            })
        })
        .expect("a closing payload was sent");
    let last_message = closing_payload["messages"]
        .as_array()
        .expect("messages")
        .last()
        .expect("last message")
        .clone();
    assert_eq!(
        last_message["content"],
        json!(closing_instruction_for_role("reviewer")),
        "the closing payload must carry the canonical closing instruction"
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
fn garbage_verdict_stays_unverified_under_the_adopted_contract() {
    // The adopted clause strengthens wording only: a garbage closing
    // reply still goes through the one strict parse path and stays
    // unverified with approved == false.
    let dir = workspace("adopted-garbage");
    let llm = ScriptLlm::new(tool_only_reviewer_script("approved, probably — trust me"));
    let mut rt = runtime(&dir, llm.clone());
    let report = rt.run(TASK);
    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["status"], json!("unverified"));
    assert_eq!(reviewer["approved"], json!(false));
    assert_ne!(report["status"], json!("ok"));
}

#[test]
fn shipped_contract_keeps_the_single_strict_parse_path() {
    // reviewer_verdict_normalized is the only parse path: near-JSON
    // (the rung-1/2 residual class — a correct verdict missing its
    // final brace) is unverified, and only the exact object parses.
    let near = r#"{"approved": false, "summary": "defect", "issues": ["x"]"#;
    let verdict = reviewer_verdict_normalized(near);
    assert_eq!(verdict["status"], json!("unverified"));
    assert_eq!(verdict["approved"], json!(false));
    let exact = r#"{"approved": false, "summary": "defect", "issues": ["x"]}"#;
    let verdict = reviewer_verdict_normalized(exact);
    assert!(verdict.get("status").is_none());
    assert_eq!(verdict["approved"], json!(false));
    // The shipped closing instruction still demands exactly the
    // verdict JSON and nothing else.
    let closing = closing_instruction_for_role("reviewer");
    assert!(closing.contains("exactly the verdict JSON and nothing else"));
}

#[test]
#[should_panic(expected = "unknown contract variant id: serialization")]
fn retired_reviewer_variant_id_panics() {
    let _ = ContractVariant::from_id("serialization");
}

#[test]
#[should_panic(expected = "unknown contract variant id: plan-skeleton")]
fn retired_planner_variant_id_panics() {
    let _ = ContractVariant::from_id("plan-skeleton");
}

// ---------------------------------------------------------------------------
// D-1075 rung 3: coder variants (validation + adversarial)
// ---------------------------------------------------------------------------

#[test]
fn coder_variant_reaches_the_wire_and_records_provenance() {
    let dir = workspace("coder-variant");
    let llm = ScriptLlm::new(happy_script());
    let mut rt = runtime(&dir, llm.clone());
    rt.set_coder_contract_variant(ContractVariant::CheckPerWrite);
    let report = rt.run(TASK);
    let payloads = llm.payloads();
    let coder_prompt = system_prompt_on_wire(&payloads, "coder");
    assert_eq!(
        coder_prompt,
        system_prompt_for_role_variant("coder", ContractVariant::CheckPerWrite)
    );
    assert!(coder_prompt.starts_with(&system_prompt_for_role("coder")));
    assert!(coder_prompt.contains("after EVERY file_write, run flow_check"));
    let coder = role_state(&report, "coder");
    assert_eq!(coder["contract_variant"], json!("check-per-write"));
    assert_eq!(coder["status"], json!("ok"));
    // Other roles stay canonical and record nothing.
    for role in ["planner", "reviewer"] {
        assert!(role_state(&report, role).get("contract_variant").is_none());
    }
}

/// A script whose coder writes on every turn (never speaks), then
/// answers the closing turn with a summary; the reviewer approves.
fn tool_only_coder_script(closing_reply: &str) -> Vec<Value> {
    let mut script = vec![chat_text("Plan: fix add in calc.py so the tests pass.")];
    for index in 0..3 {
        script.push(chat_tools(json!([tool_call(
            &format!("write_{index}"),
            "file_write",
            json!({"path": "calc.py", "content": CORRECT_FIX}),
        )])));
    }
    script.push(chat_text(closing_reply));
    script.push(chat_text(APPROVING_VERDICT));
    script
}

#[test]
fn coder_variant_closing_instruction_reaches_the_wire() {
    let dir = workspace("coder-variant-closing");
    let llm = ScriptLlm::new(tool_only_coder_script("Done. Updated calc.py; checks ok."));
    let mut rt = runtime(&dir, llm.clone());
    rt.set_coder_contract_variant(ContractVariant::StrictStop);
    let report = rt.run(TASK);
    let coder = role_state(&report, "coder");
    assert_eq!(coder["closing_turn"], json!(true));
    assert_eq!(coder["status"], json!("ok"));
    let payloads = llm.payloads();
    let closing_payload = payloads
        .iter()
        .rev()
        .find(|payload| {
            payload["messages"].as_array().is_some_and(|messages| {
                messages.iter().any(|message| {
                    message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("iteration budget is exhausted"))
                })
            })
        })
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
            "coder",
            ContractVariant::StrictStop
        )),
        "the coder closing payload must carry the variant closing instruction"
    );
    assert!(
        last_message["content"]
            .as_str()
            .expect("content")
            .contains("report the first flow_check status")
    );
}

#[test]
#[should_panic(expected = "contract variant check-per-write does not apply to role: planner")]
fn coder_variant_cannot_be_set_on_the_planner() {
    let dir = workspace("coder-variant-mismatch");
    let llm = ScriptLlm::new(happy_script());
    let mut rt = runtime(&dir, llm);
    rt.set_planner_contract_variant(ContractVariant::CheckPerWrite);
}

#[test]
#[should_panic(expected = "contract variant strict-stop does not apply to role: reviewer")]
fn coder_variant_cannot_be_set_on_the_reviewer() {
    let dir = workspace("coder-variant-mismatch-reviewer");
    let llm = ScriptLlm::new(happy_script());
    let mut rt = runtime(&dir, llm);
    rt.set_reviewer_contract_variant(ContractVariant::StrictStop);
}
