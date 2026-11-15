//! PH-6 — lesson cards at the runtime boundary.
//!
//! Three behaviours, driven through the real [`Runtime`] with a
//! scripted, capturing transport:
//!
//! 1. A run on a seat with `[agent] task_class` records its outcome
//!    row under that class (not the `flow_run` surface name) and
//!    leaves a plan-divergence record derived from its own plan.
//! 2. When a lesson card exists for the class, the planner receives
//!    it as run context — and the planner's system prompt is
//!    byte-identical to a run without the card (retrieved lessons,
//!    not prompt mutation).
//! 3. The card changes nothing else: the run's status with a card
//!    equals the same run's status without one (the PH-6 gauntlet
//!    gate in its bounded, scripted form).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use phlow_config::{FlowConfig, LoadOptions, load_config};
use phlow_editor::{EditorTransport, SCHEMAS_LUA, TransportError};
use phlow_llm::{LlmError, LlmTransport};
use phlow_runtime::Runtime;
use phlow_self_improve::OutcomeStore;
use phlow_self_improve::divergence::{DivergenceStore, PlanDivergence};
use phlow_self_improve::lessons::LessonStore;
use serde_json::{Map, Value, json};

fn testdir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "phlow-lessons-test-{}-{}-{}",
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

const PLAN: &str = "**Goal**\nFix the bound in `src/a.rs`.\n\n**Files**\n- `src/a.rs`\n\n\
                    **Steps**\n1. Replace the value in `src/a.rs`.\n2. Save the file.\n\n\
                    **Checks**\ncargo test";

/// One captured chat: the system prompt and the user content of the
/// first planner call this transport served.
#[derive(Default)]
struct Capture {
    planner_system: Option<String>,
    planner_user: Option<String>,
}

/// A transport that plays all three roles from their system
/// prompts and captures what the planner was given.
#[derive(Clone)]
struct ScriptedLlm {
    capture: Arc<Mutex<Capture>>,
}

impl ScriptedLlm {
    fn new() -> Self {
        ScriptedLlm {
            capture: Arc::new(Mutex::new(Capture::default())),
        }
    }
}

