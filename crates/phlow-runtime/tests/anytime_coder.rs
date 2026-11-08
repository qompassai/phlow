//! D-1075 rung 2: the anytime coder contract.
//!
//! Drives the real `Runtime` planner → coder → verification → reviewer
//! loop with a scripted model transport, on the same bounded task as
//! `closing_turn.rs` (fix `add` in `calc.py`). The defect under repair
//! (rung-1 anatomy): in 19 of 23 budget-death runs the coder's patch
//! already passed host verification when the budget died — the only
//! missing artifact was the closing text turn — and all 249 in-loop
//! `flow_check` events returned aggregate `unverified`, so the model
//! never saw a signal that entitled it to stop.
//!
//! The contract under test: (a) the coder scores every `flow_check`
//! result and retains a snapshot of the best scored workspace state,
//! restoring it on budget exhaustion when it strictly outranks the
//! final state; (b) the D-1060 closing turn extends to the coder,
//! after any restore; (c) `flow_check` events record the called check
//! name and the per-check statuses. Validation tests pin salvage;
//! adversarial tests pin that ties keep the earliest snapshot, that
//! closing-turn prose cannot fabricate verification, that an
//! unconfigured check name scores by its aggregate, and that the
//! D-1062 watchdog still never fires for the coder.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use phlow_config::{FlowConfig, LoadOptions, load_config};
use phlow_editor::{EditorTransport, SCHEMAS_LUA, TransportError};
use phlow_llm::{LlmError, LlmTransport};
use phlow_runtime::{ContractVariant, Runtime};
use serde_json::{Map, Value, json};

// ---------------------------------------------------------------------------
// Test infrastructure (mirrors tests/closing_turn.rs)
// ---------------------------------------------------------------------------

fn testdir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "phlow-anytime-coder-test-{}-{}-{}",
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
const CORRECT_FIX_V2: &str = "def add(a, b):\n    return b + a\n";
// Deliberately a different byte length from CORRECT_FIX: Python's
// .pyc cache validates on (mtime seconds, size), so a same-length
// rewrite inside one second would be served stale bytecode and the
// second check would test the first version.
const BROKEN_FIX: &str = "def add(a, b):\n    return a * b + 100\n";

const CHECKS_TOML: &str = "\
[checks.tests]\ncmd = [\"python3\", \"test_calc.py\"]\nkind = \"test\"\nrequired = true\nfiletypes = [\"python\"]\n\n[checks.lint]\ncmd = [\"true\"]\nkind = \"lint\"\nrequired = true\nfiletypes = [\"python\"]\n";

const PLAN_TEXT: &str = "Plan: fix add in calc.py so the tests pass.";
const APPROVE_VERDICT: &str = r#"{"approved": true, "summary": "Tests pass.", "issues": []}"#;
const TASK: &str = "fix add in calc.py so test_calc.py passes";

fn toml_with_iterations(iterations: u32) -> String {
    format!("[agent]\nmax_iterations = {iterations}\nmax_cycles = 1\n\n{CHECKS_TOML}")
}

fn seed_workspace(dir: &std::path::Path) {
    std::fs::write(dir.join("calc.py"), STARTER).expect("starter");
    std::fs::write(dir.join("test_calc.py"), TESTS).expect("tests");
}

fn write_turn(id: &str, content: &str) -> Value {
    chat_tools(json!([tool_call(
        id,
        "file_write",
        json!({"path": "calc.py", "content": content}),
    )]))
}

fn check_turn(id: &str, arguments: Value) -> Value {
    chat_tools(json!([tool_call(id, "flow_check", arguments)]))
}

fn read_turn(id: &str) -> Value {
    chat_tools(json!([tool_call(
        id,
        "file_read",
        json!({"path": "calc.py"}),
    )]))
}

fn role_state<'a>(report: &'a Value, role: &str) -> &'a Value {
    report["roles"]
        .as_array()
        .expect("roles array")
        .iter()
        .find(|entry| entry["role"] == json!(role))
        .unwrap_or_else(|| panic!("role {role} ran"))
}

