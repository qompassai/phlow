//! D-1075 rung 3 coder probe — REAL full pipeline, one coder contract
//! variant per pass. Ignored by default: it talks to the live local
//! model backend (rose) and takes minutes per task. Run deliberately,
//! under the heavy-slot protocol:
//!
//! ```sh
//! CODER3_TASKS_JSON=$HOME/.local/share/pax/ncm-s1-20261106/tasks.json \
//! CODER3_VARIANT=canonical \
//! CODER3_OUT=$HOME/.local/share/pax/detach/coder3-canonical-results.json \
//! cargo test -p phlow-runtime --test coder_variants_live -- --ignored --nocapture
//! ```
//!
//! One pass runs ONE coder contract variant over the frozen S1 task
//! set (20 tasks, frozen 2026-11-06; the set file is read, never
//! modified) as full pipeline runs at k=1 (best_of_k off): planner,
//! coder, host verification, and reviewer are all live, because the
//! coder's outcomes only exist inside a full cycle. Posture is the
//! rung-5 baseline held constant across every config — model
//! `hf-qwen3-coder-30b` via rose at 127.0.0.1:11434, temperature 0.7,
//! max_cycles = 1, max_iterations = 10 — so the coder contract
//! variant is the ONLY per-arm difference.
//!
//! Per task the pass records: run status, verified, the coder role
//! state (status, turns, error, closing-turn and anytime_snapshot
//! engagement), every coder `flow_check` event with its rung-2
//! instrumentation (`check_name` — null means the unnamed all-checks
//! call — and `check_results`), wall-clock ms, and model calls. The
//! full report for each task is written beside the results file as
//! `<out>.<task>.<variant>.full.json`.
//!
//! The companion ignored test `dump_coder3_probed_prompts` records
//! the exact coder prompt + closing strings per config to
//! `CODER3_PROMPTS_OUT` before probing (the byte-check baseline for
//! any adoption).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use phlow_config::{LoadOptions, load_config};
use phlow_editor::{EditorTransport, SCHEMAS_LUA, TransportError};
use phlow_runtime::prompt::{closing_instruction_for_role_variant, system_prompt_for_role_variant};
use phlow_runtime::transport::ReqwestTransport;
use phlow_runtime::{ContractVariant, Runtime};
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
        "phlow-coder3-{}-{}-{}",
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

fn role_state(report: &Value, role: &str) -> Value {
    report["roles"]
        .as_array()
        .expect("roles")
        .iter()
        .find(|item| item["role"] == json!(role))
        .cloned()
        .unwrap_or(Value::Null)
}

