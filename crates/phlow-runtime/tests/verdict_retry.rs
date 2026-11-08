//! D-1076: reviewer verdict retry on a strict-parse failure.
//!
//! The failure class, measured in the D-1075 S1 re-measure (rung 4):
//! reviewer approvals written as prose with an embedded JSON verdict —
//! `... {"approved":true,...}` behind a prose preamble — are rejected
//! by the byte-strict verdict parser, so a run whose coder patch stood
//! host-verified still failed on the reviewer stage. The ruling's fix
//! is contract-side: on a parse failure the reviewer gets exactly one
//! retry turn carrying its own raw text, the required verdict format
//! restated, and an instruction to re-emit only the verdict. The
//! parser is not loosened; the retry's reply parses through the same
//! strict path, and an unparseable retry keeps the recorded parse
//! failure. celsius.A — a verdict that parsed but judged wrongly — is
//! out of scope: the retry repairs form, never judgment.
//!
//! Drives the real `Runtime` planner → coder → verification → reviewer
//! loop with a scripted model transport, on the same bounded task as
//! `closing_turn.rs` (fix `add` in `calc.py`). Validation tests pin
//! recovery and the retry prompt's construction; adversarial tests pin
//! that garbage stays unverified, that exactly one retry fires, that
//! the planner and coder never retry, and that a retry verdict of
//! reject is honored exactly like a first-pass reject.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use phlow_config::{FlowConfig, LoadOptions, load_config};
use phlow_editor::{EditorTransport, SCHEMAS_LUA, TransportError};
use phlow_llm::{LlmError, LlmTransport};
use phlow_runtime::Runtime;
use serde_json::{Map, Value, json};

// ---------------------------------------------------------------------------
// Test infrastructure (mirrors tests/closing_turn.rs)
// ---------------------------------------------------------------------------

fn testdir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "phlow-verdict-retry-test-{}-{}-{}",
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

/// `count` consecutive tool-only turns reading `calc.py`.
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
// The frozen task: fix `add` in calc.py (as in tests/closing_turn.rs)
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
const REJECT_VERDICT: &str =
    r#"{"approved": false, "summary": "Wrong operator.", "issues": ["add still subtracts"]}"#;
/// The mean.A/dedupe.A shape: a correct approval buried behind prose,
/// so the strict parser sees prose first and rejects the whole reply.
const PROSE_APPROVAL: &str = "I reviewed the patch and the tests now pass. \
    {\"approved\": true, \"summary\": \"Tests pass.\", \"issues\": []}";
/// The same shape carrying a rejection.
const PROSE_REJECTION: &str = "The patch does not fix add. \
    {\"approved\": false, \"summary\": \"Wrong operator.\", \"issues\": [\"add still subtracts\"]}";
/// The exact format template the retry instruction must restate.
const VERDICT_FORMAT: &str = "{\"approved\":true|false,\"summary\":\"...\",\"issues\":[\"...\"]}";
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
// Validation: the retry recovers the verdict the prose buried
// ---------------------------------------------------------------------------

#[test]
fn prose_approval_is_recovered_by_the_retry() {
    let dir = testdir("retry-recovers");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.extend(coder_turns());
    script.push(chat_text(PROSE_APPROVAL));
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    // The retry's verdict is the one that counts: the run completes
    // approved on a host-verified patch, exactly the mean.A shape.
    assert_eq!(report["status"], json!("ok"));
    assert_eq!(report["verified"], json!(true));
    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["verdict_retry"], json!(true));
    assert_eq!(reviewer["status"], json!("ok"));
    assert_eq!(reviewer["approved"], json!(true));
    assert_eq!(reviewer["summary"], json!("Tests pass."));
    assert_eq!(reviewer.get("error"), None, "no stale parse error");
    assert_eq!(reviewer["turns"], json!(2), "verdict turn + retry turn");
    assert_eq!(llm.post_count(), 5);
    assert_eq!(report["model_calls"], json!(5));
    let payloads = llm.payloads();
    let retry = payloads.last().expect("retry payload");
    assert!(payload_is_tools_off(retry), "retry turn offers no tools");
}

#[test]
fn retry_instruction_quotes_the_raw_text_and_restates_the_format() {
    let dir = testdir("retry-prompt");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.extend(coder_turns());
    script.push(chat_text(PROSE_APPROVAL));
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let _report = rt.run(TASK);

    let payloads = llm.payloads();
    let instruction = payload_last_message_text(payloads.last().expect("retry payload"));
    assert!(
        instruction.contains(PROSE_APPROVAL),
        "retry hands the reviewer its own raw text back verbatim"
    );
    assert!(
        instruction.contains(VERDICT_FORMAT),
        "retry restates the exact required verdict format"
    );
    assert!(
        instruction.contains("Re-emit only the verdict"),
        "retry instructs re-emission of the verdict only"
    );
}

#[test]
fn clean_verdict_makes_no_retry() {
    let dir = testdir("no-retry");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.extend(coder_turns());
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["approved"], json!(true));
    assert_eq!(reviewer.get("verdict_retry"), None);
    assert_eq!(reviewer["turns"], json!(1));
    assert_eq!(llm.post_count(), 4, "no extra model call was made");
}

// ---------------------------------------------------------------------------
// Adversarial: the retry cannot invent, repeat, leak, or bias a verdict
// ---------------------------------------------------------------------------