/// The coder's `flow_check` events in this report, in order.
fn coder_check_events(report: &Value) -> Vec<&Value> {
    report["events"]
        .as_array()
        .expect("events array")
        .iter()
        .filter(|event| event["role"] == json!("coder") && event["tool"] == json!("flow_check"))
        .collect()
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

fn workspace_calc(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("calc.py")).expect("calc.py")
}

// ---------------------------------------------------------------------------
// Validation: snapshots, restore, closing turn, instrumentation
// ---------------------------------------------------------------------------

#[test]
fn snapshot_and_closing_turn_salvage_a_verified_cycle() {
    // The T1 shape: the patch verifies in-loop, the coder keeps
    // checking until the budget dies, and only the summary is missing.
    let dir = testdir("salvage");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.push(write_turn("w1", CORRECT_FIX));
    script.push(check_turn("c1", json!({})));
    script.push(check_turn("c2", json!({})));
    script.push(chat_text("Wrote the fix; the checks pass."));
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, &toml_with_iterations(3), llm.clone());
    let report = rt.run(TASK);

    // The salvaged cycle reaches approval through the ordinary path.
    assert_eq!(report["status"], json!("ok"));
    assert_eq!(report["verified"], json!(true));
    let coder = role_state(&report, "coder");
    assert_eq!(coder["status"], json!("ok"));
    assert_eq!(coder["closing_turn"], json!(true));
    assert_eq!(coder["summary"], json!("Wrote the fix; the checks pass."));
    assert_eq!(coder["turns"], json!(4), "3 tool turns + 1 closing turn");
    // The snapshot was retained at the first check and never restored:
    // the final state is that same scored state.
    let snapshot = &coder["anytime_snapshot"];
    assert_eq!(snapshot["turn"], json!(2));
    assert_eq!(snapshot["aggregate"], json!("ok"));
    assert_eq!(snapshot["aggregate_rank"], json!(2));
    assert_eq!(snapshot["ok_checks"], json!(2));
    assert_eq!(snapshot["restored"], json!(false));
    assert_eq!(llm.post_count(), 6);

    // Instrumentation: the unnamed call records check_name = null and
    // the per-check (name, status) pairs from the result.
    let events = coder_check_events(&report);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["check_name"], Value::Null);
    let pairs = events[0]["check_results"].as_array().expect("pairs");
    assert_eq!(pairs.len(), 2);
    assert!(
        pairs.iter().all(|pair| pair["status"] == json!("ok")
            && (pair["name"] == json!("tests") || pair["name"] == json!("lint"))),
        "per-check pairs name both configured checks, both ok: {pairs:?}"
    );
}

#[test]
fn exhaustion_restores_the_best_snapshot_over_a_worse_final_state() {
    let dir = testdir("restore");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.push(write_turn("w1", BROKEN_FIX));
    script.push(check_turn("c1", json!({}))); // failed: score (0, 1)
    script.push(write_turn("w2", CORRECT_FIX));
    script.push(check_turn("c2", json!({}))); // ok: score (2, 2), displaces
    script.push(write_turn("w3", BROKEN_FIX)); // unscored trailing write
    script.push(chat_text("Summary of the patch as it stands."));
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, &toml_with_iterations(5), llm.clone());
    let report = rt.run(TASK);

    // The workspace the cycle verified is the restored snapshot, not
    // the trailing broken write.
    assert_eq!(workspace_calc(&dir), CORRECT_FIX);
    assert_eq!(report["status"], json!("ok"));
    assert_eq!(report["verified"], json!(true));
    let coder = role_state(&report, "coder");
    let snapshot = &coder["anytime_snapshot"];
    assert_eq!(snapshot["turn"], json!(4), "the better check displaced");
    assert_eq!(snapshot["aggregate"], json!("ok"));
    assert_eq!(snapshot["restored"], json!(true));
    // The closing turn ran after the restore (its summary is the
    // coder's artifact) and the run took exactly the scripted calls.
    assert_eq!(coder["closing_turn"], json!(true));
    assert_eq!(coder["status"], json!("ok"));
    assert_eq!(llm.post_count(), 8);
}

