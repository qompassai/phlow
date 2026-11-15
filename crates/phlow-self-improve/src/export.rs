//! Outcome export to bloch (PH-13): a recorded run becomes a bloch
//! BL-1 intake record, through one tested mapping.
//!
//! The shapes differ on purpose (the reconciliation table lives in
//! the bloch plan's folded state): phlow's PH-1 row keys its
//! strength `verification_strength`, stores the plan rather than
//! the task text, and measures wall time in milliseconds; bloch's
//! intake (BL-1, with the BL-3 `result` field) wants the key
//! `verification`, the task text as `asked`, and seconds. This
//! module is the one place the translation happens:
//!
//! | bloch intake field | source |
//! |---|---|
//! | `outcome_id` | the run's `run_id` |
//! | `server` | the constant `phlow` |
//! | `asked` | the run's task text, joined from its report |
//! | `produced` | empty — phlow records check results, not artifact hashes |
//! | `result` | the PH-7 capture's BL-3 result |
//! | `verification` | the row's verification strength (same ladder) |
//! | `cost.wall_secs` | `wall_ms / 1000` (seconds, never milliseconds) |
//! | `cost.tokens`, `cost.disk_bytes` | 0 — not metered; tokens are not characters and are never converted |
//! | `skills` | the row's PH-8 skill list |
//! | `origin_lane`, `content_label` | the PH-7 capture's labels |
//! | `evidence` | the capture's trajectory evidence, as shape |
//! | `recorded_at_epoch` | the outcome row's store timestamp |
//!
//! ## Lane law at export
//!
//! The labels come from the run's PH-7 capture — derived inside
//! the capture from the run's flowguard trajectory label — never
//! from the outcome row's own labels (runtime rows carry the
//! placeholders `phlow` / `nsfw`) and never from the caller: the
//! export API has no lane parameter. A run with no capture has no
//! derivable lane and is refused, as is a capture whose lane is
//! not `general` / `restricted` — a lane of `unknown` is refused,
//! never defaulted (the PH-13 gate). And an `nsfw`-labelled record
//! is never exported into an sfw-scoped intake
//! ([`ExportScope::SfwOnly`]): the refusal is typed
//! ([`SelfImproveError::ExportLaneRefused`]) and pinned in the
//! safety case.
//!
//! ## Dedupe — phlow owns it
//!
//! Bloch's store is append-only and does not dedupe (its own
//! round-trip test appends one record twice and reads two back),
//! so a re-export must be stopped on this side. Two nets: the
//! append-only `outcome_exports` table in the same feedback
//! database records every (run, target) export, and the exporter
//! also scans the target store for outcome ids already present
//! before appending — so a lost export table cannot duplicate a
//! record either. The append itself mirrors bloch's discipline:
//! one JSON object per line, written only after the mapping has
//! validated the record.
//!
//! ## The task join
//!
//! The outcome row stores the plan, not the task text; the run's
//! report carries both `task` and `outcome_run_id`. The exporter
//! joins captures to task text through a caller-supplied map of
//! run id → task text (the operator tool builds it from report
//! files). A run whose task text is unavailable is refused by run
//! id — `asked` is never fabricated from the plan.

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::capture::LessonCapture;
use crate::error::SelfImproveError;
use crate::outcome::{OutcomeStore, RecordedOutcome};

/// The producing server name on every exported record.
pub const EXPORT_SERVER_NAME: &str = "phlow";
/// Longest `asked` text bloch's intake accepts, in characters
/// (bloch `ASKED_MAX`, src/outcome.rs at blochs 1.0.0). Longer
/// task text is truncated on a char boundary — `asked` is a
/// bounded summary; the full text stays in the run's report.
pub const ASKED_CHARS_MAX: usize = 4_000;
/// Largest target store the exporter will scan for dedupe, in
/// bytes (bloch `OUTCOMES_FILE_MAX_BYTES`). A bigger target is
/// refused rather than scanned partially — a partial scan could
/// miss an id and duplicate a record.
pub const EXPORT_TARGET_BYTES_MAX: u64 = 8 * 1024 * 1024;
/// Most runs one export batch considers (the outcome store's own
/// list bound, [`crate::outcome::OUTCOME_LIST_LIMIT_MAX`]).
pub const EXPORT_RUNS_MAX: u32 = 1_000;

