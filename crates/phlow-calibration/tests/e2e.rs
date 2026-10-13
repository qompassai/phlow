//! End-to-end against a real gauntlet corpus on disk.
//!
//! Ignored by default: it needs `PHLOW_CALIBRATION_CORPUS` set to
//! a real decision-traces.jsonl produced by the gauntlet's
//! TraceRecorder (on primo: `~/workspace/gauntlet-corpus-20261013/
//! decision-traces.jsonl` and the 2026-10-11 corpus beside it).
//! Run with: `cargo test -p phlow-calibration --test e2e --
//! --ignored --nocapture`.

use std::path::PathBuf;

use phlow_calibration::{CollectionPaths, collect_all, compute_split_metrics, render_report};

#[test]
#[ignore = "requires PHLOW_CALIBRATION_CORPUS pointing at a real trace corpus"]
fn real_corpus_end_to_end() {
    let corpus = PathBuf::from(
        std::env::var("PHLOW_CALIBRATION_CORPUS")
            .expect("PHLOW_CALIBRATION_CORPUS names the corpus file"),
    );
    let dir = corpus
        .parent()
        .expect("corpus has a parent dir")
        .to_path_buf();
    let paths = CollectionPaths {
        approval_store: dir.join("consumed-approvals.jsonl"),
        feedback_db: dir.join("feedback.db"),
        trace_file: Some(corpus),
    };
    let collection = collect_all(&paths).expect("the real corpus collects");
    let gauntlet = collection.gauntlet.as_ref().expect("gauntlet collected");
    assert!(
        gauntlet.trace_records >= 100,
        "expected a real corpus, got {} traces",
        gauntlet.trace_records
    );
    assert_eq!(gauntlet.outcomes.len(), 68);
    let split = compute_split_metrics(collection.tuples(), 10);
    let report = render_report(&collection, &split);
    println!("{report}");
    // Today's corpora are label-pending by design, so the honest
    // verdict is INSUFFICIENT DATA; if a future corpus carries
    // labels, this assertion is the one to revisit first.
    assert!(report.contains("INSUFFICIENT DATA"), "report:\n{report}");
}