#[test]
fn coder_closing_turn_fires_once_with_tools_off() {
    let dir = testdir("closing-once");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.push(read_turn("r1"));
    script.push(read_turn("r2"));
    script.push(read_turn("r3"));
    script.push(chat_text("I only read calc.py; no changes made."));
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, &toml_with_iterations(3), llm.clone());
    let report = rt.run(TASK);

    let coder = role_state(&report, "coder");
    assert_eq!(coder["closing_turn"], json!(true));
    assert_eq!(coder["status"], json!("ok"));
    assert_eq!(
        coder["summary"],
        json!("I only read calc.py; no changes made.")
    );
    assert_eq!(coder["turns"], json!(4), "budget + exactly one closing");
    // No flow_check ran, so no snapshot was ever retained.
    assert_eq!(coder.get("anytime_snapshot"), None);
    assert_eq!(llm.post_count(), 6, "no call beyond the closing turn");
    let payloads = llm.payloads();
    let closing = &payloads[4];
    assert!(
        payload_is_tools_off(closing),
        "closing turn offers no tools"
    );
    assert!(
        payload_last_message_text(closing).contains("summary of the patch as it stands"),
        "closing payload demands the patch summary"
    );
}

#[test]
fn coder_contract_variant_slot_round_trips() {
    // Rung 3 seam: the coder carries a variant slot exactly like the
    // planner/reviewer; canonical is the default and the only shipped
    // variant in this rung, and a canonical coder run records no
    // provenance field.
    let dir = testdir("variant-slot");
    seed_workspace(&dir);
    let mut rt = runtime(&dir, &toml_with_iterations(3), ScriptLlm::new(vec![]));
    assert_eq!(rt.coder_contract_variant(), ContractVariant::Canonical);
    rt.set_coder_contract_variant(ContractVariant::Canonical);
    assert_eq!(rt.coder_contract_variant(), ContractVariant::Canonical);
}

// ---------------------------------------------------------------------------
// Adversarial: salvage cannot be fabricated or misapplied
// ---------------------------------------------------------------------------

#[test]
fn score_tie_keeps_the_earliest_snapshot_and_the_final_state() {
    let dir = testdir("tie");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.push(write_turn("w1", CORRECT_FIX));
    script.push(check_turn("c1", json!({}))); // ok (2, 2): snapshot at turn 2
    script.push(write_turn("w2", CORRECT_FIX_V2));
    script.push(check_turn("c2", json!({}))); // ok (2, 2): tie, no displacement
    script.push(read_turn("r1"));
    script.push(chat_text("Rewrote add commutatively; checks pass."));
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, &toml_with_iterations(5), llm.clone());
    let report = rt.run(TASK);

    // The final state scores the same as the snapshot, so it is NOT
    // strictly outranked and must not be rewound to the earlier bytes.
    assert_eq!(workspace_calc(&dir), CORRECT_FIX_V2);
    let coder = role_state(&report, "coder");
    let snapshot = &coder["anytime_snapshot"];
    assert_eq!(snapshot["turn"], json!(2), "the earliest snapshot is kept");
    assert_eq!(snapshot["restored"], json!(false));
    assert_eq!(report["status"], json!("ok"));
}

#[test]
fn closing_turn_prose_cannot_fabricate_host_verification() {
    let dir = testdir("no-fabrication");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.push(write_turn("w1", BROKEN_FIX));
    script.push(check_turn("c1", json!({})));
    script.push(read_turn("r1"));
    // The closing summary claims success the checks never reported.
    script.push(chat_text("All checks pass; ship it."));
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, &toml_with_iterations(3), llm.clone());
    let report = rt.run(TASK);

    let coder = role_state(&report, "coder");
    assert_eq!(coder["status"], json!("ok"), "the summary is recorded");
    assert_eq!(coder["summary"], json!("All checks pass; ship it."));
    // Verification still comes only from the host checks runner.
    assert_eq!(report["verified"], json!(false));
    assert_eq!(report["status"], json!("failed"));
    assert_eq!(report["verification"]["status"], json!("failed"));
}

