//! The plan-versus-actual divergence record (PH-6).
//!
//! Beside every PH-1 outcome row, the runtime records how the run
//! diverged from its own plan: which planned steps were executed,
//! which were skipped, and what work was added that the plan never
//! named. The derivation is a pure function of the run's report —
//! the plan text (the planner contract's Goal / Files / Steps /
//! Checks skeleton), the host-observed `changed_files`, and the
//! cycle counter. It invents nothing: a step is *executed* only when
//! a file the step names was actually changed, *skipped* only when
//! it names files and none of them changed, and *untracked* when it
//! names no file at all (no file evidence exists either way, and the
//! record says so instead of guessing).
//!
//! Records live in the **same SQLite file** as the outcome store
//! (one store file per phlow home), in their own append-only table:
//! `run_id` is `UNIQUE` (one run, one record) and SQLite triggers
//! refuse UPDATE/DELETE through any connection. Lane law (Bloch):
//! every record carries the run's `origin_lane` and `content_label`
//! from birth, with the outcome store's fail-closed defaults.
//!
//! # Bounds
//!
//! - [`PLANNED_STEPS_MAX`], [`STEP_TEXT_CHARS_MAX`],
//!   [`DIVERGENCE_FILES_MAX`], [`DIVERGENCE_JSON_BYTES_MAX`],
//!   [`DIVERGENCE_LIST_LIMIT_MAX`]. Counts and stored lists are
//!   capped at the named bounds; a plan beyond them is recorded up
//!   to the cap, never beyond it.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, ErrorCode, params};
use serde_json::Value;

use crate::error::SelfImproveError;
use crate::outcome::{
    LANE_LABEL_CHARS_MAX, RUN_ID_CHARS_MAX, TASK_CLASS_CHARS_MAX, UNCLASSIFIED_CONTENT_LABEL,
    UNKNOWN_ORIGIN_LANE, check_chars, utc_now_iso,
};

/// Most planned steps one record itemizes.
pub const PLANNED_STEPS_MAX: usize = 64;
/// Longest stored step text, in characters; longer texts are cut at
/// a char boundary and marked with an ellipsis, never silently.
pub const STEP_TEXT_CHARS_MAX: usize = 512;
/// Most file paths one stored list (untouched / added) may hold.
/// The list caps compose: the largest list (64 entries of at most
/// ~515 serialized chars) stays under [`DIVERGENCE_JSON_BYTES_MAX`].
pub const DIVERGENCE_FILES_MAX: usize = 64;
/// A serialized divergence list may not exceed this many bytes.
pub const DIVERGENCE_JSON_BYTES_MAX: usize = 65_536;
/// `list_for_class` never returns more than this many rows.
pub const DIVERGENCE_LIST_LIMIT_MAX: u32 = 1_000;

const DIVERGENCES_DDL: &str = "CREATE TABLE IF NOT EXISTS plan_divergences (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    run_id TEXT NOT NULL UNIQUE,
                    task_class TEXT NOT NULL,
                    planned_steps INTEGER NOT NULL,
                    executed_steps INTEGER NOT NULL,
                    skipped_steps_json TEXT NOT NULL,
                    untracked_steps INTEGER NOT NULL,
                    planned_files_untouched_json TEXT NOT NULL,
                    added_files_json TEXT NOT NULL,
                    repair_cycles INTEGER NOT NULL,
                    origin_lane TEXT NOT NULL,
                    content_label TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );
                CREATE TRIGGER IF NOT EXISTS plan_divergences_no_update
                BEFORE UPDATE ON plan_divergences
                BEGIN SELECT RAISE(ABORT, 'plan_divergences is append-only'); END;
                CREATE TRIGGER IF NOT EXISTS plan_divergences_no_delete
                BEFORE DELETE ON plan_divergences
                BEGIN SELECT RAISE(ABORT, 'plan_divergences is append-only'); END;";

