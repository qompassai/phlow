//! NCM S1 (D-1050): best-of-k candidate generation + verifier selection.
//!
//! Drives the real `Runtime` planner → coder → verification → reviewer
//! loop with a scripted model transport, on a bounded, test-covered
//! task: fix `add` in `calc.py` so `test_calc.py` passes. Arm A is the
//! single-sample behaviour (`k = 1`, the default); Arm B is `k = 4`.
//! The script offers the same candidate stream to both arms: a wrong
//! fix, a syntax error, then a correct fix. Arm A ends on the first
//! candidate; Arm B must select the correct one by verification —
//! host checks first, reviewer second, never consensus.
//!
//! These are mechanism tests (deterministic, scripted). The decisive
//! real-model run over the frozen task set is a separate, ignored
//! test in `best_of_k_real.rs`.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use phlow_config::{FlowConfig, LoadOptions, load_config};
use phlow_editor::{EditorTransport, SCHEMAS_LUA, TransportError};
use phlow_llm::{LlmError, LlmTransport};
use phlow_runtime::Runtime;
use serde_json::{Map, Value, json};

// ---------------------------------------------------------------------------
// Test infrastructure (mirrors tests/runtime.rs)
// ---------------------------------------------------------------------------

fn testdir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "phlow-best-of-k-test-{}-{}-{}",
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
// The frozen task: fix `add` in calc.py
// ---------------------------------------------------------------------------

const STARTER: &str = "def add(a, b):\n    return a - b\n";
const TESTS: &str = "\
import sys

from calc import add