#[test]
fn unconfigured_check_name_scores_by_its_aggregate() {
    let dir = testdir("unknown-name");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.push(write_turn("w1", CORRECT_FIX));
    script.push(check_turn("c1", json!({"name": "nope"})));
    script.push(read_turn("r1"));
    script.push(chat_text("Wrote the fix; my named check was unavailable."));
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, &toml_with_iterations(3), llm.clone());
    let report = rt.run(TASK);

    // No panic; the snapshot scores exactly what the aggregate said.
    let coder = role_state(&report, "coder");
    let snapshot = &coder["anytime_snapshot"];
    assert_eq!(snapshot["aggregate"], json!("unverified"));
    assert_eq!(snapshot["aggregate_rank"], json!(1));
    assert_eq!(snapshot["ok_checks"], json!(0));
    assert_eq!(snapshot["restored"], json!(false));
    // The event names the called check and its unavailable status.
    let events = coder_check_events(&report);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["check_name"], json!("nope"));
    assert_eq!(
        events[0]["check_results"],
        json!([{"name": "nope", "status": "unavailable"}])
    );
    // Host verification (unnamed) still passes on the written fix.
    assert_eq!(report["status"], json!("ok"));
    assert_eq!(report["verified"], json!(true));
}

#[test]
fn exhaustion_with_zero_writes_records_no_snapshot() {
    let dir = testdir("zero-writes");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.push(read_turn("r1"));
    script.push(read_turn("r2"));
    script.push(read_turn("r3"));
    script.push(chat_text("No changes were made."));
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, &toml_with_iterations(3), llm.clone());
    let report = rt.run(TASK);

    // No write and no check: the snapshot mechanism never engages —
    // no snapshot fields, and the workspace bytes and the report
    // ledger stay exactly as seeded.
    assert_eq!(workspace_calc(&dir), STARTER);
    assert_eq!(report["changed_files"], json!([]));
    let coder = role_state(&report, "coder");
    assert_eq!(coder.get("anytime_snapshot"), None);
    // Planner/reviewer states carry no anytime fields, ever.
    assert_eq!(role_state(&report, "planner").get("anytime_snapshot"), None);
    assert_eq!(
        role_state(&report, "reviewer").get("anytime_snapshot"),
        None
    );
}

#[test]
fn watchdog_never_fires_for_the_coder() {
    // Six tool calls cross the D-1062 threshold of five; the watchdog
    // is planner/reviewer-scoped in code and must stay silent here —
    // the coder's rescue is the anytime snapshot + closing turn.
    let dir = testdir("no-watchdog");
    seed_workspace(&dir);
    let mut script = vec![chat_text(PLAN_TEXT)];
    script.push(write_turn("w1", CORRECT_FIX));
    script.push(check_turn("c1", json!({})));
    script.push(write_turn("w2", CORRECT_FIX));
    script.push(check_turn("c2", json!({})));
    script.push(read_turn("r1"));
    script.push(read_turn("r2"));
    script.push(chat_text("Done; checks pass."));
    script.push(chat_text(APPROVE_VERDICT));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, &toml_with_iterations(6), llm.clone());
    let report = rt.run(TASK);

    let coder = role_state(&report, "coder");
    assert_eq!(coder.get("watchdog_fired"), None);
    assert_eq!(coder.get("watchdog_at_tool_calls"), None);
    assert_eq!(coder["closing_turn"], json!(true));
    assert_eq!(coder["status"], json!("ok"));
    assert_eq!(report["status"], json!("ok"));
    assert_eq!(report["verified"], json!(true));
}