/// The coder's `flow_check` events, with the rung-2 instrumentation:
/// `check_name` is null for the unnamed (all-checks) call and the
/// called name string for a named call.
fn coder_flow_checks(report: &Value) -> Vec<Value> {
    report["events"]
        .as_array()
        .map(|events| {
            events
                .iter()
                .filter(|event| {
                    event["role"] == json!("coder") && event["tool"] == json!("flow_check")
                })
                .map(|event| {
                    json!({
                        "status": event.get("status").cloned().unwrap_or(Value::Null),
                        "check_name": event.get("check_name").cloned().unwrap_or(Value::Null),
                        "check_name_present": event.get("check_name").is_some(),
                        "check_results": event
                            .get("check_results")
                            .cloned()
                            .unwrap_or(Value::Null),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
#[ignore = "live: records the exact probed coder prompts; run before the probe"]
fn dump_coder3_probed_prompts() {
    let out_path = env_or(
        "CODER3_PROMPTS_OUT",
        "/home/phaedrus/.local/share/pax/detach/coder3-probed-prompts.json",
    );
    let ids = env_or(
        "CODER3_PROMPT_IDS",
        "canonical,check-per-write,coder-worked-example,strict-stop",
    );
    let mut configs = serde_json::Map::new();
    for id in ids.split(',').map(str::trim).filter(|id| !id.is_empty()) {
        let variant = ContractVariant::from_id(id);
        configs.insert(
            id.to_owned(),
            json!({
                "variant_id": variant.id(),
                "coder_prompt": system_prompt_for_role_variant("coder", variant),
                "coder_closing": closing_instruction_for_role_variant("coder", variant),
            }),
        );
    }
    let document = json!({ "role": "coder", "configs": configs });
    std::fs::write(
        &out_path,
        serde_json::to_string_pretty(&document).expect("prompts JSON"),
    )
    .expect("prompts file writes");
    println!("coder3 probed prompts written to {out_path}");
}

#[test]
#[ignore = "live model run; see module docs"]
fn live_coder_variant_probe() {
    let tasks_path = env_or(
        "CODER3_TASKS_JSON",
        "/home/phaedrus/.local/share/pax/ncm-s1-20261106/tasks.json",
    );
    let out_path = env_or("CODER3_OUT", "/tmp/coder3-results.json");
    let model = env_or("CODER3_MODEL", "hf-qwen3-coder-30b");
    let base_url = env_or("CODER3_BASE_URL", "http://127.0.0.1:11434");
    let variant = ContractVariant::from_id(&env_or("CODER3_VARIANT", "canonical"));
    let limit: usize = env_or("CODER3_LIMIT", "20").parse().expect("CODER3_LIMIT");

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
        // The rung-5 baseline posture, byte-identical across configs:
        // temperature 0.7, max_cycles = 1, max_iterations = 10.
        let toml = format!(
            "[ollama]\nbase_url = \"{base_url}\"\nmodel = \"{model}\"\ntemperature = 0.7\ntimeout = 600\n\n\
             [agent]\nmax_cycles = 1\nmax_iterations = 10\n\n\
             [checks.tests]\ncmd = [\"python3\", \"{test_file}\"]\nkind = \"test\"\nrequired = true\nfiletypes = [\"python\"]\ntimeout = 30000\n\n\
             [checks.lint]\ncmd = [\"true\"]\nkind = \"lint\"\nrequired = true\nfiletypes = [\"python\"]\n"
        );
        let dir = testdir(&format!("{}-{}", variant.id(), id));
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
        let mut runtime = Runtime::new(config, transport, FakeEditor, None).expect("runtime");
        runtime.set_best_of_k(1);
        runtime.set_coder_contract_variant(variant);
        let started = Instant::now();
        let report = runtime.run(prompt);
        let wall_ms = started.elapsed().as_millis();
        let coder = role_state(&report, "coder");
        let reviewer = role_state(&report, "reviewer");
        let flow_checks = coder_flow_checks(&report);
        let named = flow_checks
            .iter()
            .filter(|call| call["check_name"].is_string())
            .count();
        let unnamed = flow_checks
            .iter()
            .filter(|call| {
                call["check_name_present"] == json!(true) && call["check_name"].is_null()
            })
            .count();
        let snapshot = coder
            .get("anytime_snapshot")
            .cloned()
            .unwrap_or(Value::Null);
        let record = json!({
            "task": id,
            "variant": variant.id(),
            "status": report["status"],
            "verified": report["verified"],
            "verification_status": report["verification"]["status"],
            "verification_verified": report["verification"]["verified"],
            "wall_ms": wall_ms,
            "model_calls": report["model_calls"],
            "coder_status": coder["status"],
            "coder_turns": coder["turns"],
            "coder_error": coder.get("error").cloned().unwrap_or(Value::Null),
            "coder_closing_turn": coder.get("closing_turn").cloned().unwrap_or(Value::Bool(false)),
            "coder_contract_variant": coder
                .get("contract_variant")
                .cloned()
                .unwrap_or(Value::Null),
            "reviewer_status": reviewer["status"],
            "reviewer_approved": reviewer.get("approved").cloned().unwrap_or(Value::Null),
            "flow_check_calls": flow_checks.len(),
            "flow_check_named": named,
            "flow_check_unnamed": unnamed,
            "flow_checks": flow_checks,
            "anytime_snapshot": snapshot,
            "anytime_engaged": !snapshot.is_null(),
            "anytime_restored": snapshot["restored"] == json!(true),
            "final_module": std::fs::read_to_string(dir.join(module)).unwrap_or_default(),
        });
        println!(
            "variant {} task {id}: status={} verified={} coder={} wall_ms={wall_ms}",
            variant.id(),
            record["status"],
            record["verified"],
            record["coder_status"]
        );
        let full_path = format!("{out_path}.{id}.{}.full.json", variant.id());
        std::fs::write(
            &full_path,
            serde_json::to_string_pretty(&report).expect("report json"),
        )
        .expect("write full report");
        results.push(record);
    }
    let document = json!({
        "role": "coder",
        "variant": variant.id(),
        "model": model,
        "base_url": base_url,
        "temperature": 0.7,
        "max_cycles": 1,
        "max_iterations": 10,
        "best_of_k": 1,
        "tasks_path": tasks_path,
        "tasks": results,
    });
    std::fs::write(
        &out_path,
        serde_json::to_string_pretty(&document).expect("results JSON"),
    )
    .expect("results file writes");
    println!(
        "coder probe: variant {} over {} tasks written to {out_path}",
        variant.id(),
        results.len()
    );
}
