//! Integration tests for the PH-2 / PH-5 proposal machinery.
//!
//! Half validation (the machinery produces correct proposals from a
//! real store) and half adversarial (cutting budgets, unlabelled
//! calibration rows, and disagreeing manifests are refused, never
//! repaired by default).

use std::path::{Path, PathBuf};

use phlow_self_improve::budget::{
    BudgetProposal, ClassFit, Margin, Split, Truth, fit_budgets, join_labelled, load_corpus_labels,
    load_outcome_rows, validate_budgets,
};
use phlow_self_improve::reviewer_cal::{CalibrationVerdict, reliability_table};
use phlow_self_improve::{OutcomeStore, RunOutcome, VerificationStrength};

/// A unique temp directory for one test.
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("phlow-proposals-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir creates");
    dir
}

/// One synthetic outcome row for the store.
fn outcome(
    run_id: &str,
    verdict: Option<&str>,
    cycles: u64,
    tool_calls: u64,
    model_calls: u64,
    wall_ms: u64,
    strength: VerificationStrength,
) -> RunOutcome {
    RunOutcome {
        run_id: run_id.to_owned(),
        task_class: "flow_run".to_owned(),
        plan: "test plan".to_owned(),
        checks_json: "[{\"name\":\"theodolite_zero\",\"status\":\"ok\"}]".to_owned(),
        reviewer_verdict: verdict.map(str::to_owned),
        cycles,
        tool_calls,
        model_calls,
        context_chars: None,
        wall_ms,
        model_ids: vec!["rev-model".to_owned()],
        status: "ok".to_owned(),
        verification_strength: strength,
        origin_lane: "phlow".to_owned(),
        content_label: "nsfw".to_owned(),
        evidence: "integration test".to_owned(),
    }
}

/// Write a labels manifest and return its path.
fn write_labels(dir: &Path, labels_json: &str) -> PathBuf {
    let path = dir.join("labels.json");
    std::fs::write(&path, labels_json).expect("labels write");
    path
}

/// Seed a store with the given outcomes and return its path.
fn seed_store(dir: &Path, outcomes: &[RunOutcome]) -> PathBuf {
    let db = dir.join("feedback.db");
    let store = OutcomeStore::open(&db).expect("store opens");
    for outcome in outcomes {
        store.record(outcome).expect("row records");
    }
    drop(store);
    db
}

