//! D-1062: the runtime watchdog for the planner and reviewer stages.
//!
//! The rung-1 closing turn (D-1060/D-1061) rescues a role that spent its
//! whole iteration budget on tool calls, one turn past the budget. The
//! watchdog generalises it inside the loop: after
//! `WATCHDOG_TOOL_CALLS` (5) tool calls in one role run with no text
//! turn yet, the runtime appends the closing instruction and closes tool
//! access for the rest of the run — later turns are called with an empty
//! tool list, attempted calls are ignored and never dispatched, and any
//! text reply goes through the normal finish path.
//!
//! Tests run at the same scripted seam as `closing_turn.rs`, under the
//! S2 live posture for the roles (`max_iterations = 6`), so post counts
//! and turn counts are exact. The counterfactual block is the falsifier
//! measurement by construction: a script that answers in text after M
//! tool calls would have concluded correctly with no watchdog at all, so
//! for M > N every firing is a would-have-concluded-anyway fire, counted
//! here by construction; for M < N the run must be identical to
//! watchdog-off (no marker, no tools-off payload, no extra call).

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
        "phlow-watchdog-test-{}-{}-{}",
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
    rescuable: bool,
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
                rescuable: false,
                payloads: Vec::new(),
            })),
        }
    }

    /// A script whose fallback models a tool-looping role faithfully:
    /// while tools are offered it calls a tool again, and only a
    /// tools-off payload makes it answer in text. Without the watchdog
    /// such a role reaches the rung-1 closing turn at turns =
    /// max_iterations + 1; the watchdog must rescue it inside the budget.
    fn new_rescuable(script: Vec<Value>) -> Self {
        ScriptLlm {
            shared: std::sync::Arc::new(std::sync::Mutex::new(ScriptState {
                script: script.into(),
                fallback: chat_text("unused"),
                rescuable: true,
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
        if let Some(next) = state.script.pop_front() {
            return Ok(next);
        }
        if state.rescuable {
            if payload.contains_key("tools") {
                return Ok(chat_tools(json!([tool_call(
                    "fallback_look",
                    "file_read",
                    json!({"path": "calc.py"}),
                )])));
            }
            return Ok(chat_text(APPROVE_VERDICT));
        }
        Ok(state.fallback.clone())
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

/// `count` consecutive tool-only turns reading `calc.py`: a role that
/// inspects and never speaks.
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

/// The S2 live posture for a role run: six iterations, one cycle so post
/// counts stay exact. The watchdog threshold (5 tool calls) fires after
/// the fifth tool turn and leaves the sixth turn as the forced text turn.
const TASK_TOML: &str = "\
[agent]\nmax_iterations = 6\nmax_cycles = 1\n\n[checks.tests]\ncmd = [\"python3\", \"test_calc.py\"]\nkind = \"test\"\nrequired = true\nfiletypes = [\"python\"]\n\n[checks.lint]\ncmd = [\"true\"]\nkind = \"lint\"\nrequired = true\nfiletypes = [\"python\"]\n";

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

/// How many payloads end with the injected closing instruction. The
/// watchdog injects it once; the rung-1 closing turn injects it once
/// more when it stacks, and no other message carries its wording.
fn instruction_payloads(payloads: &[Map<String, Value>]) -> usize {
    payloads
        .iter()
        .filter(|payload| {
            payload_last_message_text(payload).contains("Your iteration budget is exhausted")
        })
        .count()
}

// ---------------------------------------------------------------------------
// Validation: the watchdog rescues a tool-looping role inside the budget
// ---------------------------------------------------------------------------

#[test]
fn reviewer_watchdog_rescues_before_budget_exhaustion() {
    let dir = testdir("reviewer-rescue");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.extend(coder_turns());
    script.extend(tool_only_turns(5, "reviewer_look"));
    // The fallback loops on tools forever and answers only tools-off:
    // without the watchdog this run ends at the closing turn, turns = 7.
    let llm = ScriptLlm::new_rescuable(script);
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    assert_eq!(report["status"], json!("ok"));
    assert_eq!(report["verified"], json!(true));
    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["watchdog_fired"], json!(true));
    assert_eq!(reviewer["watchdog_at_tool_calls"], json!(5));
    assert_eq!(reviewer.get("closing_turn"), None, "no closing turn needed");
    assert_eq!(reviewer["status"], json!("ok"));
    assert_eq!(reviewer["approved"], json!(true));
    // Rescued inside the budget: 5 tool turns + 1 forced text turn,
    // strictly below max_iterations + 1 (the closing-turn count).
    assert_eq!(reviewer["turns"], json!(6));
    assert_eq!(role_state(&report, "planner").get("watchdog_fired"), None);
    assert_eq!(role_state(&report, "coder").get("watchdog_fired"), None);

    // 1 planner + 2 coder + 6 reviewer calls, exactly.
    assert_eq!(llm.post_count(), 9);
    assert_eq!(report["model_calls"], json!(9));
    // Dispatched tool calls: 1 coder write + 5 reviewer reads.
    assert_eq!(report["tool_calls"], json!(6));
    let payloads = llm.payloads();
    assert!(
        !payload_is_tools_off(&payloads[7]),
        "the fifth tool turn still offers tools"
    );
    let forced = payloads.last().expect("forced payload");
    assert!(payload_is_tools_off(forced), "forced turn offers no tools");
    assert!(
        payload_last_message_text(forced).contains("exactly the verdict JSON"),
        "forced payload demands the verdict"
    );
    assert_eq!(instruction_payloads(&payloads), 1, "watchdog injects once");
}

#[test]
fn planner_watchdog_rescues_and_the_run_proceeds() {
    let dir = testdir("planner-rescue");
    seed_workspace(&dir);
    let mut script = tool_only_turns(5, "planner_look");
    script.push(chat_text(PLAN_TEXT));
    script.extend(coder_turns());
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    let planner = role_state(&report, "planner");
    assert_eq!(planner["watchdog_fired"], json!(true));
    assert_eq!(planner["watchdog_at_tool_calls"], json!(5));
    assert_eq!(planner.get("closing_turn"), None);
    assert_eq!(planner["status"], json!("ok"));
    assert_eq!(planner["summary"], json!(PLAN_TEXT));
    assert_eq!(planner["turns"], json!(6), "5 tool turns + 1 forced turn");
    // The run proceeded past planning to a verified completion.
    assert_eq!(report["status"], json!("ok"));
    assert_eq!(report["verified"], json!(true));
    assert_eq!(llm.post_count(), 9);
    let payloads = llm.payloads();
    let forced = &payloads[5];
    assert!(payload_is_tools_off(forced), "forced turn offers no tools");
    assert!(
        payload_last_message_text(forced).contains("concrete plan as plain text"),
        "forced payload demands the plan"
    );
}

// ---------------------------------------------------------------------------
// Validation: the falsifier counterfactual — M tool calls, then text
// ---------------------------------------------------------------------------

/// Shared shape for the counterfactual runs: the reviewer makes `count`
/// tool-only turns and then answers with an approving verdict. With the
/// watchdog off this run concludes correctly at turn count + 1, so any
/// firing is a would-have-concluded-anyway fire by construction.
fn counterfactual_run(name: &str, count: usize) -> (Value, ScriptLlm) {
    let dir = testdir(name);
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.extend(coder_turns());
    script.extend(tool_only_turns(count, "reviewer_look"));
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    (rt.run(TASK), llm)
}

#[test]
fn counterfactual_below_threshold_is_watchdog_silent() {
    // M = 3 and M = 4, both below N = 5: no firing, and the runs are
    // identical to watchdog-off — every payload offers tools, no marker,
    // no extra model call.
    for (count, turns, posts) in [(3usize, 4u64, 7usize), (4, 5, 8)] {
        let (report, llm) = counterfactual_run(&format!("below-{count}"), count);
        let reviewer = role_state(&report, "reviewer");
        assert_eq!(reviewer.get("watchdog_fired"), None, "M = {count}");
        assert_eq!(reviewer.get("closing_turn"), None, "M = {count}");
        assert_eq!(reviewer["approved"], json!(true), "M = {count}");
        assert_eq!(reviewer["turns"], json!(turns), "M = {count}");
        assert_eq!(llm.post_count(), posts, "M = {count}");
        assert!(
            llm.payloads().iter().all(|p| !payload_is_tools_off(p)),
            "M = {count}: no payload may be tools-off"
        );
    }
}

#[test]
fn counterfactual_at_threshold_fires_and_still_concludes() {
    // M = 5 = N: the watchdog fires after the fifth tool call, one turn
    // before this script would have spoken on its own — a
    // would-have-concluded-anyway fire by construction. The run must
    // still conclude correctly through the forced text path.
    let (report, llm) = counterfactual_run("at-threshold", 5);
    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["watchdog_fired"], json!(true));
    assert_eq!(reviewer["watchdog_at_tool_calls"], json!(5));
    assert_eq!(reviewer.get("closing_turn"), None);
    assert_eq!(reviewer["status"], json!("ok"));
    assert_eq!(reviewer["approved"], json!(true));
    assert_eq!(reviewer["summary"], json!("Tests pass."));
    assert_eq!(reviewer["turns"], json!(6));
    assert_eq!(report["status"], json!("ok"));
    assert_eq!(llm.post_count(), 9);
    let payloads = llm.payloads();
    assert!(payload_is_tools_off(payloads.last().expect("last payload")));
}

// ---------------------------------------------------------------------------
// Adversarial: scope, garbage, and the at-most-once invariant
// ---------------------------------------------------------------------------

#[test]
fn watchdog_never_fires_for_the_coder() {
    let dir = testdir("coder-scope");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    // The coder makes 5 tool calls across 5 turns — the planner/reviewer
    // threshold — then reports done in a sixth turn.
    script.extend(tool_only_turns(4, "coder_look"));
    script.extend(coder_turns());
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    assert_eq!(report["status"], json!("ok"));
    let coder = role_state(&report, "coder");
    assert_eq!(coder.get("watchdog_fired"), None, "coder has no artifact");
    assert_eq!(coder.get("watchdog_at_tool_calls"), None);
    assert_eq!(coder["turns"], json!(6));
    // Every coder payload offered tools, including its final text turn:
    // tool access was never closed for the coder.
    let payloads = llm.payloads();
    assert_eq!(payloads.len(), 8);
    for payload in &payloads[1..7] {
        assert!(!payload_is_tools_off(payload), "coder keeps its tools");
    }
}

#[test]
fn garbage_after_watchdog_firing_stays_unverified() {
    let dir = testdir("garbage-after-firing");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.extend(coder_turns());
    script.extend(tool_only_turns(5, "reviewer_look"));
    script.push(chat_text("I looked at everything and it seems fine to me."));
    // D-1076: the unparseable forced-turn reply earns exactly one
    // verdict retry; it is scripted garbage too. The fallback is an
    // approving verdict: any call beyond the one retry would flip the
    // reviewer to approved and fail here.
    script.push(chat_text("Still just my prose summary, no JSON."));
    let llm = ScriptLlm::with_fallback(script, chat_text(APPROVE_VERDICT));
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    let reviewer = role_state(&report, "reviewer");
    assert_eq!(reviewer["watchdog_fired"], json!(true));
    assert_eq!(reviewer["watchdog_at_tool_calls"], json!(5));
    assert_eq!(reviewer["verdict_retry"], json!(true));
    assert_eq!(reviewer["status"], json!("unverified"));
    assert_eq!(reviewer["approved"], json!(false), "never a false approval");
    assert_ne!(report["status"], json!("ok"));
    assert_ne!(report["verified"], json!(true));
    assert_eq!(
        llm.post_count(),
        10,
        "no call beyond the forced turn + retry"
    );
}

#[test]
fn watchdog_fires_at_most_once_and_closing_turn_stacks_truthfully() {
    let dir = testdir("fires-once-stacking");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.extend(coder_turns());
    script.extend(tool_only_turns(5, "reviewer_look"));
    // From here the model only ever attempts another tool call, with no
    // text — even tools-off, even on the closing turn. The fallback is
    // the same tool call, so any re-firing or dispatch would show.
    let llm = ScriptLlm::with_fallback(
        script,
        chat_tools(json!([tool_call(
            "one_more_look",
            "file_read",
            json!({"path": "calc.py"}),
        )])),
    );
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    let report = rt.run(TASK);

    let reviewer = role_state(&report, "reviewer");
    // Both markers, both truthful: the watchdog fired at 5 tool calls;
    // the role still never spoke, so the closing turn stacked at budget
    // end (turns = max_iterations + 1) and also failed to elicit text.
    assert_eq!(reviewer["watchdog_fired"], json!(true));
    assert_eq!(reviewer["watchdog_at_tool_calls"], json!(5));
    assert_eq!(reviewer["closing_turn"], json!(true));
    assert_eq!(reviewer["turns"], json!(7));
    assert_eq!(reviewer["status"], json!("unverified"));
    assert_eq!(reviewer["approved"], json!(false));
    assert_ne!(report["status"], json!("ok"));
    // 1 planner + 2 coder + 6 in-budget reviewer + 1 closing call.
    assert_eq!(llm.post_count(), 10);
    // The attempted calls after closure were never dispatched: the
    // global dispatched count is still 1 write + 5 reads.
    assert_eq!(report["tool_calls"], json!(6));
    let payloads = llm.payloads();
    // One watchdog injection + one closing-turn injection, no more.
    assert_eq!(instruction_payloads(&payloads), 2);
    assert!(
        payload_is_tools_off(&payloads[8]) && payload_is_tools_off(&payloads[9]),
        "turns after firing are tools-off"
    );
}