failures = []
for a, b, want in [(2, 3, 5), (0, 0, 0), (-1, 1, 0), (10, -4, 6)]:
    got = add(a, b)
    if got != want:
        failures.append(f\"add({a}, {b}) = {got}, want {want}\")
if failures:
    print(\"FAIL: \" + \"; \".join(failures))
    sys.exit(1)
print(\"PASS\")
";
const WRONG_FIX: &str = "def add(a, b):\n    return a * b\n";
const SYNTAX_FIX: &str = "def add(a, b)\n    return a + b\n";
const CORRECT_FIX: &str = "def add(a, b):\n    return a + b\n";

const TASK_TOML: &str = "\
[agent]
max_cycles = 1

[checks.tests]
cmd = [\"python3\", \"test_calc.py\"]
kind = \"test\"
required = true
filetypes = [\"python\"]

[checks.lint]
cmd = [\"true\"]
kind = \"lint\"
required = true
filetypes = [\"python\"]
";

fn seed_workspace(dir: &std::path::Path) {
    std::fs::write(dir.join("calc.py"), STARTER).expect("starter");
    std::fs::write(dir.join("test_calc.py"), TESTS).expect("tests");
}

/// One candidate's script: coder writes `calc.py` (plus an extra file
/// when `extra` is set), reports done, reviewer returns `verdict`.
fn candidate_script(id_prefix: &str, content: &str, extra: bool, verdict: &str) -> Vec<Value> {
    let mut calls = vec![tool_call(
        &format!("{id_prefix}_write"),
        "file_write",
        json!({"path": "calc.py", "content": content}),
    )];
    if extra {
        calls.push(tool_call(
            &format!("{id_prefix}_extra"),
            "file_write",
            json!({"path": "scratch_notes.txt", "content": "candidate scratch\n"}),
        ));
    }
    vec![
        chat_tools(Value::Array(calls)),
        chat_text("Done. Updated calc.py."),
        chat_text(verdict),
    ]
}

/// The candidate stream both arms draw from: wrong, broken, correct.
fn arm_script() -> Vec<Value> {
    let mut script = vec![chat_text("Plan: fix add in calc.py so the tests pass.")];
    script.extend(candidate_script(
        "c1",
        WRONG_FIX,
        true,
        r#"{"approved": false, "summary": "Tests fail.", "issues": ["add is wrong"]}"#,
    ));
    script.extend(candidate_script(
        "c2",
        SYNTAX_FIX,
        false,
        r#"{"approved": false, "summary": "Syntax error.", "issues": ["missing colon"]}"#,
    ));
    script.extend(candidate_script(
        "c3",
        CORRECT_FIX,
        false,
        r#"{"approved": true, "summary": "Tests pass.", "issues": []}"#,
    ));
    script
}

// ---------------------------------------------------------------------------
// Arms
// ---------------------------------------------------------------------------

#[test]
fn arm_a_single_sample_stops_at_first_candidate() {
    let dir = testdir("arm-a");
    seed_workspace(&dir);
    let llm = ScriptLlm::new(arm_script());
    let mut rt = runtime(&dir, TASK_TOML, llm);
    assert_eq!(rt.best_of_k(), 1, "default is single-sample");

    let started = Instant::now();
    let report = rt.run("fix add in calc.py so test_calc.py passes");
    let wall = started.elapsed();
    println!("arm A wall-clock (scripted): {wall:?}");

    assert!(
        report.get("best_of_k").is_none(),
        "single-sample runs publish no best_of_k block"
    );
    assert_ne!(report["status"], json!("ok"), "first candidate is wrong");
    assert_eq!(
        std::fs::read_to_string(dir.join("calc.py")).unwrap(),
        WRONG_FIX,
        "arm A keeps the first candidate's output"
    );
}

#[test]
fn arm_b_best_of_four_selects_the_verified_candidate() {
    let dir = testdir("arm-b");
    seed_workspace(&dir);
    let llm = ScriptLlm::new(arm_script());
    let mut rt = runtime(&dir, TASK_TOML, llm.clone());
    rt.set_best_of_k(4);
    assert_eq!(rt.best_of_k(), 4);

    let started = Instant::now();
    let report = rt.run("fix add in calc.py so test_calc.py passes");
    let wall = started.elapsed();
    println!(
        "arm B wall-clock (scripted): {wall:?}, model posts: {}",
        llm.post_count()
    );

    assert_eq!(report["status"], json!("ok"));
    assert_eq!(report["verified"], json!(true));
    assert_eq!(
        report["cycles"],
        json!(1),
        "winner counts as the first cycle"
    );

    let best = &report["best_of_k"];
    assert_eq!(best["k"], json!(4));
    assert_eq!(best["generated"], json!(3), "early exit at first approval");
    assert_eq!(best["selected"], json!(2), "third candidate wins");
    let candidates = best["candidates"].as_array().expect("candidate records");
    assert_eq!(candidates.len(), 3);
    assert_eq!(candidates[0]["verified"], json!(false));
    assert_eq!(candidates[1]["verified"], json!(false));
    assert_eq!(candidates[2]["verified"], json!(true));
    assert_eq!(candidates[2]["reviewer_approved"], json!(true));

    assert_eq!(
        std::fs::read_to_string(dir.join("calc.py")).unwrap(),
        CORRECT_FIX,
        "winner's bytes are what remain"
    );
    assert!(
        !dir.join("scratch_notes.txt").exists(),
        "candidate 1's extra file is restored away, never kept"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("test_calc.py")).unwrap(),
        TESTS,
        "the test file itself is untouched"
    );
}

#[test]
fn best_of_k_winner_is_verification_first_not_reviewer_first() {
    // Candidate 1: reviewer loves it, tests fail. Candidate 2: tests
    // pass, reviewer rejects. Verification must outrank the reviewer,
    // so candidate 2 is selected even though the run cannot approve.
    let dir = testdir("rank-order");
    seed_workspace(&dir);
    let mut script = vec![chat_text("Plan: fix add in calc.py so the tests pass.")];
    script.extend(candidate_script(
        "c1",
        WRONG_FIX,
        false,
        r#"{"approved": true, "summary": "Looks right.", "issues": []}"#,
    ));
    script.extend(candidate_script(
        "c2",
        CORRECT_FIX,
        false,
        r#"{"approved": false, "summary": "Style concerns.", "issues": ["style"]}"#,
    ));
    let llm = ScriptLlm::new(script);
    let mut rt = runtime(&dir, TASK_TOML, llm);
    rt.set_best_of_k(2);
    let report = rt.run("fix add in calc.py so test_calc.py passes");

    assert_eq!(report["best_of_k"]["selected"], json!(1));
    assert_eq!(
        std::fs::read_to_string(dir.join("calc.py")).unwrap(),
        CORRECT_FIX,
    );
    assert_ne!(
        report["status"],
        json!("ok"),
        "reviewer rejection still gates the run outcome"
    );
}

#[test]
#[should_panic(expected = "best-of-k count must be in 1..=BEST_OF_K_MAX")]
fn best_of_k_count_is_bounded() {
    let dir = testdir("bounded");
    seed_workspace(&dir);
    let mut rt = runtime(&dir, TASK_TOML, ScriptLlm::new(vec![]));
    rt.set_best_of_k(0);
}
