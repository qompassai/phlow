//! Dataset export: the design's weights, the held-out reserve,
//! and the no-fabricated-labels rule.

mod common;

use common::{choice_tuple, noul_tuple, temp_path};
use phlow_calibration::{GroundTruth, TupleSource, export_rows, write_jsonl};

#[test]
fn weights_follow_the_design_by_source() {
    let mut gauntlet = noul_tuple(1, false, 0.9, true, Some(true));
    let mut approval = gauntlet.clone();
    approval.source = TupleSource::Approval {
        approval_id: "appr-1".to_string(),
    };
    let mut feedback = gauntlet.clone();
    feedback.source = TupleSource::Feedback {
        session_id: "session-a".to_string(),
    };
    gauntlet.source = TupleSource::Gauntlet {
        task_id: "task-254".to_string(),
    };
    let rows = export_rows(&[gauntlet, approval, feedback]);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].weight, 1.0);
    assert_eq!(rows[1].weight, 2.0);
    assert_eq!(rows[2].weight, 0.5);
}

#[test]
fn unlabeled_tuples_are_never_exported() {
    // An unlabeled tuple has no correct_answer; exporting one
    // would fabricate the label it claims to teach from.
    let tuples = vec![noul_tuple(1, false, 0.9, true, None)];
    assert!(export_rows(&tuples).is_empty());
}

#[test]
fn held_out_gauntlet_tuples_stay_in_the_reserve() {
    let tuples = vec![
        noul_tuple(1, true, 0.9, true, Some(true)),
        noul_tuple(2, false, 0.9, true, Some(true)),
    ];
    let rows = export_rows(&tuples);
    assert_eq!(rows.len(), 1);
}

#[test]
fn held_out_flag_alone_does_not_exclude_non_gauntlet_sources() {
    let mut tuple = noul_tuple(1, false, 0.9, true, Some(true));
    tuple.source = TupleSource::Approval {
        approval_id: "appr-1".to_string(),
    };
    tuple.held_out = true;
    let rows = export_rows(&[tuple]);
    assert_eq!(rows.len(), 1);
}

#[test]
fn jsonl_round_trip_keeps_the_design_shape() {
    let tuples = vec![
        noul_tuple(1, false, 0.9, true, Some(true)),
        choice_tuple(2, false, 0.8, 0, Some(1)),
    ];
    let rows = export_rows(&tuples);
    let path = temp_path("dataset.jsonl");
    write_jsonl(&rows, &path).expect("dataset writes");
    let text = std::fs::read_to_string(&path).expect("dataset reads back");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    for line in &lines {
        let value: serde_json::Value = serde_json::from_str(line).expect("row is JSON");
        let object = value.as_object().expect("row is an object");
        let keys: Vec<&str> = object.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["correct_answer", "question", "state", "weight"]);
    }
    let first: serde_json::Value = serde_json::from_str(lines[0]).expect("first row parses");
    assert_eq!(first["correct_answer"]["type"], "Noul");
    assert_eq!(first["correct_answer"]["actually_yes"], true);
    assert_eq!(first["weight"], 1.0);
    let second: serde_json::Value = serde_json::from_str(lines[1]).expect("second row parses");
    assert_eq!(second["correct_answer"]["type"], "Choice");
    assert_eq!(second["correct_answer"]["correct_index"], 1);
}

#[test]
fn empty_export_writes_an_empty_file() {
    let path = temp_path("dataset.jsonl");
    write_jsonl(&[], &path).expect("empty export writes");
    let text = std::fs::read_to_string(&path).expect("reads back");
    assert!(text.is_empty());
}

#[test]
fn tuple_from_each_kind_exports_its_own_truth_shape() {
    let mut score = common::score_tuple(3, false, 0.5, 0.9, Some(0.5));
    score.ground_truth = Some(GroundTruth::Score { true_value: 0.5 });
    let rows = export_rows(&[score]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].correct_answer["type"], "Score");
    assert_eq!(rows[0].correct_answer["true_value"], 0.5);
}