#[test]
fn end_to_end_fit_from_a_real_store() {
    let dir = temp_dir("e2e");
    // Four fit-split successes (tool calls 10..13) + one held-out
    // success (12) + one held-out failure (200 tool calls).
    let mut outcomes = Vec::new();
    for i in 0..4 {
        outcomes.push(outcome(
            &format!("fit-{i}"),
            Some("approved"),
            1,
            10 + i,
            6,
            60_000,
            VerificationStrength::ToolChecked,
        ));
    }
    outcomes.push(outcome(
        "held-ok",
        Some("approved"),
        1,
        12,
        6,
        60_000,
        VerificationStrength::ToolChecked,
    ));
    outcomes.push(outcome(
        "held-bad",
        Some("not_approved"),
        2,
        200,
        40,
        600_000,
        VerificationStrength::SelfReported,
    ));
    let db = seed_store(&dir, &outcomes);
    let mut labels = String::from("{\"labels\":[");
    for i in 0..4 {
        labels.push_str(&format!(
            "{{\"run_id\":\"fit-{i}\",\"task_class\":\"analysis\",\"split\":\"fit\",\
             \"truth\":\"success\",\"reviewer_model\":\"rev-model\"}},"
        ));
    }
    labels.push_str(
        "{\"run_id\":\"held-ok\",\"task_class\":\"analysis\",\"split\":\"held_out\",\
         \"truth\":\"success\",\"reviewer_model\":\"rev-model\"},\
         {\"run_id\":\"held-bad\",\"task_class\":\"analysis\",\"split\":\"held_out\",\
         \"truth\":\"failure\",\"reviewer_model\":\"rev-model\"}]}",
    );
    let labels_path = write_labels(&dir, &labels);
    let rows = load_outcome_rows(&db).expect("rows load read-only");
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[0].checks[0].0, "theodolite_zero");
    let labels = load_corpus_labels(&labels_path).expect("labels load");
    let (runs, unlabelled) = join_labelled(&rows, &labels).expect("join");
    assert_eq!(unlabelled, 0);
    let fits = fit_budgets(&runs, Margin::DEFAULT);
    let proposal = fits[0].proposal.as_ref().expect("proposal fitted");
    assert_eq!(proposal.max_tool_calls, 17); // p95 of [10,11,12,13] = 13, x1.25 -> 17
    validate_budgets(&runs, &fits).expect("gate holds");
    let table = reliability_table(&runs);
    assert_eq!(table[0].model, "rev-model");
    assert_eq!(table[0].scored, 6);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn escalation_is_disclosed_and_covers_the_held_out_maximum() {
    let dir = temp_dir("escalation");
    let mut outcomes = Vec::new();
    for i in 0..3 {
        outcomes.push(outcome(
            &format!("fit-{i}"),
            Some("approved"),
            1,
            8,
            5,
            30_000,
            VerificationStrength::ToolChecked,
        ));
    }
    // Held-out success far above the fit distribution.
    outcomes.push(outcome(
        "held-big",
        Some("approved"),
        1,
        40,
        5,
        30_000,
        VerificationStrength::ToolChecked,
    ));
    let db = seed_store(&dir, &outcomes);
    let labels = "{\"labels\":[
        {\"run_id\":\"fit-0\",\"task_class\":\"analysis\",\"split\":\"fit\",\"truth\":\"success\"},
        {\"run_id\":\"fit-1\",\"task_class\":\"analysis\",\"split\":\"fit\",\"truth\":\"success\"},
        {\"run_id\":\"fit-2\",\"task_class\":\"analysis\",\"split\":\"fit\",\"truth\":\"success\"},
        {\"run_id\":\"held-big\",\"task_class\":\"analysis\",\"split\":\"held_out\",\"truth\":\"success\"}
    ]}";
    let labels_path = write_labels(&dir, labels);
    let rows = load_outcome_rows(&db).expect("rows load");
    let labels = load_corpus_labels(&labels_path).expect("labels load");
    let (runs, _) = join_labelled(&rows, &labels).expect("join");
    let fits = fit_budgets(&runs, Margin::DEFAULT);
    let proposal = fits[0].proposal.as_ref().expect("proposal fitted");
    assert!(proposal.escalated);
    assert_eq!(proposal.initially_cut, 1);
    // Raised to held-out max 40 x 1.25 = 50.
    assert_eq!(proposal.max_tool_calls, 50);
    validate_budgets(&runs, &fits).expect("raised budgets pass the gate");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- adversarial ----

#[test]
fn validator_refuses_a_budget_that_cuts_a_held_out_success() {
    let dir = temp_dir("cutting");
    let outcomes = vec![outcome(
        "held-ok",
        Some("approved"),
        2,
        90,
        30,
        120_000,
        VerificationStrength::ToolChecked,
    )];
    let db = seed_store(&dir, &outcomes);
    let labels = "{\"labels\":[
        {\"run_id\":\"held-ok\",\"task_class\":\"analysis\",\"split\":\"held_out\",\"truth\":\"success\"}
    ]}";
    let labels_path = write_labels(&dir, labels);
    let rows = load_outcome_rows(&db).expect("rows load");
    let labels = load_corpus_labels(&labels_path).expect("labels load");
    let (runs, _) = join_labelled(&rows, &labels).expect("join");
    let fits = vec![ClassFit {
        task_class: "analysis".to_owned(),
        runs: 1,
        fit_runs: 0,
        held_out_runs: 1,
        proposal: Some(BudgetProposal {
            task_class: "analysis".to_owned(),
            max_iterations: 40,
            max_cycles: 4,
            max_tool_calls: 10, // cuts the run's 90
            timeout_ms: 300_000,
            fit_successes: 4,
            escalated: false,
            initially_cut: 0,
        }),
        no_proposal_reason: None,
    }];
    let error = validate_budgets(&runs, &fits).expect_err("cutting budget refused");
    assert!(error.to_string().contains("would cut"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn validator_refuses_a_zero_dimension_budget() {
    let fits = vec![ClassFit {
        task_class: "analysis".to_owned(),
        runs: 0,
        fit_runs: 0,
        held_out_runs: 0,
        proposal: Some(BudgetProposal {
            task_class: "analysis".to_owned(),
            max_iterations: 10,
            max_cycles: 0,
            max_tool_calls: 10,
            timeout_ms: 1000,
            fit_successes: 4,
            escalated: false,
            initially_cut: 0,
        }),
        no_proposal_reason: None,
    }];
    assert!(validate_budgets(&[], &fits).is_err());
}

#[test]
fn label_without_ground_truth_is_refused() {
    let dir = temp_dir("notruth");
    let labels = "{\"labels\":[
        {\"run_id\":\"run-1\",\"task_class\":\"analysis\",\"split\":\"fit\"}
    ]}";
    let labels_path = write_labels(&dir, labels);
    let error = load_corpus_labels(&labels_path).expect_err("unlabelled row refused");
    assert!(error.to_string().contains("no ground truth"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn duplicate_label_is_refused() {
    let dir = temp_dir("dup");
    let labels = "{\"labels\":[
        {\"run_id\":\"run-1\",\"task_class\":\"analysis\",\"split\":\"fit\",\"truth\":\"success\"},
        {\"run_id\":\"run-1\",\"task_class\":\"analysis\",\"split\":\"fit\",\"truth\":\"failure\"}
    ]}";
    let labels_path = write_labels(&dir, labels);
    assert!(load_corpus_labels(&labels_path).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn label_for_an_unknown_run_is_refused_at_the_join() {
    let dir = temp_dir("unknown");
    let db = seed_store(
        &dir,
        &[outcome(
            "real-run",
            Some("approved"),
            1,
            5,
            4,
            1000,
            VerificationStrength::ToolChecked,
        )],
    );
    let labels = "{\"labels\":[
        {\"run_id\":\"ghost-run\",\"task_class\":\"analysis\",\"split\":\"fit\",\"truth\":\"success\"}
    ]}";
    let labels_path = write_labels(&dir, labels);
    let rows = load_outcome_rows(&db).expect("rows load");
    let labels = load_corpus_labels(&labels_path).expect("labels load");
    assert!(join_labelled(&rows, &labels).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn runs_without_a_reviewer_model_are_unattributed_not_scored() {
    let dir = temp_dir("unattributed");
    let db = seed_store(
        &dir,
        &[outcome(
            "run-1",
            Some("approved"),
            1,
            5,
            4,
            1000,
            VerificationStrength::ToolChecked,
        )],
    );
    let labels = "{\"labels\":[
        {\"run_id\":\"run-1\",\"task_class\":\"analysis\",\"split\":\"fit\",\"truth\":\"success\"}
    ]}";
    let labels_path = write_labels(&dir, labels);
    let rows = load_outcome_rows(&db).expect("rows load");
    let labels = load_corpus_labels(&labels_path).expect("labels load");
    let (runs, _) = join_labelled(&rows, &labels).expect("join");
    assert!(reliability_table(&runs).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn split_and_truth_wire_strings_are_exact() {
    assert_eq!(Split::parse("fit"), Some(Split::Fit));
    assert_eq!(Split::parse("held_out"), Some(Split::HeldOut));
    assert_eq!(Split::parse("heldout"), None);
    assert_eq!(Truth::parse("success"), Some(Truth::Success));
    assert_eq!(Truth::parse("ok"), None);
    // The three calibration verdicts are distinct states.
    assert_ne!(
        CalibrationVerdict::Calibrated,
        CalibrationVerdict::DemoteToAdvisory
    );
    assert_ne!(
        CalibrationVerdict::DemoteToAdvisory,
        CalibrationVerdict::InsufficientEvidence
    );
}