/// Which intake an export targets, by lane scope. The scope is a
/// property of the *intake*, never of a record: records carry
/// their own labels from their capture, and the scope only decides
/// whether an `nsfw`-labelled record may cross.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportScope {
    /// A general-only intake: `nsfw`-labelled records are refused.
    SfwOnly,
    /// The fleet intake (bloch's store): both lanes are exported;
    /// visibility is governed on bloch's read side by the labels
    /// the records carry.
    Unrestricted,
}

impl ExportScope {
    /// Wire name (`--scope` on the operator tool).
    pub fn as_str(self) -> &'static str {
        match self {
            ExportScope::SfwOnly => "sfw",
            ExportScope::Unrestricted => "unrestricted",
        }
    }

    /// Parse a scope name; anything else is not a scope.
    pub fn parse(text: &str) -> Option<ExportScope> {
        match text {
            "sfw" => Some(ExportScope::SfwOnly),
            "unrestricted" => Some(ExportScope::Unrestricted),
            _ => None,
        }
    }

    /// True when a record with this content label may be exported
    /// into an intake of this scope.
    fn admits(self, content_label: &str) -> bool {
        match self {
            ExportScope::SfwOnly => content_label == "sfw",
            ExportScope::Unrestricted => true,
        }
    }
}

/// Refuse an `nsfw`-labelled record headed for an sfw-scoped
/// intake. Called by the batch exporter per run; the mapping
/// itself is scope-free (a record does not know its intake).
pub fn check_export_scope(
    capture: &LessonCapture,
    scope: ExportScope,
) -> Result<(), SelfImproveError> {
    if scope.admits(capture.content_label) {
        Ok(())
    } else {
        Err(SelfImproveError::ExportLaneRefused {
            run_id: capture.run_id.clone(),
            content_label: capture.content_label.to_owned(),
        })
    }
}

/// Map one recorded run to bloch's intake object. The record's
/// lane labels come from its PH-7 `capture`; the task text is the
/// run's own, joined from its report by the caller.
///
/// # Errors
/// [`SelfImproveError::ExportInvalid`] when the capture's lane is
/// not `general` / `restricted` (an `unknown` lane is refused,
/// never defaulted), when its content label or result is outside
/// the intake vocabularies, when the task text is empty, or when
/// the row's store timestamp does not parse.
pub fn intake_object(
    recorded: &RecordedOutcome,
    capture: &LessonCapture,
    task: &str,
) -> Result<serde_json::Value, SelfImproveError> {
    let row = &recorded.outcome;
    if capture.run_id != row.run_id {
        return Err(invalid(
            "run_id",
            "the capture belongs to a different run than the outcome row",
        ));
    }
    match capture.origin_lane {
        "general" | "restricted" => {}
        other => {
            return Err(invalid(
                "origin_lane",
                &format!(
                    "lane '{other}' is not general|restricted; a lane is refused, never defaulted"
                ),
            ));
        }
    }
    match capture.content_label {
        "sfw" | "nsfw" => {}
        other => {
            return Err(invalid(
                "content_label",
                &format!("content label '{other}' is not sfw|nsfw"),
            ));
        }
    }
    match capture.result {
        "ok" | "failed" | "abandoned" => {}
        other => {
            return Err(invalid(
                "result",
                &format!("result '{other}' is not ok|failed|abandoned"),
            ));
        }
    }
    let asked: String = task.trim().chars().take(ASKED_CHARS_MAX).collect();
    if asked.is_empty() {
        return Err(invalid(
            "asked",
            "the task text is empty; asked is never fabricated",
        ));
    }
    let recorded_at_epoch = parse_iso_epoch(&recorded.created_at).ok_or_else(|| {
        invalid(
            "recorded_at",
            "the outcome row's store timestamp does not parse",
        )
    })?;
    let readers = match capture.trajectory_readers {
        Some(count) => count.to_string(),
        None => "all".to_owned(),
    };
    let evidence = format!(
        "phlow PH-7 capture: flowguard trajectory audience={}, readers={}, trust={}; \
         lane derived inside the capture from the trajectory label, never caller-asserted",
        capture.trajectory_audience,
        readers,
        capture.trajectory_trust.as_deref().unwrap_or("(none)"),
    );
    Ok(serde_json::json!({
        "outcome_id": row.run_id,
        "server": EXPORT_SERVER_NAME,
        "asked": asked,
        "produced": [],
        "result": capture.result,
        "verification": row.verification_strength.as_str(),
        "cost": {
            "wall_secs": row.wall_ms / 1_000,
            "tokens": 0,
            "disk_bytes": 0,
        },
        "skills": row.skills,
        "origin_lane": capture.origin_lane,
        "content_label": capture.content_label,
        "evidence": evidence,
        "recorded_at_epoch": recorded_at_epoch,
    }))
}