#[test]
fn unparseable_retry_stays_unverified_with_the_parse_error() {
    let dir = testdir("retry-still-bad");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.extend(coder_turns());
    script.push(chat_text(PROSE_APPROVAL));
    script.push(chat_text("Still just prose, no verdict JSON."));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    // The parse failure is the recorded outcome: never a silent pass,
    // never a fabricated verdict.
    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["verdict_retry"], json!(true));
    assert_eq!(reviewer["status"], json!("unverified"));
    assert_eq!(reviewer["approved"], json!(false));
    assert_eq!(
        reviewer["error"],
        json!("Reviewer must return a structured JSON verdict")
    );
    assert_ne!(report["status"], json!("ok"));
    assert_eq!(llm.post_count(), 5);
}

#[test]
fn exactly_one_retry_even_when_the_fallback_would_answer() {
    let dir = testdir("one-retry-only");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.extend(coder_turns());
    script.push(chat_text(PROSE_APPROVAL));
    script.push(chat_text("No JSON here either."));
    // The fallback is a valid approving verdict: a second retry would
    // consume it and flip the outcome, so its non-arrival proves the
    // retry fired exactly once.
    let llm = ScriptLlm::with_fallback(script, chat_text(APPROVE_VERDICT));
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["status"], json!("unverified"));
    assert_eq!(reviewer["approved"], json!(false));
    assert_eq!(llm.post_count(), 5, "planner + coder + verdict + one retry");
}

#[test]
fn planner_and_coder_finishes_never_trigger_a_retry() {
    let dir = testdir("reviewer-only");
    seed_workspace(&dir);
    // The planner answers in plain prose and the coder reports in
    // prose: neither text is a verdict, and neither may spend a retry.
    let mut script = vec![chat_text("A prose plan, no JSON anywhere.")];
    script.extend(coder_turns());
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    assert_eq!(report["status"], json!("ok"));
    assert_eq!(role_state(&report, "planner").get("verdict_retry"), None);
    assert_eq!(role_state(&report, "coder").get("verdict_retry"), None);
    assert_eq!(role_state(&report, "reviewer").get("verdict_retry"), None);
    assert_eq!(llm.post_count(), 4);
}

#[test]
fn retry_reject_verdict_is_honored_exactly() {
    let dir = testdir("retry-reject");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.extend(coder_turns());
    script.push(chat_text(PROSE_REJECTION));
    script.push(chat_text(REJECT_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    // No approval bias: a retry verdict of reject lands exactly like a
    // first-pass reject, issues and all, though the patch verifies.
    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["verdict_retry"], json!(true));
    assert_eq!(reviewer["status"], json!("ok"));
    assert_eq!(reviewer["approved"], json!(false));
    assert_eq!(reviewer["summary"], json!("Wrong operator."));
    assert_eq!(reviewer["issues"], json!(["add still subtracts"]));
    assert_ne!(report["status"], json!("ok"));
    assert_eq!(llm.post_count(), 5);
}

#[test]
fn empty_closing_reply_is_not_a_verdict_attempt_and_gets_no_retry() {
    let dir = testdir("empty-no-retry");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.extend(coder_turns());
    script.extend(tool_only_turns(3, "reviewer_look"));
    // The closing turn's reply attempts another tool call with no
    // text at all: no verdict was attempted, so there is no raw text
    // for a retry to hand back — the failure stays the closing turn's
    // domain (a failure to speak, not a failure of form).
    script.push(chat_tools(json!([tool_call(
        "one_more_look",
        "file_read",
        json!({"path": "calc.py"}),
    )])));
    // The fallback is an approving verdict: a retry would consume it
    // and flip the outcome, so its non-arrival pins the boundary.
    let llm = ScriptLlm::with_fallback(script, chat_text(APPROVE_VERDICT));
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["closing_turn"], json!(true));
    assert_eq!(reviewer.get("verdict_retry"), None);
    assert_eq!(reviewer["status"], json!("unverified"));
    assert_eq!(reviewer["approved"], json!(false));
    assert_eq!(reviewer["turns"], json!(4), "budget + closing turn only");
    assert_ne!(report["status"], json!("ok"));
    assert_eq!(llm.post_count(), 7);
}

#[test]
fn closing_turn_verdict_gets_the_retry_on_a_parse_failure() {
    let dir = testdir("closing-plus-retry");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.extend(coder_turns());
    script.extend(tool_only_turns(3, "reviewer_look"));
    script.push(chat_text(PROSE_APPROVAL));
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    // Precedence, pinned: the closing turn fires because the reviewer
    // never spoke; its reply fails the parse; the retry stacks on top
    // — exactly one of each, and the retry's verdict counts.
    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["closing_turn"], json!(true));
    assert_eq!(reviewer["verdict_retry"], json!(true));
    assert_eq!(reviewer["approved"], json!(true));
    assert_eq!(
        reviewer["turns"],
        json!(5),
        "3 tool turns + closing turn + retry turn"
    );
    assert_eq!(report["status"], json!("ok"));
    assert_eq!(llm.post_count(), 8);
    let payloads = llm.payloads();
    assert!(
        payload_is_tools_off(&payloads[6]),
        "closing turn offers no tools"
    );
    let retry = payloads.last().expect("retry payload");
    assert!(payload_is_tools_off(retry), "retry turn offers no tools");
    assert!(
        payload_last_message_text(retry).contains(PROSE_APPROVAL),
        "retry quotes the closing turn's raw reply"
    );
}
