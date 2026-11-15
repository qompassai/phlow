//! PH-9 — self-improvement safety case, `phlow-runtime` suite.
//! The runtime half of the case: nothing the *model* can reach may
//! mutate checks, budgets, prompts, skills, or history — including
//! the PH-1 outcome store, which the runtime writes itself. Full
//! case: `docs/SELF_IMPROVEMENT_SAFETY.md`.

use std::path::PathBuf;
use std::time::Duration;

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
