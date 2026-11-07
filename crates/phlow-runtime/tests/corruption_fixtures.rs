//! Corruption-tolerance fixtures for the phlow reviewer stage (NCM S2,
//! D-1047). Fixture data lives in `corruption_fixtures/fixtures.json` and
//! derives from real past loop outputs in `phlow-trials/` (see each
//! fixture's `provenance`): plans, coder summaries, and `solve.py` bodies
//! are verbatim historical artifacts carrying exactly one corruption each —
//! a wrong plan step, a planted code defect, a flipped summary claim, a
//! tampered checker, or (f10) a genuinely defective historical output.
//!
//! Two layers:
//!
//! - the standard tests (default `cargo test`): scripted transports drive
//!   the full planner → coder → verification → reviewer loop with an
//!   *approving* reviewer, pinning the loop contract — the host check
//!   must fail on every corrupted artifact it can see, and the reviewer
//!   must be handed the corrupted evidence in its context. The
//!   checker-tamper fixture is the one corruption the host check cannot
//!   see; its test pins that gap explicitly.
//!
//! Posture note: this harness runs editor-less, exactly like the
//! historical trial runs it derives from (their reports all carry
//! verification status `unverified`, reason "Missing or failing static
//! analysis for changed source"). Without editor static-analysis
//! evidence the coverage gate caps verification at `unverified`, so the
//! loop cannot reach `verified` here; the fixtures therefore assert at
//! the level this posture decides — the required host check's own
//! status (`ok` / `failed`) and the final report status it drives.
//! - the live measurement (`--ignored`): the reviewer role is delegated
//!   to the estate's real reviewer model over the production
//!   [`ReqwestTransport`]; every other role stays scripted. Per-fixture
//!   verdicts are written as JSON to `PHLOW_S2_OUT` for before/after
//!   catch-rate comparison. Model, endpoint, and output path come from
//!   `PHLOW_S2_REVIEWER_MODEL`, `PHLOW_S2_BASE_URL`, `PHLOW_S2_OUT`.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::Duration;

use phlow_config::{FlowConfig, LoadOptions, load_config};
use phlow_editor::{CALL_LUA, EditorTransport, SCHEMAS_LUA, TransportError};
use phlow_llm::{LlmError, LlmTransport};
use phlow_runtime::transport::ReqwestTransport;
use phlow_runtime::{ContractVariant, Runtime};
use serde_json::{Map, Value, json};

/// The fixture set, embedded at compile time so the tests are hermetic.
const FIXTURES_JSON: &str = include_str!("corruption_fixtures/fixtures.json");

/// The host check the real trial runs used: the workspace's own checker.
const CHECKS_TOML: &str = "[checks.tests]\n\
    cmd = [\"python3\", \"check_solution.py\"]\n\
    timeout = 60000\n\
    required = true\n\
    kind = \"test\"\n\
    filetypes = [\"python\"]\n";

/// A scripted reviewer verdict approving the implementation: the
/// adversarial case for the loop contract (the reviewer is fooled).
const APPROVING_VERDICT: &str = r#"{"approved": true, "summary": "Looks good.", "issues": []}"#;

// ---------------------------------------------------------------------------
// Fixture data
// ---------------------------------------------------------------------------

/// One corrupted-artifact fixture from the embedded set.
struct Fixture {
    id: String,
    class: String,
    plan: String,
    writes: Vec<(String, String)>,
    summary: String,
    expect_check: String,
    marker: String,
    absent: Option<String>,
    changed_files_marker: Option<String>,
}

/// Parse the embedded fixture set. Panics on malformed data: the set is
/// generated from the historical artifacts and a shape change is a
/// programming error, not runtime input.
fn fixture_set() -> (String, String, Vec<Fixture>) {
    let root: Value = serde_json::from_str(FIXTURES_JSON).expect("fixtures JSON parses");
    let task = root["task"].as_str().expect("task").to_owned();
    let check = root["check_solution_py"]
        .as_str()
        .expect("check_solution_py")
        .to_owned();
    let fixtures = root["fixtures"]
        .as_array()
        .expect("fixtures array")
        .iter()
        .map(|item| Fixture {
            id: item["id"].as_str().expect("id").to_owned(),
            class: item["class"].as_str().expect("class").to_owned(),
            plan: item["plan"].as_str().expect("plan").to_owned(),
            writes: item["writes"]
                .as_array()
                .expect("writes")
                .iter()
                .map(|write| {
                    (
                        write["path"].as_str().expect("write path").to_owned(),
                        write["content"].as_str().expect("write content").to_owned(),
                    )
                })
                .collect(),
            summary: item["summary"].as_str().expect("summary").to_owned(),
            expect_check: item["expect_check"]
                .as_str()
                .expect("expect_check")
                .to_owned(),
            marker: item["marker"].as_str().expect("marker").to_owned(),
            absent: item["absent"].as_str().map(str::to_owned),
            changed_files_marker: item["changed_files_marker"].as_str().map(str::to_owned),
        })
        .collect();
    (task, check, fixtures)
}

