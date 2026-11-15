//! PH-13 acceptance: a mapped row is accepted by bloch's intake
//! gate — in a test that links both. The gate here is bloch's own
//! `bloch::outcome::OutcomeRecord::from_intake_json` (blochs
//! 1.0.0, dev-dependency), never a re-implementation: if bloch's
//! schema or refusal vocabulary moves, this suite is where phlow
//! finds out.

use std::collections::HashMap;
use std::path::PathBuf;

use phlow_flowguard::label::{LabelSnapshot, ReaderSet};
use phlow_self_improve::capture::CaptureStore;
use phlow_self_improve::export::{ExportScope, export_outcomes, intake_object};
use phlow_self_improve::outcome::{OutcomeStore, RunOutcome, VerificationStrength};

fn temp_dir(prefix: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "phlow-bloch-link-test-{prefix}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).expect("test setup: create temp dir");
    dir
}

fn row(run_id: &str) -> RunOutcome {
    RunOutcome {
        run_id: run_id.to_owned(),
        task_class: "analysis".to_owned(),
        plan: "the plan".to_owned(),
        checks_json: "[{\"name\":\"tests\",\"status\":\"ok\"}]".to_owned(),
        reviewer_verdict: Some("approved".to_owned()),
        cycles: 1,
        tool_calls: 2,
        model_calls: 3,
        context_chars: None,
        wall_ms: 42_500,
        model_ids: vec!["model-x".to_owned()],
        skills: vec!["tiger-style-rust".to_owned()],
        status: "ok".to_owned(),
        verification_strength: VerificationStrength::ToolChecked,
        origin_lane: "phlow".to_owned(),
        content_label: "nsfw".to_owned(),
        evidence: "test fixture".to_owned(),
    }
}

/// Record one run and capture it; `public` decides the lane the
/// capture derives (public trajectory → general/sfw, none →
/// restricted/nsfw, fail-closed).
fn store_with_capture(dir: &std::path::Path, run_id: &str, public: bool) -> PathBuf {
    let db = dir.join("feedback.db");
    let outcomes = OutcomeStore::open(&db).unwrap();
    let captures = CaptureStore::open(&db).unwrap();
    outcomes.record(&row(run_id)).unwrap();
    let snapshot = LabelSnapshot {
        readers: ReaderSet::All,
        trust: "operator".to_owned(),
    };
    captures
        .capture(
            &outcomes,
            run_id,
            if public { Some(&snapshot) } else { None },
        )
        .unwrap();
    db
}

#[test]
fn mapped_row_is_accepted_by_blochs_intake_gate() {
    let dir = temp_dir("gate");
    let db = store_with_capture(&dir, "run-1", true);
    let outcomes = OutcomeStore::open(&db).unwrap();
    let captures = CaptureStore::open(&db).unwrap();
    let recorded = outcomes.get("run-1").unwrap().expect("row");
    let capture = captures.get("run-1").unwrap().expect("capture");

    let object = intake_object(&recorded, &capture.capture, "run the S1 gate").unwrap();
    let record = bloch::outcome::OutcomeRecord::from_intake_json(&object)
        .expect("bloch's intake gate accepts the mapped row");
    assert_eq!(record.outcome_id, "run-1");
    assert_eq!(record.server, "phlow");
    assert_eq!(record.asked, "run the S1 gate");
    assert_eq!(record.result, bloch::outcome::OutcomeResult::Ok);
    assert_eq!(
        record.verification,
        bloch::outcome::VerificationStrength::ToolChecked
    );
    assert_eq!(record.cost.wall_secs, 42);
    assert_eq!(record.cost.tokens, 0);
    assert_eq!(record.skills, vec!["tiger-style-rust"]);
    assert_eq!(record.origin_lane, bloch::Lane::General);
    assert_eq!(record.content_label, bloch::ContentLabel::Sfw);
    assert!(record.visible_to_general());
    assert!(record.lesson_eligible());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exporter_output_is_a_bloch_store() {
    // End to end: phlow's exporter writes the target file; bloch's
    // own reader reads it back as records, lane law intact.
    let dir = temp_dir("store");
    let db = store_with_capture(&dir, "run-pub", true);
    let outcomes = OutcomeStore::open(&db).unwrap();
    let captures = CaptureStore::open(&db).unwrap();
    outcomes.record(&row("run-res")).unwrap();
    captures.capture(&outcomes, "run-res", None).unwrap();
    let tasks = HashMap::from([
        ("run-pub".to_owned(), "public task".to_owned()),
        ("run-res".to_owned(), "restricted task".to_owned()),
    ]);
    let target = dir.join("outcomes.jsonl");
    let summary = export_outcomes(&db, &target, ExportScope::Unrestricted, &tasks).unwrap();
    assert_eq!(summary.exported.len(), 2);

    let records = bloch::outcome::read_outcomes(&target).expect("bloch reads the store");
    assert_eq!(records.len(), 2);
    let public = &records[0];
    assert_eq!(public.outcome_id, "run-pub");
    assert!(public.visible_to_general());
    let restricted = &records[1];
    assert_eq!(restricted.outcome_id, "run-res");
    assert_eq!(restricted.origin_lane, bloch::Lane::Restricted);
    assert_eq!(restricted.content_label, bloch::ContentLabel::Nsfw);
    assert!(!restricted.visible_to_general());
    assert!(restricted.lesson_eligible());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bloch_gate_refuses_what_phlow_refuses() {
    // Wire-vocabulary parity, the other direction: the records
    // phlow's mapping refuses to build are records bloch's gate
    // refuses to accept, with the same refusal kinds.
    let mut object = serde_json::json!({
        "outcome_id": "run-x",
        "server": "phlow",
        "asked": "task",
        "produced": [],
        "result": "ok",
        "verification": "tool_checked",
        "cost": {"wall_secs": 1, "tokens": 0, "disk_bytes": 0},
        "skills": [],
        "origin_lane": "general",
        "content_label": "sfw",
        "evidence": "evidence",
        "recorded_at_epoch": 1_767_225_600,
    });
    assert!(bloch::outcome::OutcomeRecord::from_intake_json(&object).is_ok());

    object["origin_lane"] = serde_json::json!("unknown");
    let error = bloch::outcome::OutcomeRecord::from_intake_json(&object).unwrap_err();
    assert!(error.starts_with("unlabelled_outcome"), "{error}");

    object["origin_lane"] = serde_json::json!("general");
    object.as_object_mut().unwrap().remove("result");
    let error = bloch::outcome::OutcomeRecord::from_intake_json(&object).unwrap_err();
    assert!(error.starts_with("missing_result"), "{error}");

    object["result"] = serde_json::json!("ok");
    object.as_object_mut().unwrap().remove("verification");
    let error = bloch::outcome::OutcomeRecord::from_intake_json(&object).unwrap_err();
    assert!(error.starts_with("missing_verification"), "{error}");
}