impl LlmTransport for ScriptedLlm {
    fn post_chat(
        &mut self,
        _base_url: &str,
        payload: &Map<String, Value>,
        _timeout: Duration,
    ) -> Result<Value, LlmError> {
        let messages = payload
            .get("messages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let system = messages
            .first()
            .and_then(|m| m.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        if system.contains("Your deliverable is a concrete plan") {
            let user = messages
                .get(1)
                .and_then(|m| m.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let mut capture = self.capture.lock().unwrap();
            if capture.planner_system.is_none() {
                capture.planner_system = Some(system);
                capture.planner_user = Some(user);
            }
            return Ok(chat_text(PLAN));
        }
        if system.contains("Your reply is the verdict") {
            return Ok(chat_text(
                "{\"approved\":true,\"summary\":\"ok\",\"issues\":[]}",
            ));
        }
        Ok(chat_text("Done."))
    }

    fn get_tags(&mut self, _base_url: &str, _timeout: Duration) -> Result<Value, LlmError> {
        Ok(json!({"models": []}))
    }

    fn close(&mut self) {}
}

#[derive(Clone)]
struct FakeEditor {
    shared: Arc<Mutex<usize>>,
}

impl FakeEditor {
    fn new() -> Self {
        FakeEditor {
            shared: Arc::new(Mutex::new(0)),
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

const CLASS_CONFIG: &str = "[agent]\ntask_class = \"code_fix\"\n";

#[test]
fn run_records_its_class_and_its_divergence() {
    let dir = testdir("records");
    let db = dir.join("feedback.db");
    let config = load_test_config(&dir, CLASS_CONFIG);
    let mut runtime =
        Runtime::new(config, ScriptedLlm::new(), FakeEditor::new(), None).expect("runtime");
    runtime.set_outcome_db(db.clone());

    let report = runtime.run("fix the bound");
    let run_id = report["outcome_run_id"]
        .as_str()
        .expect("report names its outcome run id")
        .to_owned();
    assert!(report.get("divergence_error").is_none());

    // The outcome row carries the seat's declared class.
    let outcomes = OutcomeStore::open(&db).expect("store opens");
    let row = outcomes.get(&run_id).unwrap().expect("row exists");
    assert_eq!(row.outcome.task_class, "code_fix");

    // The divergence record was derived from the run's own plan:
    // the scripted coder changed nothing, so the file-naming step
    // was skipped, the prose step is untracked, and the planned
    // file stayed untouched.
    let divergences = DivergenceStore::open(&db).expect("store opens");
    let record = divergences
        .get(&run_id)
        .unwrap()
        .expect("divergence record exists");
    let d = &record.divergence;
    assert_eq!(d.task_class, "code_fix");
    assert_eq!(d.planned_steps, 2);
    assert_eq!(d.executed_steps, 0);
    assert_eq!(d.skipped_steps.len(), 1);
    assert_eq!(d.untracked_steps, 1);
    assert_eq!(d.planned_files_untouched, vec!["src/a.rs".to_owned()]);
    assert!(d.added_files.is_empty());
    assert_eq!(d.origin_lane, "phlow");
    assert_eq!(d.content_label, "nsfw");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn planner_gets_the_card_as_context_and_the_prompt_never_changes() {
    // Control run: card-less store.
    let control_dir = testdir("control");
    let control_db = control_dir.join("feedback.db");
    let control_llm = ScriptedLlm::new();
    let control_capture = control_llm.capture.clone();
    let config = load_test_config(&control_dir, CLASS_CONFIG);
    let mut runtime = Runtime::new(config, control_llm, FakeEditor::new(), None).expect("runtime");
    runtime.set_outcome_db(control_db);
    let control_report = runtime.run("fix the bound");
    assert!(control_report.get("lessons").is_none());

    // Card run: same seat, same task, one seeded divergence built
    // into a lesson card before the run starts.
    let card_dir = testdir("card");
    let card_db = card_dir.join("feedback.db");
    {
        let divergences = DivergenceStore::open(&card_db).unwrap();
        divergences
            .record(&PlanDivergence {
                run_id: "seed-run".to_owned(),
                task_class: "code_fix".to_owned(),
                planned_steps: 2,
                executed_steps: 0,
                skipped_steps: vec!["Replace the value in `src/a.rs`.".to_owned()],
                untracked_steps: 1,
                planned_files_untouched: vec!["src/a.rs".to_owned()],
                added_files: Vec::new(),
                repair_cycles: 1,
                origin_lane: "phlow".to_owned(),
                content_label: "nsfw".to_owned(),
            })
            .unwrap();
        let lessons = LessonStore::open(&card_db).unwrap();
        let card = lessons
            .build_card(&divergences, "code_fix")
            .unwrap()
            .expect("card builds from the seed");
        assert_eq!(card.version, 1);
    }
    let card_llm = ScriptedLlm::new();
    let card_capture = card_llm.capture.clone();
    let config = load_test_config(&card_dir, CLASS_CONFIG);
    let mut runtime = Runtime::new(config, card_llm, FakeEditor::new(), None).expect("runtime");
    runtime.set_outcome_db(card_db);
    let card_report = runtime.run("fix the bound");

    // The report names the card the run carried.
    assert_eq!(card_report["lessons"]["task_class"], json!("code_fix"));
    assert_eq!(card_report["lessons"]["card_version"], json!(1));
    assert_eq!(card_report["lessons"]["built_over_runs"], json!(1));

    let control = control_capture.lock().unwrap();
    let with_card = card_capture.lock().unwrap();
    // The planner's system prompt is byte-identical with and
    // without the card: lessons are retrieved context, never a
    // prompt mutation.
    assert_eq!(
        control.planner_system.as_deref(),
        with_card.planner_system.as_deref()
    );
    // The control planner's context is empty; the card planner's
    // context carries the rendered card.
    let control_user: Value =
        serde_json::from_str(control.planner_user.as_deref().unwrap()).unwrap();
    assert_eq!(control_user["context"], json!({}));
    let card_user: Value =
        serde_json::from_str(with_card.planner_user.as_deref().unwrap()).unwrap();
    let lessons_text = card_user["context"]["lessons"].as_str().unwrap();
    assert!(lessons_text.contains("lesson card v1"));
    assert!(lessons_text.contains("src/a.rs"));

    // The PH-6 gate in bounded form: with cards does no worse than
    // without — the same scripted run ends in the same status.
    assert_eq!(control_report["status"], card_report["status"]);
    let _ = std::fs::remove_dir_all(&control_dir);
    let _ = std::fs::remove_dir_all(&card_dir);
}