fn fixture(id: &str) -> Fixture {
    let (_, _, fixtures) = fixture_set();
    fixtures
        .into_iter()
        .find(|fixture| fixture.id == id)
        .expect("fixture id exists")
}

// ---------------------------------------------------------------------------
// Test infrastructure (mirrors tests/runtime.rs)
// ---------------------------------------------------------------------------

/// Unique scratch directory (canonicalized: `/tmp` may be a symlink).
fn testdir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "phlow-s2-test-{}-{}-{}",
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

/// Scripted model transport: pops chat responses in order and records
/// every payload so tests can inspect what each role was shown.
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

    /// Every payload the runtime sent, in order.
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

/// Hybrid transport for the live measurement: planner/coder calls are
/// scripted from the fixture; reviewer calls go to the real backend over
/// the production HTTP transport, so the reviewer stage runs exactly as
/// production runs it (same prompt, same context, same verdict parsing).
struct HybridLlm {
    script: ScriptLlm,
    real: ReqwestTransport,
}

/// True when the payload's system message addresses the reviewer role.
fn is_reviewer_payload(payload: &Map<String, Value>) -> bool {
    payload
        .get("messages")
        .and_then(|messages| messages.as_array())
        .and_then(|messages| messages.first())
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .is_some_and(|content| content.contains("Phlow's reviewer"))
}