/// One run's plan-versus-actual divergence, derived from its report
/// by [`derive_divergence`] and appended by [`DivergenceStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanDivergence {
    /// The run this record describes (one record per run, ever).
    pub run_id: String,
    /// The run's task class, copied from its outcome row.
    pub task_class: String,
    /// Steps parsed from the plan's Steps section (exact count).
    pub planned_steps: u64,
    /// Steps naming at least one file that the run changed.
    pub executed_steps: u64,
    /// Texts of steps naming files the run never changed (capped at
    /// [`PLANNED_STEPS_MAX`] entries).
    pub skipped_steps: Vec<String>,
    /// Steps naming no file: no file evidence either way.
    pub untracked_steps: u64,
    /// Files-section paths the run never changed.
    pub planned_files_untouched: Vec<String>,
    /// Changed files named neither in Files nor in any step.
    pub added_files: Vec<String>,
    /// Cycles beyond the first (`cycles - 1`): repair work the plan
    /// did not anticipate.
    pub repair_cycles: u64,
    /// Bloch origin lane; empty is stored as `unknown`.
    pub origin_lane: String,
    /// Bloch content label; empty is stored as `nsfw` (fail-closed).
    pub content_label: String,
}

/// Derive one run's divergence from its runtime report — the exact
/// `Value` the CLI, TUI, and MCP surfaces emit.
///
/// Extraction is defensive and honest, mirroring
/// [`crate::RunOutcome::from_report`]: a field the report does not
/// carry yields its empty value, never a guess. Lane labels are the
/// caller-supplied labels of the run's outcome row, copied verbatim.
pub fn derive_divergence(
    run_id: &str,
    task_class: &str,
    report: &Value,
    origin_lane: &str,
    content_label: &str,
) -> PlanDivergence {
    let plan = report
        .get("roles")
        .and_then(|v| v.as_array())
        .and_then(|roles| {
            roles
                .iter()
                .find(|r| r.get("role").and_then(|v| v.as_str()) == Some("planner"))
        })
        .and_then(|r| r.get("summary"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let (planned_files, steps) = plan_sections(plan);
    let changed: Vec<String> = report
        .get("changed_files")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let changed_set: std::collections::BTreeSet<&str> =
        changed.iter().map(String::as_str).collect();

    let mut executed_steps = 0u64;
    let mut skipped_steps: Vec<String> = Vec::new();
    let mut untracked_steps = 0u64;
    let mut named_paths: std::collections::BTreeSet<String> =
        planned_files.iter().cloned().collect();
    for step in &steps {
        let paths = backticked_paths(&step.raw);
        for path in &paths {
            named_paths.insert(path.clone());
        }
        if paths.is_empty() {
            untracked_steps += 1;
        } else if paths.iter().any(|path| changed_set.contains(path.as_str())) {
            executed_steps += 1;
        } else if skipped_steps.len() < PLANNED_STEPS_MAX {
            skipped_steps.push(step.text.clone());
        }
    }
    let planned_files_untouched: Vec<String> = planned_files
        .iter()
        .filter(|path| !changed_set.contains(path.as_str()))
        .take(DIVERGENCE_FILES_MAX)
        .cloned()
        .collect();
    let added_files: Vec<String> = changed
        .iter()
        .filter(|path| !named_paths.contains(*path))
        .take(DIVERGENCE_FILES_MAX)
        .cloned()
        .collect();
    let cycles = report.get("cycles").and_then(|v| v.as_u64()).unwrap_or(0);
    PlanDivergence {
        run_id: run_id.to_owned(),
        task_class: task_class.to_owned(),
        planned_steps: steps.len() as u64,
        executed_steps,
        skipped_steps,
        untracked_steps,
        planned_files_untouched,
        added_files,
        repair_cycles: cycles.saturating_sub(1),
        origin_lane: origin_lane.to_owned(),
        content_label: content_label.to_owned(),
    }
}

/// One parsed plan step: the cleaned text stored as evidence, and
/// the raw line its file paths are read from.
struct PlanStep {
    text: String,
    raw: String,
}

/// Split plan text into its Files-section paths and Steps-section
/// steps, following the planner contract's skeleton (Goal / Files /
/// Steps / Checks headings, plain or markdown-decorated).
fn plan_sections(plan: &str) -> (Vec<String>, Vec<PlanStep>) {
    let mut files: Vec<String> = Vec::new();
    let mut steps: Vec<PlanStep> = Vec::new();
    let mut section: Option<&'static str> = None;
    for line in plan.lines() {
        if let Some(heading) = heading_of(line) {
            section = Some(heading);
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match section {
            Some("files") => {
                let paths = backticked_paths(trimmed);
                if paths.is_empty() {
                    let cleaned = clean_list_line(trimmed);
                    if looks_like_path(&cleaned) && !files.contains(&cleaned) {
                        files.push(cleaned);
                    }
                } else {
                    for path in paths {
                        if !files.contains(&path) {
                            files.push(path);
                        }
                    }
                }
            }
            Some("steps") => {
                if steps.len() < PLANNED_STEPS_MAX {
                    steps.push(PlanStep {
                        text: cap_step_text(&clean_list_line(trimmed)),
                        raw: trimmed.to_owned(),
                    });
                }
            }
            _ => {}
        }
    }
    (files, steps)
}

/// The skeleton heading a line declares, if any: the line's text
/// with markdown decoration (`#`, `*`, trailing `:`) stripped,
/// compared case-insensitively against the four contract headings.
fn heading_of(line: &str) -> Option<&'static str> {
    let text = line
        .trim()
        .trim_start_matches(['#', '*', ' '])
        .trim_end_matches(['*', ':', ' '])
        .to_lowercase();
    match text.as_str() {
        "goal" => Some("goal"),
        "files" => Some("files"),
        "steps" => Some("steps"),
        "checks" => Some("checks"),
        _ => None,
    }
}

/// Strip a list marker (`-`, `*`, `•`) or an ordinal (`1.`, `2)`)
/// from a plan content line.
fn clean_list_line(line: &str) -> String {
    let mut text = line.trim();
    for marker in ["- ", "* ", "• "] {
        if let Some(rest) = text.strip_prefix(marker) {
            text = rest.trim();
            break;
        }
    }
    let digits: usize = text.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let rest = &text[digits..];
        if rest.starts_with('.') || rest.starts_with(')') {
            text = rest[1..].trim();
        }
    }
    text.to_owned()
}

/// Backtick-quoted spans of `text` that look like file paths.
fn backticked_paths(text: &str) -> Vec<String> {
    text.split('`')
        .enumerate()
        .filter(|(index, _)| index % 2 == 1)
        .map(|(_, span)| span.trim().to_owned())
        .filter(|span| looks_like_path(span))
        .collect()
}

/// A span names a path when it is one whitespace-free token
/// containing a `/` or a `.` (an extension) and stays short.
fn looks_like_path(text: &str) -> bool {
    !text.is_empty()
        && text.chars().count() <= 512
        && !text.chars().any(char::is_whitespace)
        && (text.contains('/') || text.contains('.'))
}

/// Cap a stored step text at [`STEP_TEXT_CHARS_MAX`], marking the
/// cut with an ellipsis so truncated evidence reads as truncated.
fn cap_step_text(text: &str) -> String {
    if text.chars().count() <= STEP_TEXT_CHARS_MAX {
        return text.to_owned();
    }
    let cut: String = text.chars().take(STEP_TEXT_CHARS_MAX - 1).collect();
    format!("{cut}…")
}

/// A recorded divergence row: the record plus its store identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedDivergence {
    /// Row id (`INTEGER PRIMARY KEY AUTOINCREMENT`).
    pub id: i64,
    /// UTC timestamp the row was appended.
    pub created_at: String,
    /// The recorded divergence.
    pub divergence: PlanDivergence,
}

