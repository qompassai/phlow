//! PH-9 — self-improvement safety case, `phlow-runtime` suite.
//! The runtime half of the case: nothing the *model* can reach may
//! mutate checks, budgets, prompts, skills, or history — including
//! the PH-1 outcome store, which the runtime writes itself. Full
//! case: `docs/SELF_IMPROVEMENT_SAFETY.md`.

use std::path::PathBuf;
use std::time::Duration;

use phlow_checks::cue::{LaneScope, claim, list_cues, write_cue};
use phlow_config::{FlowConfig, LoadOptions, load_config};
use phlow_editor::{EditorTransport, SCHEMAS_LUA, TransportError};
use phlow_llm::{LlmError, LlmTransport};
use phlow_runtime::Runtime;
use serde_json::{Map, Value, json};

fn testdir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "phlow-safety-runtime-test-{}-{}-{}",
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

fn tool_names(runtime: &mut Runtime<TextLlm, FakeEditor>, role: &str) -> Vec<String> {
    runtime
        .schemas(role)
        .iter()
        .filter_map(|schema| schema["function"]["name"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn model_tool_surface_has_no_self_mutation_tools() {
    let dir = testdir("surface");
    let config = load_test_config(&dir, "", true);
    let mut runtime = Runtime::new(config, TextLlm, FakeEditor::new(), None).expect("runtime");

    // Substrings that would name a self-improvement or self-mutation
    // capability if one were ever exposed to the model: prompt
    // evolution, budget editing, skill writes, config edits, history
    // access, and — since PH-1 — outcome-row writes.
    const FORBIDDEN: [&str; 9] = [
        "evolve",
        "budget",
        "skill",
        "prompt",
        "config",
        "history",
        "outcome",
        "shell_exec",
        "lsp_check",
    ];
    for role in ["planner", "coder", "reviewer"] {
        let names = tool_names(&mut runtime, role);
        assert!(!names.is_empty(), "{role} must see some tools");
        for name in &names {
            for needle in FORBIDDEN {
                assert!(
                    !name.contains(needle),
                    "{role} tool {name:?} exposes a self-mutation surface ({needle})"
                );
            }
        }
    }

    // The role gates themselves are part of the case: only the coder
    // may write or run checks; planner and reviewer stay read-only.
    let planner = tool_names(&mut runtime, "planner");
    let coder = tool_names(&mut runtime, "coder");
    let reviewer = tool_names(&mut runtime, "reviewer");
    assert!(planner.contains(&"file_read".to_owned()));
    assert!(!planner.contains(&"file_write".to_owned()));
    assert!(!reviewer.contains(&"file_write".to_owned()));
    assert!(coder.contains(&"file_write".to_owned()));
    assert!(coder.contains(&"flow_check".to_owned()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Checks config for the cue tests: one sentinel check whose side
/// effect (a file in the workspace) proves whether anything ran.
const CUE_CHECKS_TOML: &str = r#"
[checks.sentinel]
cmd = ["python3", "-c", "open('sentinel.txt', 'w').write('ran')"]
timeout = 10000
required = true
kind = "check"
"#;

fn cue_runtime(dir: &std::path::Path, cue_dir: &std::path::Path) -> Runtime<TextLlm, FakeEditor> {
    let config = load_test_config(dir, CUE_CHECKS_TOML, true);
    let mut runtime = Runtime::new(config, TextLlm, FakeEditor::new(), None).expect("runtime");
    runtime.set_cue_dir(cue_dir.to_path_buf());
    runtime
}

#[test]
fn cue_cannot_widen_the_approved_check_set() {
    // PH-K2 / safety §10: a cue naming a check the operator never
    // approved is refused whole — verdict `refused`, the name in
    // the evidence — and NOTHING runs, not even the approved check
    // named beside it. A cue can only ever ask for a subset of the
    // operator-approved set. Pinned at the drain itself (the exact
    // code `Runtime::run` calls): a full run's own end-of-run
    // verification legitimately runs the approved checks, which
    // would confound a sentinel-file assertion there.
    let dir = testdir("cue-widen");
    let cue_dir = dir.join("cue");
    let config = load_test_config(&dir, CUE_CHECKS_TOML, true);
    let workspace = phlow_workspace::Workspace::open(&dir, true, &[]).expect("workspace");
    let pins = phlow_checks::pin_checks(&dir, config.checks());
    let payload = serde_json::to_vec(&json!({"checks": ["sentinel", "not-approved"]})).unwrap();
    write_cue(
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

    let drain = phlow_runtime::cues::drain_cues(
        &workspace,
        config.checks(),
        &pins,
        &cue_dir,
        LaneScope::Unrestricted,
    )
    .unwrap();
    assert_eq!(drain.consumed.len(), 1);
    assert_eq!(drain.consumed[0].verdict, "refused");
    assert!(!dir.join("sentinel.txt").exists(), "nothing may run");

    let done = list_cues(&cue_dir, LaneScope::Unrestricted).unwrap();
    assert_eq!(done.len(), 1);
    let claimed = claim(&cue_dir, &done[0].id, LaneScope::Unrestricted).unwrap();
    let answer: Value = serde_json::from_slice(&claimed.payload).unwrap();
    assert_eq!(answer["verdict"], json!("refused"));
    assert!(
        answer["evidence"]
            .as_str()
            .unwrap()
            .contains("not-approved"),
        "{answer}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn malformed_cue_file_is_refused_by_name_and_the_run_proceeds() {
    // PH-K2 / safety §10: a malformed cue file is refused BY NAME
    // on the report (`cues_error`) — never parsed leniently, never
    // silently skipped — and the drain failure does not take the
    // run down with it.
    let dir = testdir("cue-malformed");
    let cue_dir = dir.join("cue");
    std::fs::create_dir_all(cue_dir.join("pending")).unwrap();
    std::fs::write(cue_dir.join("pending").join("not-a-cue.json"), "garbage").unwrap();
    let mut runtime = cue_runtime(&dir, &cue_dir);

    let report = runtime.run("do things");
    let error = report["cues_error"].as_str().expect("cues_error named");
    assert!(error.contains("not-a-cue.json"), "{error}");
    assert!(report.get("status").is_some(), "the run proceeds: {report}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cue_addressed_to_another_server_is_not_consumed() {
    // PH-K2 / safety §10: the drain consumes only cues addressed
    // to phlow. Another server's cue is left pending, untouched —
    // nobody consumes on another server's behalf.
    let dir = testdir("cue-other");
    let cue_dir = dir.join("cue");
    let mut runtime = cue_runtime(&dir, &cue_dir);
    let payload = serde_json::to_vec(&json!({"checks": ["sentinel"]})).unwrap();
    write_cue(
        &cue_dir,
        "run-this",
        "seat",
        "other-server",
        60_000,
        &payload,
        "general",
        "sfw",
        phlow_checks::run_contract::now_unix_ms(),
    )
    .unwrap();

    let report = runtime.run("do things");
    assert!(report.get("cues").is_none(), "{report}");
    let pending = list_cues(&cue_dir, LaneScope::Unrestricted).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].to, "other-server");
    let _ = std::fs::remove_dir_all(&dir);
}