impl LlmTransport for HybridLlm {
    fn post_chat(
        &mut self,
        base_url: &str,
        payload: &Map<String, Value>,
        timeout: Duration,
    ) -> Result<Value, LlmError> {
        if is_reviewer_payload(payload) {
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

/// Scripted editor transport: no schemas, so the editor is unconfigured
/// and the runtime's editor gates stay out of the measurement.
#[derive(Clone)]
struct FakeEditor {
    shared: std::sync::Arc<std::sync::Mutex<FakeState>>,
}

struct FakeState {
    schemas: Value,
    calls: HashMap<String, Value>,
}

impl FakeEditor {
    fn new() -> Self {
        FakeEditor {
            shared: std::sync::Arc::new(std::sync::Mutex::new(FakeState {
                schemas: json!([]),
                calls: HashMap::new(),
            })),
        }
    }
}

impl EditorTransport for FakeEditor {
    fn exec(
        &mut self,
        expression: &str,
        args: &[Value],
        _timeout: Duration,
    ) -> Result<Value, TransportError> {
        let state = self.shared.lock().unwrap();
        if expression == SCHEMAS_LUA {
            Ok(state.schemas.clone())
        } else if expression == CALL_LUA {
            let name = args.first().and_then(|arg| arg.as_str()).unwrap_or("");
            Ok(state
                .calls
                .get(name)
                .cloned()
                .unwrap_or(json!({"status": "ok"})))
        } else {
            Err(TransportError::Failed("unexpected expression".to_owned()))
        }
    }

    fn close(&mut self) {}
}

/// A chat completion with plain text content.
fn chat_text(content: &str) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": content}}]})
}

/// A chat completion whose message carries tool calls.
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

/// The scripted conversation for one fixture: the planner returns the
/// fixture's plan, the coder performs its writes one per turn and then
/// its summary. Reviewer responses are appended only for the fully
/// scripted (offline) runs.
fn fixture_script(fixture: &Fixture, with_reviewer: bool) -> Vec<Value> {
    let mut script = vec![chat_text(&fixture.plan)];
    for (index, (path, content)) in fixture.writes.iter().enumerate() {
        script.push(chat_tools(json!([tool_call(
            &format!("write_{index}"),
            "file_write",
            json!({"path": path, "content": content}),
        )])));
    }
    script.push(chat_text(&fixture.summary));
    if with_reviewer {
        script.push(chat_text(APPROVING_VERDICT));
        // Second cycle (the loop retries after a failed verification):
        // the coder changes nothing and the reviewer approves again.
        script.push(chat_text("Done. No further changes."));
        script.push(chat_text(APPROVING_VERDICT));
    }
    script
}

/// Prepare a fixture workspace: the host checker is pre-placed exactly
/// as in the real trial workspaces; the coder's writes land during the
/// run.
fn fixture_dir(name: &str, check: &str) -> PathBuf {
    let dir = testdir(name);
    std::fs::write(dir.join("check_solution.py"), check).expect("checker pre-placed");
    dir
}

// ---------------------------------------------------------------------------
// Live reviewer measurement (ignored: needs the served reviewer model)
// ---------------------------------------------------------------------------

/// Run every fixture through the production loop with the real reviewer
/// model and record the first reviewer verdict per fixture as JSON.
///
/// The catch definition: the first reviewer role in the report has
/// `approved == false`. The loop outcome (`status`, `verified`) is
/// recorded alongside, so reviewer catches and verification catches stay
/// distinguishable. This test asserts only that every fixture produced a
/// reviewer verdict; the catch rate is read from the output file.
#[test]
#[ignore = "live: requires the estate reviewer model served locally"]
fn live_reviewer_catch_measurement() {
    let (task, check, fixtures) = fixture_set();
    let model = std::env::var("PHLOW_S2_REVIEWER_MODEL")
        .unwrap_or_else(|_| "hf-granite-4.2-8b-tools".to_owned());
    let base_url =
        std::env::var("PHLOW_S2_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".to_owned());
    let out_path =
        std::env::var("PHLOW_S2_OUT").unwrap_or_else(|_| "s2-live-results.json".to_owned());
    // D-1063: the reviewer contract variant under probe, selected by
    // id. Default canonical: with no selection this measurement is
    // exactly the rung-1/2 harness.
    let variant = ContractVariant::from_id(
        &std::env::var("PHLOW_R3_REVIEWER_VARIANT").unwrap_or_else(|_| "canonical".to_owned()),
    );
    // The [agent] budgets mirror the production role wiring
    // (~/.config/phlow/roles-wired.toml, 2026-11-02) so the measurement
    // runs the reviewer under its production posture.
    let toml = format!(
        "[ollama]\nbase_url = \"{base_url}\"\nmodel = \"{model}\"\n\
         temperature = 0.2\ncontext_length = 32768\ntimeout = 300\n\
         allow_remote = false\n\n[models]\nreviewer = \"{model}\"\n\n\
         [agent]\nmax_iterations = 6\nmax_cycles = 2\nmax_tool_calls = 64\n\n{CHECKS_TOML}"
    );
    let mut results: Vec<Value> = Vec::with_capacity(fixtures.len());
    for item in &fixtures {
        let dir = fixture_dir(&item.id, &check);
        let config = load_test_config(&dir, &toml);
        let script = ScriptLlm::new(fixture_script(item, false));
        let real = ReqwestTransport::new().expect("HTTP transport builds");
        let hybrid = HybridLlm { script, real };
        let mut runtime =
            Runtime::new(config, hybrid, FakeEditor::new(), None).expect("runtime builds");
        runtime.set_reviewer_contract_variant(variant);
        let report = runtime.run(&task);
        let reviewer = report["roles"]
            .as_array()
            .expect("roles")
            .iter()
            .find(|role| role["role"] == json!("reviewer"))
            .expect("a reviewer role ran")
            .clone();
        results.push(json!({
            "id": item.id,
            "class": item.class,
            "reviewer_approved": reviewer["approved"],
            "reviewer_status": reviewer["status"],
            "reviewer_issues": reviewer["issues"],
            "reviewer_summary": reviewer["summary"],
            "reviewer_role": reviewer,
            "verification": report["verification"],
            "cycles": report["cycles"],
            "final_status": report["status"],
            "verified": report["verified"],
        }));
    }
    let caught = results
        .iter()
        .filter(|result| result["reviewer_approved"] == json!(false))
        .count();
    let document = json!({
        "model": model,
        "base_url": base_url,
        "reviewer_variant": variant.id(),
        "fixtures": results,
        "reviewer_caught": caught,
    });
    std::fs::write(
        &out_path,
        serde_json::to_string_pretty(&document).expect("results JSON"),
    )
    .expect("results file writes");
    println!(
        "live measurement: reviewer caught {caught} of {} fixtures",
        results.len()
    );
}

// ---------------------------------------------------------------------------
// Standard loop tests (scripted, hermetic)
// ---------------------------------------------------------------------------

/// Drive one fixture through the full loop with an approving scripted
/// reviewer and return the report plus every payload the roles were
/// shown.
fn run_offline(fixture: &Fixture) -> (Value, Vec<Map<String, Value>>) {
    let (task, check, _) = fixture_set();
    let dir = fixture_dir(&fixture.id, &check);
    let config = load_test_config(&dir, CHECKS_TOML);
    let llm = ScriptLlm::new(fixture_script(fixture, true));
    let mut runtime =
        Runtime::new(config, llm.clone(), FakeEditor::new(), None).expect("runtime builds");
    let report = runtime.run(&task);
    (report, llm.payloads())
}

/// The user content of the first reviewer call: the evidence the
/// reviewer stage was actually handed (plan, implementation summary,
/// changed files, host verification).
fn reviewer_evidence(payloads: &[Map<String, Value>]) -> String {
    let payload = payloads
        .iter()
        .find(|payload| is_reviewer_payload(payload))
        .expect("reviewer was called");
    payload["messages"][1]["content"]
        .as_str()
        .expect("reviewer user content")
        .to_owned()
}

/// The status of the required host check inside the report's
/// verification object.
fn host_check_status(report: &Value) -> String {
    report["verification"]["checks"]
        .as_array()
        .expect("verification checks")
        .iter()
        .find(|check| check["name"] == json!("tests"))
        .expect("the tests check ran")["status"]
        .as_str()
        .expect("check status")
        .to_owned()
}

/// The shared contract for one fixture under an approving reviewer: the
/// corrupted evidence reaches the reviewer's context, the host check's
/// outcome matches what the check can actually see, and the final
/// report status follows. In this editor-less posture a passing check
/// leaves the run `unverified` (the coverage gate), never `ok`.
fn assert_fixture(fixture_id: &str) {
    let item = fixture(fixture_id);
    let (report, payloads) = run_offline(&item);
    let evidence = reviewer_evidence(&payloads);
    assert!(
        evidence.contains(&item.marker),
        "{}: reviewer evidence must contain the marker {:?}",
        item.id,
        item.marker
    );
    if let Some(absent) = &item.absent {
        assert!(
            !evidence.contains(absent),
            "{}: reviewer evidence must not contain {:?}",
            item.id,
            absent
        );
    }
    if let Some(marker) = &item.changed_files_marker {
        let start = evidence
            .find("\"changed_files\"")
            .expect("changed_files in reviewer context");
        let window: String = evidence[start..].chars().take(300).collect();
        assert!(
            window.contains(marker),
            "{}: changed_files must name {marker}",
            item.id
        );
    }
    assert_eq!(
        host_check_status(&report),
        item.expect_check,
        "{}: host check outcome",
        item.id
    );
    let expected_status = if item.expect_check == "failed" {
        "failed"
    } else {
        "unverified"
    };
    assert_eq!(
        report["status"],
        json!(expected_status),
        "{}: final status",
        item.id
    );
}

#[test]
fn f00_control_clean_artifact_passes_host_check() {
    assert_fixture("f00-control-clean");
}

#[test]
fn f01_plan_wrong_step_fails_host_check() {
    assert_fixture("f01-plan-wrong-step-median-index");
}

#[test]
fn f02_plan_clamp_swap_fails_host_check() {
    assert_fixture("f02-plan-wrong-step-clamp-swap");
}

#[test]
fn f03_plan_dropped_step_fails_host_check() {
    assert_fixture("f03-plan-dropped-step-clamp");
}

#[test]
fn f04_median_max_defect_fails_host_check() {
    assert_fixture("f04-diff-defect-median-max");
}

#[test]
fn f05_median_mean_defect_fails_host_check() {
    assert_fixture("f05-diff-defect-median-mean");
}

#[test]
fn f06_clamp_without_lo_fails_host_check() {
    assert_fixture("f06-diff-defect-clamp-no-lo");
}

/// The checker-tamper fixture is the set's known host-check blind spot:
/// the weakened checker passes, so the run's only remaining control is
/// the reviewer. The test pins both halves of that fact — the host
/// check is fooled, and the tampered file is named in the reviewer's
/// evidence — because the reviewer stage (measured live, above) is the
/// only stage that can close it.
#[test]
fn f07_checker_tamper_fools_host_check_but_reaches_reviewer() {
    assert_fixture("f07-checker-tamper");
}

#[test]
fn f08_flipped_verification_claim_fails_host_check() {
    assert_fixture("f08-summary-flipped-verification");
}

#[test]
fn f09_flipped_constraint_fails_host_check() {
    assert_fixture("f09-summary-flipped-constraint");
}

#[test]
fn f10_real_historical_defect_fails_host_check() {
    assert_fixture("f10-real-defect-escaped-docstrings");
}
