//! NCM S1 (D-1050) decisive test — REAL model arms. Ignored by default:
//! it talks to the live local model backend (rose) and takes minutes
//! per task. Run deliberately, when primo's GPU is free:
//!
//! ```sh
//! S1_TASKS_JSON=$HOME/workspace/ncm-s1-20261106/tasks.json \
//! S1_OUT=/tmp/s1-real-results.json S1_LIMIT=20 \
//! cargo test -p phlow-runtime --test best_of_k_real -- --ignored --nocapture
//! ```
//!
//! Arm A: single-sample (`k = 1`). Arm B: `k = 4`, selected by the
//! runtime's verification-first rule. Both arms: same model, same
//! temperature (0.7), `max_cycles = 1`, so `k` is the only difference.
//! Results (per task: status, verified, wall-clock ms, best_of_k block)
//! are written as JSON to `S1_OUT`. Success is a measurement here, not
//! an assertion — the comparison lives in the S1 report.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use phlow_config::{LoadOptions, load_config};
use phlow_editor::{EditorTransport, SCHEMAS_LUA, TransportError};
use phlow_runtime::Runtime;
use phlow_runtime::transport::ReqwestTransport;
use serde_json::{Value, json};

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

fn testdir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "phlow-best-of-k-real-{}-{}-{}",
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

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

#[test]
#[ignore = "live model run; see module docs"]
fn real_model_arms() {
    let tasks_path = env_or("S1_TASKS_JSON", "tasks.json");
    let out_path = env_or("S1_OUT", "/tmp/s1-real-results.json");
    let limit: usize = env_or("S1_LIMIT", "20").parse().expect("S1_LIMIT");
    let model = env_or("S1_MODEL", "hf-qwen3-coder-30b");
    let base_url = env_or("S1_BASE_URL", "http://127.0.0.1:11434");

    let raw = std::fs::read_to_string(&tasks_path).expect("tasks json");
    let set: Value = serde_json::from_str(&raw).expect("tasks parse");
    let tasks = set["tasks"].as_array().expect("tasks array");

    let mut results = Vec::new();
    for task in tasks.iter().take(limit) {
        let id = task["id"].as_str().expect("id");
        let module = task["module"].as_str().expect("module");
        let starter = task["starter"].as_str().expect("starter");
        let tests = task["tests"].as_str().expect("tests");
        let prompt = task["task"].as_str().expect("task text");
        let test_file = format!("test_{id}.py");
        let toml = format!(
            "[ollama]\nbase_url = \"{base_url}\"\nmodel = \"{model}\"\ntemperature = 0.7\ntimeout = 600\n\n\
             [agent]\nmax_cycles = 1\nmax_iterations = 10\n\n\
             [checks.tests]\ncmd = [\"python3\", \"{test_file}\"]\nkind = \"test\"\nrequired = true\nfiletypes = [\"python\"]\ntimeout = 30000\n\n\
             [checks.lint]\ncmd = [\"true\"]\nkind = \"lint\"\nrequired = true\nfiletypes = [\"python\"]\n"
        );
        let mut arms = Vec::new();
        let arms_filter = env_or("S1_ARMS", "AB");
        for (arm, k) in [("A", 1u32), ("B", 4u32)] {
            if !arms_filter.contains(arm) {
                continue;
            }
            let dir = testdir(&format!("{id}-{arm}"));
            std::fs::write(dir.join(module), starter).expect("starter");
            std::fs::write(dir.join(&test_file), tests).expect("tests");
            std::fs::write(dir.join("config.toml"), &toml).expect("config");
            let config = load_config(&LoadOptions {
                config_path: Some(dir.join("config.toml")),
                workspace: Some(dir.clone()),
                trusted: true,
                model: None,
            })
            .expect("config loads");
            let transport = ReqwestTransport::new().expect("transport builds");
            let mut rt = Runtime::new(config, transport, FakeEditor, None).expect("runtime");
            rt.set_best_of_k(k);
            let started = Instant::now();
            let report = rt.run(prompt);
            let wall_ms = started.elapsed().as_millis();
            let record = json!({
                "task": id,
                "arm": arm,
                "k": k,
                "status": report["status"],
                "verified": report["verified"],
                "wall_ms": wall_ms,
                "model_calls": report["model_calls"],
                "best_of_k": report.get("best_of_k").cloned().unwrap_or(Value::Null),
                "final_module": std::fs::read_to_string(dir.join(module)).unwrap_or_default(),
            });
            println!(
                "task {id} arm {arm}: status={} verified={} wall_ms={wall_ms}",
                record["status"], record["verified"]
            );
            arms.push(record);
            let full_path = format!("{out_path}.{id}.{arm}.full.json");
            std::fs::write(
                &full_path,
                serde_json::to_string_pretty(&report).expect("report json"),
            )
            .expect("write full report");
        }
        results.push(json!({"task": id, "arms": arms}));
    }
    std::fs::write(
        &out_path,
        serde_json::to_string_pretty(&results).expect("json"),
    )
    .expect("write results");
    println!("results written to {out_path}");
}
