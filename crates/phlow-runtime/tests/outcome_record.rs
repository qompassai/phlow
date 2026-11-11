//! PH-1 — the outcome-record suite test: every run leaves exactly
//! one append-only row, written by the runtime.
//!
//! Drives the real [`Runtime`] with scripted transports and an
//! outcome store in a temp directory, then reads the store back
//! through `phlow-self-improve`'s own API. The killed-run case (a run
//! closed as `abandoned` by a reaper) is covered at the store level
//! in `phlow-self-improve::outcome`'s tests; here the runtime-level
//! shapes are a completed run and a backend-failure run — both must
//! leave exactly one row, and a run with no row is a suite failure by
//! construction of these assertions.

use std::path::PathBuf;
use std::time::Duration;

use phlow_config::{FlowConfig, LoadOptions, load_config};
use phlow_editor::{EditorTransport, SCHEMAS_LUA, TransportError};
use phlow_llm::{LlmError, LlmTransport};
use phlow_runtime::Runtime;
use phlow_self_improve::OutcomeStore;
use serde_json::{Map, Value, json};

fn testdir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "phlow-outcome-test-{}-{}-{}",
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

/// A transport that answers every chat with the same text reply.
struct TextLlm;

impl LlmTransport for TextLlm {
    fn post_chat(
        &mut self,
        _base_url: &str,
        _payload: &Map<String, Value>,
        _timeout: Duration,
    ) -> Result<Value, LlmError> {
        Ok(chat_text("Done."))
    }

    fn get_tags(&mut self, _base_url: &str, _timeout: Duration) -> Result<Value, LlmError> {
        Ok(json!({"models": []}))
    }

    fn close(&mut self) {}
}

/// A transport whose backend is down: every chat fails.
struct FailingLlm;

impl LlmTransport for FailingLlm {
    fn post_chat(
        &mut self,
        _base_url: &str,
        _payload: &Map<String, Value>,
        _timeout: Duration,
    ) -> Result<Value, LlmError> {
        Err(LlmError::Transport("connection refused".to_owned()))
    }

    fn get_tags(&mut self, _base_url: &str, _timeout: Duration) -> Result<Value, LlmError> {
        Err(LlmError::Transport("connection refused".to_owned()))
    }

    fn close(&mut self) {}
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

#[test]
fn completed_run_leaves_exactly_one_outcome_row() {
    let dir = testdir("completed");
    let db = dir.join("feedback.db");
    let config = load_test_config(&dir, "", true);
    let mut runtime = Runtime::new(config, TextLlm, FakeEditor::new(), None).expect("runtime");
    runtime.set_outcome_db(db.clone());

    let report = runtime.run("do things");
    let run_id = report["outcome_run_id"]
        .as_str()
        .expect("report names its outcome run id")
        .to_owned();
    assert!(report.get("outcome_error").is_none());

    let store = OutcomeStore::open(&db).expect("store opens");
    assert_eq!(store.count_for_run(&run_id).unwrap(), 1);
    let row = store.get(&run_id).unwrap().expect("row exists");
    assert_eq!(row.outcome.task_class, "flow_run");
    assert_eq!(row.outcome.status, report["status"].as_str().unwrap());
    assert_eq!(row.outcome.cycles, report["cycles"].as_u64().unwrap());
    assert_eq!(
        row.outcome.tool_calls,
        report["tool_calls"].as_u64().unwrap()
    );
    // Lane labels are present from birth, fail-closed by default.
    assert_eq!(row.outcome.origin_lane, "phlow");
    assert_eq!(row.outcome.content_label, "nsfw");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failed_run_still_leaves_exactly_one_outcome_row() {
    let dir = testdir("failed");
    let db = dir.join("feedback.db");
    let config = load_test_config(&dir, "", true);
    let mut runtime = Runtime::new(config, FailingLlm, FakeEditor::new(), None).expect("runtime");
    runtime.set_outcome_db(db.clone());

    let report = runtime.run("do things");
    assert_eq!(report["status"], json!("error"));
    let run_id = report["outcome_run_id"]
        .as_str()
        .expect("even an errored run names its outcome run id")
        .to_owned();

    let store = OutcomeStore::open(&db).expect("store opens");
    assert_eq!(store.count_for_run(&run_id).unwrap(), 1);
    let row = store.get(&run_id).unwrap().expect("row exists");
    assert_eq!(row.outcome.status, "error");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_runs_leave_two_distinct_rows() {
    let dir = testdir("tworuns");
    let db = dir.join("feedback.db");
    let config = load_test_config(&dir, "", true);
    let mut runtime = Runtime::new(config, TextLlm, FakeEditor::new(), None).expect("runtime");
    runtime.set_outcome_db(db.clone());

    let first = runtime.run("first task");
    let second = runtime.run("second task");
    let first_id = first["outcome_run_id"].as_str().unwrap().to_owned();
    let second_id = second["outcome_run_id"].as_str().unwrap().to_owned();
    assert_ne!(first_id, second_id);

    let store = OutcomeStore::open(&db).expect("store opens");
    assert_eq!(store.count_for_run(&first_id).unwrap(), 1);
    assert_eq!(store.count_for_run(&second_id).unwrap(), 1);
    assert_eq!(store.list(10).unwrap().len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unconfigured_runtime_records_nothing_and_says_so_by_absence() {
    // Library embedders that never set an outcome store get the
    // pre-PH-1 behaviour, byte-for-byte: no outcome keys on the
    // report. Production wiring (phlow-cli) always sets the store.
    let dir = testdir("unconfigured");
    let config = load_test_config(&dir, "", true);
    let mut runtime = Runtime::new(config, TextLlm, FakeEditor::new(), None).expect("runtime");
    let report = runtime.run("do things");
    assert!(report.get("outcome_run_id").is_none());
    assert!(report.get("outcome_error").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
