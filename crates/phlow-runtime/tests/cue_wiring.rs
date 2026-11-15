//! PH-K2 — the live-wiring suite: cues fire in real runs.
//!
//! Drives the real [`Runtime`] with scripted transports and a cue
//! directory in a temp dir. A `run-this` cue planted before the
//! run must be consumed during it and answered with a `run-done`
//! in the same directory; an expired cue must be reaped and listed
//! on the report, never consumed. The refusal shapes (unapproved
//! check, malformed cue file) are pinned in the runtime safety
//! case (`tests/safety_case.rs`, safety doc §10).

use std::path::PathBuf;
use std::time::Duration;

use phlow_checks::cue::{LaneScope, list_cues, write_cue};
use phlow_config::{FlowConfig, LoadOptions, load_config};
use phlow_editor::{EditorTransport, SCHEMAS_LUA, TransportError};
use phlow_llm::{LlmError, LlmTransport};
use phlow_runtime::Runtime;
use serde_json::{Map, Value, json};

fn testdir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "phlow-cue-test-{}-{}-{}",
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

/// Two operator-approved checks: one that always passes, one that
/// leaves a sentinel file in the workspace when it runs.
const CHECKS_TOML: &str = r#"
[checks.ok]
cmd = ["python3", "-c", "pass"]
timeout = 10000
required = true
kind = "check"

[checks.sentinel]
cmd = ["python3", "-c", "open('sentinel.txt', 'w').write('ran')"]
timeout = 10000
required = true
kind = "check"
"#;

struct TextLlm;

impl LlmTransport for TextLlm {
    fn post_chat(
        &mut self,
        _base_url: &str,
        _payload: &Map<String, Value>,
        _timeout: Duration,
    ) -> Result<Value, LlmError> {
        Ok(json!({"choices": [{"message": {"role": "assistant", "content": "Done."}}]}))
    }

    fn get_tags(&mut self, _base_url: &str, _timeout: Duration) -> Result<Value, LlmError> {
        Ok(json!({"models": []}))
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

fn runtime_with_cues(
    dir: &std::path::Path,
    cue_dir: &std::path::Path,
) -> Runtime<TextLlm, FakeEditor> {
    let config = load_test_config(dir, CHECKS_TOML, true);
    let mut runtime = Runtime::new(config, TextLlm, FakeEditor::new(), None).expect("runtime");
    runtime.set_cue_dir(cue_dir.to_path_buf());
    runtime
}

#[test]
fn run_consumes_a_cue_and_answers_run_done() {
    let dir = testdir("consume");
    let cue_dir = dir.join("cue");
    let mut runtime = runtime_with_cues(&dir, &cue_dir);
    let payload = serde_json::to_vec(&json!({"checks": ["ok"]})).unwrap();
    let cue = write_cue(
        &cue_dir,
        "run-this",
        "seat",
        "phlow",
        60_000,
        &payload,
        "general",
        "sfw",
        phlow_checks::run_contract::now_unix_ms(),
    )
    .unwrap();

    let report = runtime.run("do things");
    let cues = report["cues"].as_array().expect("cues on the report");
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0]["cue_id"], json!(cue.id));
    assert_eq!(cues[0]["from"], json!("seat"));
    assert_eq!(cues[0]["verdict"], json!("ok"));

    // The answer sits in the same directory, addressed back to the
    // sender, carrying the original cue's labels from birth.
    let pending = list_cues(&cue_dir, LaneScope::Unrestricted).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].kind, "run-done");
    assert_eq!(pending[0].from, "phlow");
    assert_eq!(pending[0].to, "seat");
    assert_eq!(pending[0].origin_lane, "general");
    assert_eq!(pending[0].content_label, "sfw");
    assert_eq!(cues[0]["done_cue_id"], json!(pending[0].id));

    // A second run consumes nothing: consumption is exactly once.
    let second = runtime.run("do things again");
    assert!(second.get("cues").is_none(), "{second}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_reaps_an_expired_cue_and_lists_it_never_consumes_it() {
    let dir = testdir("expired");
    let cue_dir = dir.join("cue");
    let mut runtime = runtime_with_cues(&dir, &cue_dir);
    let payload = serde_json::to_vec(&json!({"checks": ["ok"]})).unwrap();
    // Written already old: created at 1 s, TTL 1 s — long past
    // expiry, and still well-formed (expiry after creation).
    let cue = write_cue(
        &cue_dir, "run-this", "seat", "phlow", 1_000, &payload, "general", "sfw", 1_000,
    )
    .unwrap();

    let report = runtime.run("do things");
    assert_eq!(report["cues_expired"], json!([cue.id]));
    assert!(report.get("cues").is_none(), "{report}");
    // The cue rests in expired/ and no run-done was written for it.
    assert!(
        cue_dir
            .join("expired")
            .join(format!("{}.json", cue.id))
            .exists()
    );
    assert!(
        list_cues(&cue_dir, LaneScope::Unrestricted)
            .unwrap()
            .is_empty()
    );
    let _ = std::fs::remove_dir_all(&dir);
}