fn invalid(field: &'static str, reason: &str) -> SelfImproveError {
    SelfImproveError::ExportInvalid {
        field,
        reason: reason.to_owned(),
    }
}

/// Parse the outcome store's timestamp shape
/// (`utc_now_iso`: `YYYY-MM-DDTHH:MM:SS.ffffff`, UTC) to epoch
/// seconds. Strict about shape and ranges; anything else is `None`
/// and the caller refuses — a timestamp is never guessed.
fn parse_iso_epoch(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let field = |start: usize, len: usize| -> Option<i64> {
        let mut value: i64 = 0;
        for &byte in &bytes[start..start + len] {
            if !byte.is_ascii_digit() {
                return None;
            }
            value = value * 10 + i64::from(byte - b'0');
        }
        Some(value)
    };
    let year = field(0, 4)?;
    let month = field(5, 2)?;
    let day = field(8, 2)?;
    let hour = field(11, 2)?;
    let minute = field(14, 2)?;
    let second = field(17, 2)?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Days from the Unix epoch for a civil date (Howard Hinnant's
/// algorithm — the inverse of `outcome`'s `civil_from_days`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month_prime = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

const EXPORTS_DDL: &str = "
CREATE TABLE IF NOT EXISTS outcome_exports (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    target TEXT NOT NULL,
    outcome_id TEXT NOT NULL,
    exported_at TEXT NOT NULL,
    UNIQUE(run_id, target)
);
CREATE TRIGGER IF NOT EXISTS outcome_exports_no_update
BEFORE UPDATE ON outcome_exports
BEGIN SELECT RAISE(ABORT, 'outcome_exports is append-only'); END;
CREATE TRIGGER IF NOT EXISTS outcome_exports_no_delete
BEFORE DELETE ON outcome_exports
BEGIN SELECT RAISE(ABORT, 'outcome_exports is append-only'); END;
";

/// The export ledger: which runs were exported to which target,
/// in the same feedback database as the outcome rows and captures
/// it reconciles against. Append-only, one row per (run, target).
pub struct ExportStore {
    conn: Connection,
}

impl ExportStore {
    /// Open (creating if needed) the export ledger in the database
    /// at `path`. Parent directories are created, matching
    /// [`OutcomeStore::open`].
    ///
    /// # Errors
    /// [`SelfImproveError::Database`] on any database failure.
    pub fn open(path: &Path) -> Result<Self, SelfImproveError> {
        let parent = path
            .parent()
            .ok_or_else(|| SelfImproveError::BadPath(path.to_path_buf()))?;
        std::fs::create_dir_all(parent).map_err(SelfImproveError::Io)?;
        let conn = Connection::open(path)?;
        conn.execute_batch(EXPORTS_DDL)?;
        Ok(Self { conn })
    }

    /// True when `run_id` is already recorded as exported to
    /// `target` (the target string as the caller named it).
    ///
    /// # Errors
    /// [`SelfImproveError::Database`] on database failure.
    pub fn is_exported(&self, run_id: &str, target: &str) -> Result<bool, SelfImproveError> {
        let found: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM outcome_exports WHERE run_id = ?1 AND target = ?2",
                params![run_id, target],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// Record one export. A second record for the same (run,
    /// target) is [`SelfImproveError::ExportDuplicate`] — the
    /// ledger is the audit trail, and a duplicate entry would
    /// falsify it.
    ///
    /// # Errors
    /// [`SelfImproveError::ExportDuplicate`] when already recorded;
    /// [`SelfImproveError::Database`] on database failure.
    pub fn record(
        &self,
        run_id: &str,
        target: &str,
        outcome_id: &str,
    ) -> Result<(), SelfImproveError> {
        let insert = self.conn.execute(
            "INSERT INTO outcome_exports (run_id, target, outcome_id, exported_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![run_id, target, outcome_id, crate::outcome::utc_now_iso()],
        );
        match insert {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(SelfImproveError::ExportDuplicate {
                    run_id: run_id.to_owned(),
                })
            }
            Err(error) => Err(error.into()),
        }
    }
}

/// What one export batch did, by run id. Refusals carry their
/// reason: a refused run is a visible outcome of the batch, not a
/// silent skip.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportSummary {
    /// Runs appended to the target by this batch, oldest first.
    pub exported: Vec<String>,
    /// Runs already in the target (ledger or target scan); nothing
    /// was appended for them.
    pub already_exported: Vec<String>,
    /// Runs refused, with the refusal's reason.
    pub refused: Vec<(String, String)>,
}

/// Export a feedback store's recorded runs to bloch's intake
/// store at `target` (the JSONL file bloch serves outcomes from).
/// Runs are considered oldest first, at most [`EXPORT_RUNS_MAX`].
/// `tasks` joins run ids to task text (from the runs' reports);
/// a run without task text is refused, never fabricated.
///
/// Per-run refusals (no capture, lane refusal, unmappable fields,
/// missing task text) are collected in the summary. Store and
/// target failures are hard errors: the batch stops rather than
/// guessing at a half-read store.
///
/// # Errors
/// [`SelfImproveError`] on store failure, an oversized or corrupt
/// target, or a failed append.
pub fn export_outcomes(
    db_path: &Path,
    target: &Path,
    scope: ExportScope,
    tasks: &HashMap<String, String>,
) -> Result<ExportSummary, SelfImproveError> {
    let outcomes = OutcomeStore::open(db_path)?;
    let captures = crate::capture::CaptureStore::open(db_path)?;
    let exports = ExportStore::open(db_path)?;
    let target_name = target.to_string_lossy().into_owned();
    let mut present = scan_target(target)?;
    let mut rows = outcomes.list(EXPORT_RUNS_MAX)?;
    rows.reverse();
    let mut summary = ExportSummary::default();
    for recorded in &rows {
        let run_id = recorded.outcome.run_id.clone();
        if present.contains(&run_id) || exports.is_exported(&run_id, &target_name)? {
            summary.already_exported.push(run_id);
            continue;
        }
        let Some(capture) = captures.get(&run_id)? else {
            summary.refused.push((
                run_id,
                "no PH-7 capture for this run; its lane is underivable and is never defaulted"
                    .to_owned(),
            ));
            continue;
        };
        if let Err(error) = check_export_scope(&capture.capture, scope) {
            summary.refused.push((run_id, error.to_string()));
            continue;
        }
        let Some(task) = tasks.get(&run_id) else {
            summary.refused.push((
                run_id,
                "no task text for this run (no report joined it); asked is never fabricated"
                    .to_owned(),
            ));
            continue;
        };
        let object = match intake_object(recorded, &capture.capture, task) {
            Ok(object) => object,
            Err(error) => {
                summary.refused.push((run_id, error.to_string()));
                continue;
            }
        };
        append_line(target, &object)?;
        exports.record(&run_id, &target_name, &run_id)?;
        present.insert(run_id.clone());
        summary.exported.push(run_id);
    }
    Ok(summary)
}

/// The outcome ids already in the target store. A missing target
/// reads as empty; an oversized or corrupt one is a hard refusal —
/// appending past either could duplicate a record.
fn scan_target(target: &Path) -> Result<HashSet<String>, SelfImproveError> {
    let meta = match std::fs::metadata(target) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(HashSet::new());
        }
        Err(error) => return Err(SelfImproveError::Io(error)),
    };
    if meta.len() > EXPORT_TARGET_BYTES_MAX {
        return Err(invalid(
            "target",
            "the target store exceeds the dedupe scan bound; export refuses to append unscanned",
        ));
    }
    let text = std::fs::read_to_string(target).map_err(SelfImproveError::Io)?;
    let mut ids = HashSet::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).map_err(|_| {
            invalid(
                "target",
                &format!(
                    "line {} of the target store is not a JSON object; the store is corrupt \
                     and export refuses to append",
                    index + 1
                ),
            )
        })?;
        match value.get("outcome_id").and_then(|id| id.as_str()) {
            Some(id) => {
                ids.insert(id.to_owned());
            }
            None => {
                return Err(invalid(
                    "target",
                    &format!(
                        "line {} of the target store carries no outcome_id; export refuses \
                         to append unscanned",
                        index + 1
                    ),
                ));
            }
        }
    }
    Ok(ids)
}

