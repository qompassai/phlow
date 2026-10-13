//! Collector: real corpus lines parse; every corrupt or lying
//! variant is rejected whole; absent stores stay absent.

mod common;

use common::{
    CHOICE_LINE, NOUL_LINE, OUTCOME_LINE, SCORE_LINE, labeled_noul_line, temp_path, write_temp,
};
use phlow_calibration::{
    CalibrationError, CollectionPaths, QuestionKind, SourceState, collect_all,
    collect_approval_store, collect_feedback, collect_gauntlet_traces,
};
use phlow_self_improve::FeedbackStore;

#[test]
fn real_corpus_lines_parse_into_tuples_and_outcomes() {
    let text = format!("{CHOICE_LINE}\n{NOUL_LINE}\n{SCORE_LINE}\n{OUTCOME_LINE}\n");
    let path = write_temp("traces", &text);
    let collection = collect_gauntlet_traces(&path).expect("real lines parse");
    assert_eq!(collection.trace_records, 3);
    assert_eq!(collection.outcome_records, 1);
    assert_eq!(collection.tuples.len(), 3);
    assert_eq!(collection.tuples[0].kind(), QuestionKind::Choice);
    assert_eq!(collection.tuples[1].kind(), QuestionKind::Noul);
    assert_eq!(collection.tuples[2].kind(), QuestionKind::Score);
    // The Noul trace's recorded distribution survives verbatim.
    let dist = collection.tuples[1]
        .predicted_dist
        .as_ref()
        .expect("recorded distribution");
    assert_eq!(dist.len(), 2);
    assert!((dist[1] - 0.99).abs() < 1e-15);
    // Scalar-backend Choice/Score traces honestly carry none.
    assert!(collection.tuples[0].predicted_dist.is_none());
    assert!(collection.tuples[2].predicted_dist.is_none());
    // All three real traces are label-pending, as captured.
    assert!(collection.tuples.iter().all(|t| t.ground_truth.is_none()));
    let outcome = collection.outcomes.get("task-251").expect("outcome joins");
    assert!(outcome.passed);
    assert!(outcome.held_out);
}

#[test]
fn empty_trace_file_collects_empty() {
    let path = write_temp("traces", "");
    let collection = collect_gauntlet_traces(&path).expect("empty is valid");
    assert_eq!(collection.tuples.len(), 0);
    assert_eq!(collection.outcomes.len(), 0);
}

#[test]
fn blank_lines_are_skipped() {
    let path = write_temp("traces", &format!("\n{NOUL_LINE}\n\n"));
    let collection = collect_gauntlet_traces(&path).expect("blanks skipped");
    assert_eq!(collection.tuples.len(), 1);
}

#[test]
fn labeled_trace_line_carries_its_truth() {
    let line = labeled_noul_line("task-fixture-labeled", 0.9, true, true);
    let path = write_temp("traces", &format!("{line}\n"));
    let collection = collect_gauntlet_traces(&path).expect("labeled line parses");
    assert_eq!(collection.tuples.len(), 1);
    assert!(collection.tuples[0].ground_truth.is_some());
    let (confidence, correct) = collection.tuples[0]
        .scored_outcome()
        .expect("labeled tuple scores");
    assert!((confidence - 0.9).abs() < 1e-12);
    assert!(correct);
}

#[test]
fn corrupt_json_line_rejects_the_file_with_its_line_number() {
    let path = write_temp("traces", &format!("{NOUL_LINE}\n{{not json}}\n"));
    let error = collect_gauntlet_traces(&path).expect_err("corrupt line rejected");
    match error {
        CalibrationError::MalformedLine { line, .. } => assert_eq!(line, 2),
        other => panic!("expected MalformedLine, got {other}"),
    }
}

#[test]
fn unknown_record_kind_is_rejected_not_skipped() {
    let line = r#"{"kind":"mystery","task_id":"task-1"}"#;
    let path = write_temp("traces", &format!("{line}\n"));
    let error = collect_gauntlet_traces(&path).expect_err("unknown kind rejected");
    match error {
        CalibrationError::UnknownRecordKind { kind, .. } => assert_eq!(kind, "mystery"),
        other => panic!("expected UnknownRecordKind, got {other}"),
    }
}

#[test]
fn record_without_kind_is_rejected() {
    let path = write_temp("traces", "{\"task_id\":\"task-1\"}\n");
    let error = collect_gauntlet_traces(&path).expect_err("kindless record rejected");
    assert!(matches!(
        error,
        CalibrationError::MalformedLine { line: 1, .. }
    ));
}

#[test]
fn ground_truth_of_the_wrong_kind_is_rejected() {
    let line = CHOICE_LINE.replace(
        "\"ground_truth\":null",
        "\"ground_truth\":{\"actually_yes\":true,\"type\":\"Noul\"}",
    );
    let path = write_temp("traces", &format!("{line}\n"));
    assert!(collect_gauntlet_traces(&path).is_err());
}

#[test]
fn distribution_that_does_not_sum_to_one_is_rejected() {
    let line = NOUL_LINE.replace("[0.010000000000000009,0.99]", "[0.5,0.6]");
    let path = write_temp("traces", &format!("{line}\n"));
    assert!(collect_gauntlet_traces(&path).is_err());
}

#[test]
fn distribution_of_the_wrong_length_is_rejected() {
    let line = NOUL_LINE.replace("[0.010000000000000009,0.99]", "[0.2,0.3,0.5]");
    let path = write_temp("traces", &format!("{line}\n"));
    assert!(collect_gauntlet_traces(&path).is_err());
}

