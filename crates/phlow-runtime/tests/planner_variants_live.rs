//! D-1063 planner probe — REAL planner, scripted coder/reviewer.
//! Ignored by default: it talks to the live local model backend (rose)
//! and takes minutes per variant pass. Run deliberately:
//!
//! ```sh
//! PHLOW_R3_TASKS_JSON=$HOME/.local/share/pax/ncm-s1-20261106/tasks.json \
//! PHLOW_R3_PLANNER_VARIANT=canonical \
//! PHLOW_R3_PLANNER_OUT=$HOME/.local/share/pax/detach/rung3-planner-canonical.json \
//! cargo test -p phlow-runtime --test planner_variants_live -- --ignored --nocapture
//! ```
//!
//! One pass runs ONE planner contract variant over the frozen S1 task
//! set (20 tasks, frozen 2026-11-06; the set file is read, never
//! modified). The planner role goes to the real backend over the
//! production [`ReqwestTransport`]; the coder and reviewer are
//! scripted (the S2 harness pattern), so the planner's plan — recorded
//! verbatim in the output JSON — is the only live artifact. Posture
//! mirrors rungs 1–2 (max_iterations = 6, max_tool_calls = 64,
//! temperature 0.2, watchdog active) with max_cycles = 1: downstream
//! roles are stubs, and a second cycle would re-plan against a stub
//! coder's non-fix, muddying the per-variant plan metrics. The model
//! is held constant across all variants
//! (`PHLOW_R3_PLANNER_MODEL`, default `hf-granite-4.2-8b-tools`, the
//! same model as the reviewer probe).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use phlow_config::{FlowConfig, LoadOptions, load_config};
use phlow_editor::{EditorTransport, SCHEMAS_LUA, TransportError};
use phlow_llm::{LlmError, LlmTransport};
use phlow_runtime::transport::ReqwestTransport;
use phlow_runtime::{ContractVariant, Runtime};
use serde_json::{Map, Value, json};

fn testdir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "phlow-r3-planner-{}-{}-{}",
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

fn load_test_config(dir: &std::path::Path, toml: &str) -> FlowConfig {
    std::fs::write(dir.join("config.toml"), toml).expect("config.toml");
    load_config(&LoadOptions {
        config_path: Some(dir.join("config.toml")),
        workspace: Some(dir.to_path_buf()),
        trusted: true,
        model: None,
    })
    .expect("test config loads")
}

fn chat_text(content: &str) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": content}}]})
}

const APPROVING_VERDICT: &str =
    r#"{"approved": true, "summary": "Scripted reviewer: probe stub.", "issues": []}"#;

/// Scripted transport for the non-planner roles: the coder reports no
/// changes, the reviewer approves. The fallback is the approving
/// verdict so any extra scripted call stays benign.
#[derive(Clone)]
struct ScriptLlm {
    shared: std::sync::Arc<std::sync::Mutex<VecDeque<Value>>>,
}

impl ScriptLlm {
    fn new() -> Self {
        ScriptLlm {
            shared: std::sync::Arc::new(std::sync::Mutex::new(
                vec![
                    chat_text("Done. No changes (probe stub coder)."),
                    chat_text(APPROVING_VERDICT),
                ]
                .into(),
            )),
        }
    }
}

impl LlmTransport for ScriptLlm {
    fn post_chat(
        &mut self,
        _base_url: &str,
        _payload: &Map<String, Value>,
        _timeout: Duration,
    ) -> Result<Value, LlmError> {
        let mut script = self.shared.lock().unwrap();
        Ok(script
            .pop_front()
            .unwrap_or_else(|| chat_text(APPROVING_VERDICT)))
    }

    fn get_tags(&mut self, _base_url: &str, _timeout: Duration) -> Result<Value, LlmError> {
        Ok(json!({"models": []}))
    }

    fn close(&mut self) {}
}

/// Hybrid transport: planner calls go to the real backend; every
/// other role is scripted.
struct HybridLlm {
    script: ScriptLlm,
    real: ReqwestTransport,
}

fn is_planner_payload(payload: &Map<String, Value>) -> bool {
    payload
        .get("messages")
        .and_then(|messages| messages.as_array())
        .and_then(|messages| messages.first())
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .is_some_and(|content| content.contains("Phlow's planner"))
}

impl LlmTransport for HybridLlm {
    fn post_chat(
        &mut self,
        base_url: &str,
        payload: &Map<String, Value>,
        timeout: Duration,
    ) -> Result<Value, LlmError> {
        if is_planner_payload(payload) {
            self.real.post_chat(base_url, payload, timeout)
        } else {
            self.script.post_chat(base_url, payload, timeout)
        }
    }