/// Append one intake object as a JSON line, creating the target
/// and its parents — bloch's own append discipline, so the bytes
/// on disk are the shape its reader expects.
fn append_line(target: &Path, object: &serde_json::Value) -> Result<(), SelfImproveError> {
    if let Some(parent) = target.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(SelfImproveError::Io)?;
    }
    let line = serde_json::to_string(object)
        .map_err(|error| invalid("record", &format!("serialization failed: {error}")))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(target)
        .map_err(SelfImproveError::Io)?;
    file.write_all(line.as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .map_err(SelfImproveError::Io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{CaptureStore, lane_labels_for_trajectory};
    use crate::outcome::{OutcomeStore, RunOutcome, VerificationStrength};
    use phlow_flowguard::label::{LabelSnapshot, ReaderSet};

    fn temp_dir(prefix: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "phlow-export-test-{prefix}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("test setup: create temp dir");
        dir
    }

    fn row(run_id: &str, status: &str, wall_ms: u64) -> RunOutcome {
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
            wall_ms,
            model_ids: vec!["model-x".to_owned()],
            skills: vec!["tiger-style-rust".to_owned()],
            status: status.to_owned(),
            verification_strength: VerificationStrength::ToolChecked,
            origin_lane: "phlow".to_owned(),
            content_label: "nsfw".to_owned(),
            evidence: "test fixture".to_owned(),
        }
    }

    fn capture(run_id: &str, public: bool) -> LessonCapture {
        let snapshot = LabelSnapshot {
            readers: ReaderSet::All,
            trust: "operator".to_owned(),
        };
        let trajectory = if public { Some(&snapshot) } else { None };
        let (origin_lane, content_label) = lane_labels_for_trajectory(trajectory);
        LessonCapture {
            run_id: run_id.to_owned(),
            task_class: "analysis".to_owned(),
            result: crate::capture::result_for_status("ok"),
            verification_strength: "tool_checked".to_owned(),
            lesson_eligible: true,
            origin_lane,
            content_label,
            trajectory_audience: if public { "public" } else { "none" },
            trajectory_readers: None,
            trajectory_trust: Some("operator".to_owned()),
            lesson_text: "digest".to_owned(),
        }
    }

    fn recorded(run_id: &str) -> RecordedOutcome {
        RecordedOutcome {
            id: 1,
            created_at: "2026-01-01T00:00:00.000000".to_owned(),
            outcome: row(run_id, "ok", 42_500),
        }
    }

    #[test]
    fn mapping_produces_the_intake_shape() {
        let object = intake_object(&recorded("run-1"), &capture("run-1", true), "run the gate")
            .expect("mapping");
        assert_eq!(object["outcome_id"], "run-1");
        assert_eq!(object["server"], "phlow");
        assert_eq!(object["asked"], "run the gate");
        assert_eq!(object["produced"], serde_json::json!([]));
        assert_eq!(object["result"], "ok");
        // The key is bloch's `verification`, not the row's key.
        assert_eq!(object["verification"], "tool_checked");
        assert!(object.get("verification_strength").is_none());
        // Seconds, not milliseconds; tokens are never converted.
        assert_eq!(object["cost"]["wall_secs"], 42);
        assert_eq!(object["cost"]["tokens"], 0);
        assert_eq!(object["cost"]["disk_bytes"], 0);
        assert_eq!(object["skills"], serde_json::json!(["tiger-style-rust"]));
        assert_eq!(object["origin_lane"], "general");
        assert_eq!(object["content_label"], "sfw");
        assert!(!object["evidence"].as_str().unwrap().is_empty());
        assert_eq!(object["recorded_at_epoch"], 1_767_225_600);
    }

    #[test]
    fn mapping_refuses_an_unknown_lane() {
        // The PH-13 gate: a lane of `unknown` (or anything outside
        // general|restricted) is refused, never defaulted.
        let mut cap = capture("run-1", true);
        cap.origin_lane = "unknown";
        let error = intake_object(&recorded("run-1"), &cap, "task").unwrap_err();
        assert!(
            matches!(
                error,
                SelfImproveError::ExportInvalid {
                    field: "origin_lane",
                    ..
                }
            ),
            "{error}"
        );
    }

    #[test]
    fn mapping_refuses_out_of_vocabulary_labels_and_results() {
        let mut cap = capture("run-1", true);
        cap.content_label = "maybe";
        let error = intake_object(&recorded("run-1"), &cap, "task").unwrap_err();
        assert!(matches!(
            error,
            SelfImproveError::ExportInvalid {
                field: "content_label",
                ..
            }
        ));
        let mut cap = capture("run-1", true);
        cap.result = "passed";
        let error = intake_object(&recorded("run-1"), &cap, "task").unwrap_err();
        assert!(matches!(
            error,
            SelfImproveError::ExportInvalid {
                field: "result",
                ..
            }
        ));
    }

    #[test]
    fn mapping_refuses_empty_task_text_and_truncates_long_text() {
        let error = intake_object(&recorded("run-1"), &capture("run-1", true), "  ").unwrap_err();
        assert!(matches!(
            error,
            SelfImproveError::ExportInvalid { field: "asked", .. }
        ));
        let long = "x".repeat(ASKED_CHARS_MAX + 500);
        let object = intake_object(&recorded("run-1"), &capture("run-1", true), &long).unwrap();
        assert_eq!(
            object["asked"].as_str().unwrap().chars().count(),
            ASKED_CHARS_MAX
        );
    }

    #[test]
    fn scope_refuses_nsfw_into_an_sfw_intake_only() {
        let restricted = capture("run-1", false);
        assert!(matches!(
            check_export_scope(&restricted, ExportScope::SfwOnly),
            Err(SelfImproveError::ExportLaneRefused { .. })
        ));
        assert!(check_export_scope(&restricted, ExportScope::Unrestricted).is_ok());
        let public = capture("run-1", true);
        assert!(check_export_scope(&public, ExportScope::SfwOnly).is_ok());
    }

    #[test]
    fn iso_epoch_parses_the_store_shape_and_refuses_garbage() {
        assert_eq!(parse_iso_epoch("1970-01-01T00:00:00.000000"), Some(0));
        assert_eq!(
            parse_iso_epoch("2026-01-01T00:00:00.000000"),
            Some(1_767_225_600)
        );
        assert_eq!(
            parse_iso_epoch("2026-11-15T05:55:00.123456"),
            Some(1_794_722_100)
        );
        assert_eq!(parse_iso_epoch("2026-13-01T00:00:00.000000"), None);
        assert_eq!(parse_iso_epoch("not a timestamp"), None);
        assert_eq!(parse_iso_epoch("2026-01-01 00:00:00"), None);
    }

    fn store_with_runs(dir: &std::path::Path) -> std::path::PathBuf {
        let db = dir.join("feedback.db");
        let outcomes = OutcomeStore::open(&db).unwrap();
        let captures = CaptureStore::open(&db).unwrap();
        outcomes.record(&row("run-pub", "ok", 1_000)).unwrap();
        outcomes.record(&row("run-res", "ok", 2_000)).unwrap();
        outcomes.record(&row("run-nocap", "ok", 3_000)).unwrap();
        let public = LabelSnapshot {
            readers: ReaderSet::All,
            trust: "operator".to_owned(),
        };
        captures
            .capture(&outcomes, "run-pub", Some(&public))
            .unwrap();
        captures.capture(&outcomes, "run-res", None).unwrap();
        db
    }

    fn tasks() -> HashMap<String, String> {
        HashMap::from([
            ("run-pub".to_owned(), "public task".to_owned()),
            ("run-res".to_owned(), "restricted task".to_owned()),
            ("run-nocap".to_owned(), "uncaptured task".to_owned()),
        ])
    }

    #[test]
    fn batch_exports_once_and_reports_refusals() {
        let dir = temp_dir("batch");
        let db = store_with_runs(&dir);
        let target = dir.join("nested").join("outcomes.jsonl");
        let summary = export_outcomes(&db, &target, ExportScope::Unrestricted, &tasks()).unwrap();
        assert_eq!(summary.exported, vec!["run-pub", "run-res"]);
        assert_eq!(summary.refused.len(), 1);
        assert_eq!(summary.refused[0].0, "run-nocap");
        let lines: Vec<String> = std::fs::read_to_string(&target)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        assert_eq!(lines.len(), 2);
        // A second batch appends nothing: the ledger dedupes.
        let again = export_outcomes(&db, &target, ExportScope::Unrestricted, &tasks()).unwrap();
        assert!(again.exported.is_empty());
        assert_eq!(again.already_exported, vec!["run-pub", "run-res"]);
        assert_eq!(std::fs::read_to_string(&target).unwrap().lines().count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn batch_dedupes_by_target_scan_when_the_ledger_is_fresh() {
        // A new database (empty ledger) exporting into a target
        // that already holds the records appends nothing: the
        // target scan is the second net.
        let dir = temp_dir("scan");
        let db = store_with_runs(&dir);
        let target = dir.join("outcomes.jsonl");
        export_outcomes(&db, &target, ExportScope::Unrestricted, &tasks()).unwrap();
        let fresh_db = dir.join("fresh").join("feedback.db");
        let outcomes = OutcomeStore::open(&fresh_db).unwrap();
        let captures = CaptureStore::open(&fresh_db).unwrap();
        outcomes.record(&row("run-pub", "ok", 1_000)).unwrap();
        let public = LabelSnapshot {
            readers: ReaderSet::All,
            trust: "operator".to_owned(),
        };
        captures
            .capture(&outcomes, "run-pub", Some(&public))
            .unwrap();
        let summary =
            export_outcomes(&fresh_db, &target, ExportScope::Unrestricted, &tasks()).unwrap();
        assert!(summary.exported.is_empty());
        assert_eq!(summary.already_exported, vec!["run-pub"]);
        assert_eq!(std::fs::read_to_string(&target).unwrap().lines().count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn batch_sfw_scope_refuses_the_restricted_run() {
        let dir = temp_dir("sfwbatch");
        let db = store_with_runs(&dir);
        let target = dir.join("outcomes.jsonl");
        let summary = export_outcomes(&db, &target, ExportScope::SfwOnly, &tasks()).unwrap();
        assert_eq!(summary.exported, vec!["run-pub"]);
        let refused: Vec<&str> = summary
            .refused
            .iter()
            .map(|(run_id, _)| run_id.as_str())
            .collect();
        assert!(refused.contains(&"run-res"), "{refused:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn batch_refuses_a_run_without_task_text() {
        let dir = temp_dir("notask");
        let db = store_with_runs(&dir);
        let target = dir.join("outcomes.jsonl");
        let mut partial = tasks();
        partial.remove("run-pub");
        let summary = export_outcomes(&db, &target, ExportScope::Unrestricted, &partial).unwrap();
        assert_eq!(summary.exported, vec!["run-res"]);
        assert!(
            summary
                .refused
                .iter()
                .any(|(run_id, reason)| run_id == "run-pub" && reason.contains("task text")),
            "{summary:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
