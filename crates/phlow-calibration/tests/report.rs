//! Report: the rendered text carries the source states and the
//! verdict the metrics imply — including the honest one.

mod common;

use common::{CHOICE_LINE, NOUL_LINE, OUTCOME_LINE, SCORE_LINE, noul_tuple, temp_path, write_temp};
use phlow_calibration::{
    Collection, CollectionPaths, collect_all, compute_split_metrics, render_report,
};

/// Collect the real corpus lines against absent stores.
fn real_collection() -> Collection {
    let trace_path = write_temp(
        "traces",
        &format!("{CHOICE_LINE}\n{NOUL_LINE}\n{SCORE_LINE}\n{OUTCOME_LINE}\n"),
    );
    let paths = CollectionPaths {
        approval_store: temp_path("consumed-approvals.jsonl"),
        feedback_db: temp_path("feedback.db"),
        trace_file: Some(trace_path),
    };
    collect_all(&paths).expect("real collection collects")
}

#[test]
fn report_on_real_corpus_is_insufficient_data_and_says_why() {
    let collection = real_collection();
    let split = compute_split_metrics(collection.tuples(), 10);
    let report = render_report(&collection, &split);
    assert!(report.contains("INSUFFICIENT DATA"), "report:\n{report}");
    assert!(report.contains("3 decision traces"), "report:\n{report}");
    assert!(report.contains("0 labeled, 3 pending"), "report:\n{report}");
    assert!(
        report.contains("feedback store: ABSENT"),
        "report:\n{report}"
    );
    assert!(
        report.contains("consumed-approvals store: ABSENT"),
        "report:\n{report}"
    );
    assert!(
        report.contains("approval decision history: persisted nowhere"),
        "report:\n{report}"
    );
    assert!(
        report.contains("brier: no labeled tuples"),
        "report:\n{report}"
    );
}

#[test]
fn report_renders_curves_for_labeled_kinds() {
    let tuples: Vec<_> = (0..10)
        .map(|seed| noul_tuple(400 + seed, true, 0.9, true, Some(true)))
        .collect();
    let split = compute_split_metrics(&tuples, 10);
    let collection = Collection {
        approvals: phlow_calibration::ApprovalCollection {
            consumed_ids: Vec::new(),
            state: phlow_calibration::SourceState::Absent,
        },
        feedback: phlow_calibration::FeedbackCollection {
            average_rating: None,
            entries: Vec::new(),
            state: phlow_calibration::SourceState::Absent,
        },
        gauntlet: None,
    };
    let report = render_report(&collection, &split);
    assert!(report.contains("### noul"), "report:\n{report}");
    assert!(
        report.contains("[0.9, 1.0): 10, 0.900, 1.000"),
        "report:\n{report}"
    );
}

#[test]
fn report_verdict_not_calibrated_for_confident_wrong_reserve() {
    let tuples: Vec<_> = (0..10)
        .map(|seed| noul_tuple(500 + seed, true, 0.99, true, Some(false)))
        .collect();
    let split = compute_split_metrics(&tuples, 10);
    let collection = real_collection();
    let report = render_report(&collection, &split);
    assert!(report.contains("NOT CALIBRATED"), "report:\n{report}");
    assert!(report.contains("brier: 0.9801"), "report:\n{report}");
}