#[test]
fn lying_held_out_stamp_on_a_trace_is_rejected() {
    // task-254 is not held out; a record claiming otherwise does
    // not match the task-ID split the recorder stamps from.
    let line = CHOICE_LINE.replace("\"held_out\":false", "\"held_out\":true");
    let path = write_temp("traces", &format!("{line}\n"));
    assert!(collect_gauntlet_traces(&path).is_err());
}

#[test]
fn lying_held_out_stamp_on_an_outcome_is_rejected() {
    let line = r#"{"held_out":true,"kind":"task_outcome","passed":true,"recorded_at_unix_ms":1791882433215,"task_id":"task-254"}"#;
    let path = write_temp("traces", &format!("{line}\n"));
    assert!(collect_gauntlet_traces(&path).is_err());
}

#[test]
fn duplicate_outcome_records_last_wins_and_are_counted() {
    let second = r#"{"held_out":true,"kind":"task_outcome","passed":false,"recorded_at_unix_ms":1791882433999,"task_id":"task-251"}"#;
    let path = write_temp("traces", &format!("{OUTCOME_LINE}\n{second}\n"));
    let collection = collect_gauntlet_traces(&path).expect("duplicates tolerated");
    assert_eq!(collection.outcome_records, 2);
    assert_eq!(collection.duplicate_outcomes, 1);
    assert_eq!(collection.outcomes.len(), 1);
    assert!(!collection.outcomes["task-251"].passed);
}

#[test]
fn oversized_line_is_rejected() {
    let big = "a".repeat(70_000);
    let path = write_temp("traces", &format!("{big}\n"));
    let error = collect_gauntlet_traces(&path).expect_err("oversized line rejected");
    assert!(matches!(error, CalibrationError::LineTooLong { .. }));
}

#[test]
fn missing_trace_file_is_an_io_error_not_an_empty_collection() {
    let path = temp_path("traces-missing");
    let error = collect_gauntlet_traces(&path).expect_err("missing file errors");
    assert!(matches!(error, CalibrationError::Io { .. }));
}

#[test]
fn feedback_absent_stays_absent() {
    let path = temp_path("feedback.db");
    let collection = collect_feedback(&path).expect("absent is a state");
    assert_eq!(collection.state, SourceState::Absent);
    assert!(collection.entries.is_empty());
    assert!(collection.average_rating.is_none());
    // Collecting must not create the store as a side effect.
    assert!(!path.exists());
}

#[test]
fn feedback_present_reads_real_rows() {
    let path = temp_path("feedback.db");
    let store = FeedbackStore::open(&path).expect("store opens");
    store
        .record("session-a", 2, "wrong file edited", "prompt", "failed")
        .expect("row records");
    store
        .record("session-b", 5, "good", "prompt", "passed")
        .expect("row records");
    drop(store);
    let collection = collect_feedback(&path).expect("present store reads");
    assert_eq!(collection.state, SourceState::Present);
    assert_eq!(collection.entries.len(), 2);
    let average = collection.average_rating.expect("rows exist");
    assert!((average - 3.5).abs() < 1e-12);
}

#[test]
fn approval_absent_is_a_state() {
    let path = temp_path("consumed-approvals.jsonl");
    let collection = collect_approval_store(&path).expect("absent is a state");
    assert_eq!(collection.state, SourceState::Absent);
    assert!(collection.consumed_ids.is_empty());
    assert!(!path.exists());
}

#[test]
fn approval_present_reads_ids_in_order() {
    let text = "{\"id\":\"appr-1\",\"expires_ms\":1791882433215}\n{\"id\":\"appr-2\",\"expires_ms\":1791882433999}\n";
    let path = write_temp("approvals", text);
    let collection = collect_approval_store(&path).expect("store reads");
    assert_eq!(collection.state, SourceState::Present);
    assert_eq!(collection.consumed_ids, vec!["appr-1", "appr-2"]);
}

#[test]
fn approval_duplicate_id_is_corrupt() {
    let text = "{\"id\":\"appr-1\",\"expires_ms\":1}\n{\"id\":\"appr-1\",\"expires_ms\":2}\n";
    let path = write_temp("approvals", text);
    let error = collect_approval_store(&path).expect_err("duplicate rejected");
    assert!(matches!(error, CalibrationError::ApprovalStore { .. }));
}

#[test]
fn approval_malformed_line_is_corrupt() {
    let path = write_temp("approvals", "{\"id\":\"appr-1\"}\n");
    let error = collect_approval_store(&path).expect_err("missing expires_ms rejected");
    assert!(matches!(error, CalibrationError::ApprovalStore { .. }));
}

#[test]
fn approval_oversized_id_is_corrupt() {
    let id = "x".repeat(65);
    let path = write_temp(
        "approvals",
        &format!("{{\"id\":\"{id}\",\"expires_ms\":1}}\n"),
    );
    let error = collect_approval_store(&path).expect_err("oversized id rejected");
    assert!(matches!(error, CalibrationError::ApprovalStore { .. }));
}

#[test]
fn approval_store_over_the_size_bound_is_not_read() {
    let big = "x".repeat(1_048_577);
    let path = write_temp("approvals", &big);
    let error = collect_approval_store(&path).expect_err("oversized store rejected");
    assert!(matches!(error, CalibrationError::FileTooLarge { .. }));
}

#[test]
fn collect_all_without_a_trace_file_has_no_tuples() {
    let paths = CollectionPaths {
        approval_store: temp_path("consumed-approvals.jsonl"),
        feedback_db: temp_path("feedback.db"),
        trace_file: None,
    };
    let collection = collect_all(&paths).expect("collects");
    assert!(collection.gauntlet.is_none());
    assert!(collection.tuples().is_empty());
    assert_eq!(collection.feedback.state, SourceState::Absent);
    assert_eq!(collection.approvals.state, SourceState::Absent);
}
