//! Weighted fine-tuning dataset export (design Phase B).
//!
//! The export is the design's JSONL shape — one
//! `{"state", "question", "correct_answer", "weight"}` object per
//! line — with two honesty rules the design's own text implies:
//!
//! - **Labeled tuples only.** A tuple whose ground truth is still
//!   pending has no `correct_answer` to teach from; exporting it
//!   would fabricate a label.
//! - **Held-out gauntlet tuples are excluded.** The design reserves
//!   them for validation ("never trained on"); the reserve is kept
//!   even though metrics may read them.
//!
//! `correct_answer` uses the ground-truth wire shape the trace
//! corpus itself uses (`{"type": "Choice", "correct_index": n}`,
//! `{"type": "Noul", "actually_yes": b}`, `{"type": "Score",
//! "true_value": v}`) — it is the realized label, not a prediction.

use std::path::Path;

use serde_json::{Map, Value};

use crate::CalibrationError;
use crate::tuple::{CalibrationTuple, GroundTruth};

/// Largest dataset one export produces. Real calibration corpora
/// are thousands of rows; the bound keeps a pathological input
/// from producing an unbounded file.
pub const DATASET_ROWS_MAX: usize = 100_000;

/// One export row, pre-serialization.
#[derive(Debug, Clone, PartialEq)]
pub struct DatasetRow {
    /// The realized label, in ground-truth wire shape.
    pub correct_answer: Value,
    /// The question, serialized in the protocol's wire shape.
    pub question: Value,
    /// The state string the decision was made from.
    pub state: String,
    /// The source weight (design Phase B).
    pub weight: f64,
}

/// Build the export rows from collected tuples: labeled, and not
/// held-out gauntlet tuples.
///
/// # Panics
///
/// Panics if more than [`DATASET_ROWS_MAX`] tuples qualify; the
/// tuple set is program-produced, not raw external data, so an
/// overflow is a caller bug, not a data condition.
pub fn export_rows(tuples: &[CalibrationTuple]) -> Vec<DatasetRow> {
    let rows: Vec<DatasetRow> = tuples
        .iter()
        .filter(|tuple| tuple.ground_truth.is_some())
        .filter(|tuple| !(tuple.held_out && tuple.source.gauntlet_task_id().is_some()))
        .map(|tuple| DatasetRow {
            correct_answer: correct_answer_json(tuple),
            question: serde_json::to_value(&tuple.question)
                .expect("Question serialization is infallible"),
            state: tuple.state.clone(),
            weight: tuple.source.weight(),
        })
        .collect();
    assert!(
        rows.len() <= DATASET_ROWS_MAX,
        "dataset export exceeds DATASET_ROWS_MAX"
    );
    rows
}

/// Write rows as JSONL to `path`, atomically: a sibling temp file
/// is written first and renamed over the target, so a reader never
/// sees a half-written dataset. Parent directories are created.
pub fn write_jsonl(rows: &[DatasetRow], path: &Path) -> Result<(), CalibrationError> {
    let io_error = |error: std::io::Error| CalibrationError::Io {
        path: path.to_path_buf(),
        reason: error.to_string(),
    };
    let mut text = String::new();
    for row in rows {
        text.push_str(&row_json(row));
        text.push('\n');
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(io_error)?;
    }
    let temp_path = path.with_extension("jsonl.tmp");
    std::fs::write(&temp_path, text).map_err(io_error)?;
    std::fs::rename(&temp_path, path).map_err(io_error)?;
    Ok(())
}

/// One row as a JSON object string, keys alphabetical.
fn row_json(row: &DatasetRow) -> String {
    let mut object = Map::new();
    object.insert("correct_answer".to_string(), row.correct_answer.clone());
    object.insert("question".to_string(), row.question.clone());
    object.insert("state".to_string(), Value::String(row.state.clone()));
    object.insert("weight".to_string(), serde_json::json!(row.weight));
    Value::Object(object).to_string()
}

/// The realized label of a labeled tuple, in the trace corpus's
/// ground-truth wire shape.
fn correct_answer_json(tuple: &CalibrationTuple) -> Value {
    let mut object = Map::new();
    match tuple.ground_truth {
        Some(GroundTruth::Choice { correct_index }) => {
            object.insert(
                "correct_index".to_string(),
                serde_json::json!(correct_index),
            );
            object.insert("type".to_string(), Value::String("Choice".to_string()));
        }
        Some(GroundTruth::Noul { actually_yes }) => {
            object.insert("actually_yes".to_string(), Value::Bool(actually_yes));
            object.insert("type".to_string(), Value::String("Noul".to_string()));
        }
        Some(GroundTruth::Score { true_value }) => {
            object.insert("true_value".to_string(), serde_json::json!(true_value));
            object.insert("type".to_string(), Value::String("Score".to_string()));
        }
        None => unreachable!("export_rows filters unlabeled tuples first"),
    }
    Value::Object(object)
}