    fn get_tags(&mut self, base_url: &str, timeout: Duration) -> Result<Value, LlmError> {
        self.real.get_tags(base_url, timeout)
    }

    fn close(&mut self) {
        self.script.close();
        self.real.close();
    }
}

#[derive(Clone)]
struct FakeEditor;

impl EditorTransport for FakeEditor {
    fn exec(
        &mut self,
        expression: &str,
        _args: &[Value],
        _timeout: Duration,
    ) -> Result<Value, TransportError> {
        if expression == SCHEMAS_LUA {
            Ok(json!([]))
        } else {
            Err(TransportError::Failed("unexpected expression".to_owned()))
        }
    }

    fn close(&mut self) {}
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

#[test]
#[ignore = "live: requires the planner model served locally"]
fn live_planner_variant_probe() {
    let tasks_path = env_or(
        "PHLOW_R3_TASKS_JSON",
        "/home/phaedrus/.local/share/pax/ncm-s1-20261106/tasks.json",
    );
    let out_path = env_or("PHLOW_R3_PLANNER_OUT", "r3-planner-results.json");
    let model = env_or("PHLOW_R3_PLANNER_MODEL", "hf-granite-4.2-8b-tools");
    let base_url = env_or("PHLOW_R3_BASE_URL", "http://127.0.0.1:11434");
    let variant = ContractVariant::from_id(&env_or("PHLOW_R3_PLANNER_VARIANT", "canonical"));
    let limit: usize = env_or("PHLOW_R3_LIMIT", "20")
        .parse()
        .expect("limit parses");

    let raw = std::fs::read_to_string(&tasks_path).expect("tasks json reads");
    let set: Value = serde_json::from_str(&raw).expect("tasks json parses");
    let tasks = set["tasks"].as_array().expect("tasks array");

    let mut results: Vec<Value> = Vec::with_capacity(tasks.len());
    for task in tasks.iter().take(limit) {
        let id = task["id"].as_str().expect("id");
        let module = task["module"].as_str().expect("module");
        let starter = task["starter"].as_str().expect("starter");
        let tests = task["tests"].as_str().expect("tests");
        let prompt = task["task"].as_str().expect("task text");
        let test_file = format!("test_{id}.py");
        let toml = format!(
            "[ollama]\nbase_url = \"{base_url}\"\nmodel = \"{model}\"\n\
             temperature = 0.2\ncontext_length = 32768\ntimeout = 300\n\
             allow_remote = false\n\n[models]\nplanner = \"{model}\"\n\n\
             [agent]\nmax_iterations = 6\nmax_cycles = 1\nmax_tool_calls = 64\n\n\
             [checks.tests]\ncmd = [\"python3\", \"{test_file}\"]\nkind = \"test\"\n\
             required = true\nfiletypes = [\"python\"]\ntimeout = 30000\n\n\
             [checks.lint]\ncmd = [\"true\"]\nkind = \"lint\"\nrequired = true\n\
             filetypes = [\"python\"]\n"
        );
        let dir = testdir(&format!("{}-{}", variant.id(), id));
        std::fs::write(dir.join(module), starter).expect("starter");
        std::fs::write(dir.join(&test_file), tests).expect("tests");
        let config = load_test_config(&dir, &toml);
        let hybrid = HybridLlm {
            script: ScriptLlm::new(),
            real: ReqwestTransport::new().expect("HTTP transport builds"),
        };
        let mut runtime = Runtime::new(config, hybrid, FakeEditor, None).expect("runtime builds");
        runtime.set_planner_contract_variant(variant);
        let report = runtime.run(prompt);
        let planner = report["roles"]
            .as_array()
            .expect("roles")
            .iter()
            .find(|role| role["role"] == json!("planner"))
            .expect("a planner role ran")
            .clone();
        results.push(json!({
            "id": id,
            "module": module,
            "test_file": test_file,
            "plan": planner["summary"],
            "planner_status": planner["status"],
            "planner_role": planner,
            "final_status": report["status"],
        }));
        println!(
            "variant {} task {id}: planner status {}",
            variant.id(),
            results.last().expect("record")["planner_status"]
        );
    }
    let document = json!({
        "role": "planner",
        "variant": variant.id(),
        "model": model,
        "base_url": base_url,
        "tasks": results,
    });
    std::fs::write(
        &out_path,
        serde_json::to_string_pretty(&document).expect("results JSON"),
    )
    .expect("results file writes");
    println!(
        "planner probe: variant {} over {} tasks written to {out_path}",
        variant.id(),
        results.len()
    );
}