/// The divergence store: append records, read them back. Owns one
/// synchronous SQLite [`Connection`] over the feedback database
/// file, like [`crate::OutcomeStore`].
pub struct DivergenceStore {
    db_path: PathBuf,
    connection: Connection,
}

impl DivergenceStore {
    /// Open (creating parent directories) the feedback database file
    /// and initialize the divergence table and its append-only
    /// triggers.
    pub fn open(db_path: &Path) -> Result<DivergenceStore, SelfImproveError> {
        let parent = db_path
            .parent()
            .ok_or_else(|| SelfImproveError::BadPath(db_path.to_path_buf()))?;
        std::fs::create_dir_all(parent).map_err(SelfImproveError::Io)?;
        let connection = Connection::open(db_path).map_err(SelfImproveError::Database)?;
        connection.execute_batch(DIVERGENCES_DDL)?;
        Ok(DivergenceStore {
            db_path: db_path.to_path_buf(),
            connection,
        })
    }

    /// The database file path.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Append one run's divergence record. Returns the row id.
    ///
    /// A second record for the same `run_id` fails with
    /// [`SelfImproveError::DivergenceDuplicate`]: every run leaves
    /// exactly one record.
    pub fn record(&self, divergence: &PlanDivergence) -> Result<i64, SelfImproveError> {
        let divergence = divergence.clone().validated()?;
        let skipped_json = serde_json::Value::Array(
            divergence
                .skipped_steps
                .iter()
                .map(|s| Value::String(s.clone()))
                .collect(),
        )
        .to_string();
        let untouched_json = serde_json::Value::Array(
            divergence
                .planned_files_untouched
                .iter()
                .map(|s| Value::String(s.clone()))
                .collect(),
        )
        .to_string();
        let added_json = serde_json::Value::Array(
            divergence
                .added_files
                .iter()
                .map(|s| Value::String(s.clone()))
                .collect(),
        )
        .to_string();
        // Backstop: the count caps compose to stay under the byte
        // bound, so this refusal is unreachable by construction —
        // it exists so a future cap change fails loudly, not silently.
        for json in [&skipped_json, &untouched_json, &added_json] {
            if json.len() > DIVERGENCE_JSON_BYTES_MAX {
                return Err(invalid(
                    "divergence_json",
                    "exceeds DIVERGENCE_JSON_BYTES_MAX",
                ));
            }
        }
        match self.connection.execute(
            "INSERT INTO plan_divergences (run_id, task_class, planned_steps,
             executed_steps, skipped_steps_json, untracked_steps,
             planned_files_untouched_json, added_files_json, repair_cycles,
             origin_lane, content_label, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                divergence.run_id,
                divergence.task_class,
                divergence.planned_steps as i64,
                divergence.executed_steps as i64,
                skipped_json,
                divergence.untracked_steps as i64,
                untouched_json,
                added_json,
                divergence.repair_cycles as i64,
                divergence.origin_lane,
                divergence.content_label,
                utc_now_iso(),
            ],
        ) {
            Ok(_) => Ok(self.connection.last_insert_rowid()),
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == ErrorCode::ConstraintViolation =>
            {
                Err(SelfImproveError::DivergenceDuplicate {
                    run_id: divergence.run_id.clone(),
                })
            }
            Err(e) => Err(SelfImproveError::Database(e)),
        }
    }

    /// How many records exist for `run_id` (0 or 1).
    pub fn count_for_run(&self, run_id: &str) -> Result<u64, SelfImproveError> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM plan_divergences WHERE run_id = ?",
            params![run_id],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    /// One run's recorded divergence, or `None` when it left none.
    pub fn get(&self, run_id: &str) -> Result<Option<RecordedDivergence>, SelfImproveError> {
        let mut statement = self.connection.prepare(
            "SELECT id, run_id, task_class, planned_steps, executed_steps,
             skipped_steps_json, untracked_steps, planned_files_untouched_json,
             added_files_json, repair_cycles, origin_lane, content_label,
             created_at
             FROM plan_divergences WHERE run_id = ?",
        )?;
        let mut rows = statement.query_map(params![run_id], row_to_recorded)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// One task class's records, oldest first (aggregation order is
    /// stable), at most `limit` rows (capped at
    /// [`DIVERGENCE_LIST_LIMIT_MAX`]).
    pub fn list_for_class(
        &self,
        task_class: &str,
        limit: u32,
    ) -> Result<Vec<RecordedDivergence>, SelfImproveError> {
        let limit = limit.min(DIVERGENCE_LIST_LIMIT_MAX);
        let mut statement = self.connection.prepare(
            "SELECT id, run_id, task_class, planned_steps, executed_steps,
             skipped_steps_json, untracked_steps, planned_files_untouched_json,
             added_files_json, repair_cycles, origin_lane, content_label,
             created_at
             FROM plan_divergences WHERE task_class = ? ORDER BY id ASC LIMIT ?",
        )?;
        let rows = statement.query_map(params![task_class, limit], row_to_recorded)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Every task class with at least one record, sorted.
    pub fn classes(&self) -> Result<Vec<String>, SelfImproveError> {
        let mut statement = self
            .connection
            .prepare("SELECT DISTINCT task_class FROM plan_divergences ORDER BY task_class ASC")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

impl PlanDivergence {
    /// Apply the fail-closed lane defaults and check every bound,
    /// mirroring [`crate::RunOutcome`]'s validation discipline.
    fn validated(mut self) -> Result<PlanDivergence, SelfImproveError> {
        if self.origin_lane.trim().is_empty() {
            self.origin_lane = UNKNOWN_ORIGIN_LANE.to_owned();
        }
        if self.content_label.trim().is_empty() {
            self.content_label = UNCLASSIFIED_CONTENT_LABEL.to_owned();
        }
        check_chars("run_id", &self.run_id, RUN_ID_CHARS_MAX)?;
        if self.run_id.trim().is_empty() {
            return Err(invalid("run_id", "must not be empty"));
        }
        check_chars("task_class", &self.task_class, TASK_CLASS_CHARS_MAX)?;
        if self.skipped_steps.len() > PLANNED_STEPS_MAX {
            return Err(invalid("skipped_steps", "exceeds PLANNED_STEPS_MAX"));
        }
        for step in &self.skipped_steps {
            check_chars("skipped_step", step, STEP_TEXT_CHARS_MAX + 1)?;
        }
        if self.planned_files_untouched.len() > DIVERGENCE_FILES_MAX {
            return Err(invalid(
                "planned_files_untouched",
                "exceeds DIVERGENCE_FILES_MAX",
            ));
        }
        if self.added_files.len() > DIVERGENCE_FILES_MAX {
            return Err(invalid("added_files", "exceeds DIVERGENCE_FILES_MAX"));
        }
        check_chars("origin_lane", &self.origin_lane, LANE_LABEL_CHARS_MAX)?;
        check_chars("content_label", &self.content_label, LANE_LABEL_CHARS_MAX)?;
        Ok(self)
    }
}

fn invalid(field: &'static str, reason: &str) -> SelfImproveError {
    SelfImproveError::DivergenceInvalid {
        field,
        reason: reason.to_owned(),
    }
}

fn parse_string_list(json: &str) -> Vec<String> {
    serde_json::from_str::<Value>(json)
        .ok()
        .and_then(|v| {
            v.as_array().map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default()
}

fn row_to_recorded(row: &rusqlite::Row) -> rusqlite::Result<RecordedDivergence> {
    let skipped_json: String = row.get(5)?;
    let untouched_json: String = row.get(7)?;
    let added_json: String = row.get(8)?;
    Ok(RecordedDivergence {
        id: row.get(0)?,
        created_at: row.get(12)?,
        divergence: PlanDivergence {
            run_id: row.get(1)?,
            task_class: row.get(2)?,
            planned_steps: row.get::<_, i64>(3)? as u64,
            executed_steps: row.get::<_, i64>(4)? as u64,
            skipped_steps: parse_string_list(&skipped_json),
            untracked_steps: row.get::<_, i64>(6)? as u64,
            planned_files_untouched: parse_string_list(&untouched_json),
            added_files: parse_string_list(&added_json),
            repair_cycles: row.get::<_, i64>(9)? as u64,
            origin_lane: row.get(10)?,
            content_label: row.get(11)?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn db_path(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "phlow-divergence-test-{prefix}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("test setup: create temp dir");
        dir.join("feedback.db")
    }

    fn cleanup(path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    /// A report in the rung-3 corpus's shape: a skeleton plan naming
    /// one file in Files and in two steps, one prose-only step, and
    /// a changed-file list covering the named file plus an unplanned
    /// one.
    fn corpus_shaped_report() -> Value {
        serde_json::json!({
            "status": "ok",
            "cycles": 2,
            "roles": [
                {"role": "planner", "model": "m", "status": "ok",
                 "summary": "**Goal**\nFix the bound.\n\n**Files**\n- `crates/osc/src/codec.rs`\n\n**Steps**\n1. Open `crates/osc/src/codec.rs`.\n2. Replace the value in `crates/osc/src/codec.rs`.\n3. Save the file.\n\n**Checks**\ncargo test"},
                {"role": "coder", "model": "m", "status": "ok", "summary": "done"}
            ],
            "changed_files": ["crates/osc/src/codec.rs", "crates/osc/src/lib.rs"]
        })
    }

    #[test]
    fn derive_reads_the_plan_skeleton() {
        let report = corpus_shaped_report();
        let d = derive_divergence("run-1", "code_fix", &report, "phlow", "nsfw");
        assert_eq!(d.planned_steps, 3);
        // Both file-naming steps saw their file changed.
        assert_eq!(d.executed_steps, 2);
        // "Save the file." names no file: untracked, not skipped.
        assert_eq!(d.untracked_steps, 1);
        assert!(d.skipped_steps.is_empty());
        // The Files-section file was changed; nothing planned stayed.
        assert!(d.planned_files_untouched.is_empty());
        // lib.rs changed without being named anywhere in the plan.
        assert_eq!(d.added_files, vec!["crates/osc/src/lib.rs".to_owned()]);
        // Two cycles = one repair cycle beyond the first.
        assert_eq!(d.repair_cycles, 1);
    }

    #[test]
    fn derive_marks_unexecuted_steps_and_untouched_files() {
        // Adversarial: nothing changed at all — every file-naming
        // step is skipped, the planned file is untouched, and a
        // one-cycle run burns no repair cycles.
        let mut report = corpus_shaped_report();
        report["changed_files"] = serde_json::json!([]);
        report["cycles"] = serde_json::json!(1);
        let d = derive_divergence("run-2", "code_fix", &report, "phlow", "nsfw");
        assert_eq!(d.executed_steps, 0);
        assert_eq!(d.skipped_steps.len(), 2);
        assert_eq!(
            d.planned_files_untouched,
            vec!["crates/osc/src/codec.rs".to_owned()]
        );
        assert!(d.added_files.is_empty());
        assert_eq!(d.repair_cycles, 0);
    }

    #[test]
    fn derive_tolerates_a_report_without_a_plan() {
        // Adversarial: no roles, no changed files, no cycles — the
        // derivation is all zeros, never a panic or an invention.
        let report = serde_json::json!({"status": "error"});
        let d = derive_divergence("run-3", "flow_run", &report, "phlow", "nsfw");
        assert_eq!(d.planned_steps, 0);
        assert_eq!(d.executed_steps, 0);
        assert_eq!(d.untracked_steps, 0);
        assert_eq!(d.repair_cycles, 0);
        assert!(d.planned_files_untouched.is_empty());
        assert!(d.added_files.is_empty());
    }

    #[test]
    fn records_are_append_only_and_one_per_run() {
        let db = db_path("appendonly");
        let store = DivergenceStore::open(&db).unwrap();
        let report = corpus_shaped_report();
        let d = derive_divergence("run-1", "code_fix", &report, "phlow", "nsfw");
        store.record(&d).unwrap();
        assert_eq!(store.count_for_run("run-1").unwrap(), 1);
        // A second record for the same run is a refusal.
        assert!(matches!(
            store.record(&d),
            Err(SelfImproveError::DivergenceDuplicate { .. })
        ));
        // Even a second, independent connection cannot rewrite it.
        let other = Connection::open(&db).unwrap();
        assert!(
            other
                .execute(
                    "UPDATE plan_divergences SET repair_cycles = 0 WHERE run_id = 'run-1'",
                    []
                )
                .is_err()
        );
        assert!(
            other
                .execute("DELETE FROM plan_divergences WHERE run_id = 'run-1'", [])
                .is_err()
        );
        let read = store.get("run-1").unwrap().unwrap();
        assert_eq!(read.divergence, d);
        assert_eq!(store.classes().unwrap(), vec!["code_fix".to_owned()]);
        cleanup(&db);
    }

    #[test]
    fn unclassified_labels_default_fail_closed() {
        let db = db_path("labels");
        let store = DivergenceStore::open(&db).unwrap();
        let report = corpus_shaped_report();
        let d = derive_divergence("run-1", "code_fix", &report, "", "");
        store.record(&d).unwrap();
        let read = store.get("run-1").unwrap().unwrap();
        assert_eq!(read.divergence.origin_lane, UNKNOWN_ORIGIN_LANE);
        assert_eq!(read.divergence.content_label, UNCLASSIFIED_CONTENT_LABEL);
        cleanup(&db);
    }
}
