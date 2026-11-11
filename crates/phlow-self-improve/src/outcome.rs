//! The per-run outcome record (PH-1; INDEX §2.3).
//!
//! One append-only row per run, written by the **runtime**, never by
//! the model: task class, plan, each check's result, reviewer verdict,
//! cycles used, tool calls used, context used, wall time, and the
//! model ids that served. Every run — including one that errored or
//! was killed mid-flight and closed as `abandoned` — leaves exactly
//! one row: `run_id` is `UNIQUE`, a second record for the same run is
//! a typed refusal, and SQLite triggers make the table append-only
//! (no `UPDATE`, no `DELETE` through any connection).
//!
//! The table lives in the **same SQLite file** as the feedback store
//! ([`crate::feedback`]): one store file per phlow home, two tables.
//! It is not a parallel store — [`OutcomeStore::open`] takes the
//! feedback database path, exactly as the calibration collector
//! already passes that path around.
//!
//! Lane law (Bloch): every row carries `origin_lane` and
//! `content_label` from birth. An empty `content_label` is stored as
//! `nsfw` — unclassified defaults to the restrictive label,
//! fail-closed; an empty `origin_lane` is stored as `unknown`, never
//! upgraded silently.
//!
//! # Bounds
//!
//! - [`RUN_ID_CHARS_MAX`], [`TASK_CLASS_CHARS_MAX`],
//!   [`PLAN_CHARS_MAX`], [`CHECKS_JSON_BYTES_MAX`],
//!   [`REVIEWER_VERDICT_CHARS_MAX`], [`STATUS_CHARS_MAX`],
//!   [`MODEL_ID_CHARS_MAX`] / [`MODEL_IDS_MAX`],
//!   [`EVIDENCE_CHARS_MAX`], [`LANE_LABEL_CHARS_MAX`],
//!   [`OUTCOME_LIST_LIMIT_MAX`].

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, ErrorCode, params};

use crate::error::SelfImproveError;

/// A run id longer than this is rejected.
pub const RUN_ID_CHARS_MAX: usize = 128;
/// A task class longer than this is rejected; classes are labels.
pub const TASK_CLASS_CHARS_MAX: usize = 128;
/// A stored plan longer than this is rejected (the feedback store's
/// prompt budget: plans are prompt-shaped text).
pub const PLAN_CHARS_MAX: usize = 100_000;
/// The serialized per-check results may not exceed this many bytes.
pub const CHECKS_JSON_BYTES_MAX: usize = 65_536;
/// A reviewer verdict longer than this is rejected; verdicts are
/// short labels (`approved`, `not_approved`, a status, …).
pub const REVIEWER_VERDICT_CHARS_MAX: usize = 128;
/// A run status longer than this is rejected.
pub const STATUS_CHARS_MAX: usize = 64;
/// One model id longer than this is rejected.
pub const MODEL_ID_CHARS_MAX: usize = 256;
/// Most model ids one row may name.
pub const MODEL_IDS_MAX: usize = 16;
/// Evidence text longer than this is rejected.
pub const EVIDENCE_CHARS_MAX: usize = 10_000;
/// A lane label longer than this is rejected.
pub const LANE_LABEL_CHARS_MAX: usize = 64;
/// `list` never returns more than this many rows.
pub const OUTCOME_LIST_LIMIT_MAX: u32 = 1000;

/// Fail-closed content label for an unclassified run.
pub const UNCLASSIFIED_CONTENT_LABEL: &str = "nsfw";
/// Fail-closed origin lane for a run whose origin was not recorded.
pub const UNKNOWN_ORIGIN_LANE: &str = "unknown";

const OUTCOMES_DDL: &str = "CREATE TABLE IF NOT EXISTS run_outcomes (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    run_id TEXT NOT NULL UNIQUE,
                    task_class TEXT NOT NULL,
                    plan TEXT NOT NULL,
                    checks_json TEXT NOT NULL,
                    reviewer_verdict TEXT,
                    cycles INTEGER NOT NULL,
                    tool_calls INTEGER NOT NULL,
                    model_calls INTEGER NOT NULL,
                    context_chars INTEGER,
                    wall_ms INTEGER NOT NULL,
                    model_ids_json TEXT NOT NULL,
                    status TEXT NOT NULL,
                    verification_strength TEXT NOT NULL,
                    origin_lane TEXT NOT NULL,
                    content_label TEXT NOT NULL,
                    evidence TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );
                CREATE TRIGGER IF NOT EXISTS run_outcomes_no_update
                BEFORE UPDATE ON run_outcomes
                BEGIN SELECT RAISE(ABORT, 'run_outcomes is append-only'); END;
                CREATE TRIGGER IF NOT EXISTS run_outcomes_no_delete
                BEFORE DELETE ON run_outcomes
                BEGIN SELECT RAISE(ABORT, 'run_outcomes is append-only'); END;";

/// How strongly a run's result was verified (INDEX §2.3's ladder:
/// self-reported < tool-checked < independently cross-checked <
/// Owner-judged). An automated run can carry at most
/// [`VerificationStrength::ToolChecked`] — the host verification gate
/// result; the two stronger labels are attached later, by a checker
/// or the Owner, in a *new* record-keeping step, never by rewriting
/// this row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationStrength {
    /// The run's own report, unverified by any tool.
    SelfReported,
    /// A host-run tool (the named checks) verified the result.
    ToolChecked,
    /// An independent second check confirmed the result.
    IndependentlyCrossChecked,
    /// The Owner judged the result personally.
    OwnerJudged,
}

impl VerificationStrength {
    /// Wire string for this strength.
    pub fn as_str(self) -> &'static str {
        match self {
            VerificationStrength::SelfReported => "self_reported",
            VerificationStrength::ToolChecked => "tool_checked",
            VerificationStrength::IndependentlyCrossChecked => "independently_cross_checked",
            VerificationStrength::OwnerJudged => "owner_judged",
        }
    }

    /// Parse a wire string; anything else is refused (PH-K3: an
    /// outcome without a verification-strength label is refused, and
    /// an unknown label is no label).
    pub fn parse(text: &str) -> Option<VerificationStrength> {
        match text {
            "self_reported" => Some(VerificationStrength::SelfReported),
            "tool_checked" => Some(VerificationStrength::ToolChecked),
            "independently_cross_checked" => Some(VerificationStrength::IndependentlyCrossChecked),
            "owner_judged" => Some(VerificationStrength::OwnerJudged),
            _ => None,
        }
    }
}

/// One run's outcome, as the runtime records it. All fields are set by
/// runtime code from the run's own counters and report — the model
/// being run has no path to this struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    /// Unique run id; one row per id, ever.
    pub run_id: String,
    /// Caller classification of the task (the runtime passes its
    /// surface name, `flow_run`, until intake classes land — PH-6).
    pub task_class: String,
    /// The plan the run executed (planner summary).
    pub plan: String,
    /// Per-check results as a JSON array of `{name, status}` objects,
    /// serialized by [`RunOutcome::from_report`] or the caller.
    pub checks_json: String,
    /// The reviewer's verdict label, when a reviewer ran.
    pub reviewer_verdict: Option<String>,
    /// Planner→coder→reviewer cycles used.
    pub cycles: u64,
    /// Tool calls used.
    pub tool_calls: u64,
    /// Model calls used.
    pub model_calls: u64,
    /// Context characters used, when the runtime measured them;
    /// `None` is recorded as SQL `NULL` — unmeasured, never invented.
    pub context_chars: Option<u64>,
    /// Wall-clock duration of the run in milliseconds.
    pub wall_ms: u64,
    /// Model ids that served the run's roles, in first-use order.
    pub model_ids: Vec<String>,
    /// Final run status (`ok`, `failed`, `unverified`, `error`,
    /// `abandoned`, …).
    pub status: String,
    /// Verification strength; see [`VerificationStrength`].
    pub verification_strength: VerificationStrength,
    /// Bloch origin lane; empty is stored as `unknown`.
    pub origin_lane: String,
    /// Bloch content label; empty is stored as `nsfw` (fail-closed).
    pub content_label: String,
    /// What evidence backs this row (e.g. the host verification gate).
    pub evidence: String,
}

impl RunOutcome {
    /// Build the outcome for one finished run from its runtime report
    /// — the exact `Value` the CLI, TUI, and MCP surfaces emit.
    ///
    /// Extraction is defensive and honest: a field the report does not
    /// carry becomes its empty/`None` value, never a guess. `plan` is
    /// the planner role's summary; `reviewer_verdict` is the reviewer
    /// role's `approved` flag when present, else its status;
    /// `model_ids` are the distinct role models in first-use order;
    /// verification strength is `tool_checked` exactly when the
    /// report's host verification says `verified`, else
    /// `self_reported` (PH-K3: the host gate is the strongest label an
    /// automated run can carry).
    pub fn from_report(
        run_id: &str,
        task_class: &str,
        report: &serde_json::Value,
        wall_ms: u64,
    ) -> RunOutcome {
        let roles: Vec<&serde_json::Value> = report
            .get("roles")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        let role_named = |name: &str| {
            roles
                .iter()
                .find(|r| r.get("role").and_then(|v| v.as_str()) == Some(name))
                .copied()
        };
        let plan = role_named("planner")
            .and_then(|r| r.get("summary"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let reviewer_verdict =
            role_named("reviewer").map(|r| match r.get("approved").and_then(|v| v.as_bool()) {
                Some(true) => "approved".to_owned(),
                Some(false) => "not_approved".to_owned(),
                None => r
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unverified")
                    .to_owned(),
            });
        let mut model_ids: Vec<String> = Vec::new();
        for role in &roles {
            if let Some(model) = role.get("model").and_then(|v| v.as_str())
                && !model.is_empty()
                && !model_ids.iter().any(|m| m == model)
            {
                model_ids.push(model.to_owned());
            }
        }
        let checks: Vec<serde_json::Value> = report
            .get("checks")
            .and_then(|v| v.as_array())
            .map(|checks| {
                checks
                    .iter()
                    .map(|check| {
                        serde_json::json!({
                            "name": check.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                            "status": check.get("status").and_then(|v| v.as_str()).unwrap_or(""),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let counter = |key: &str| report.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
        let verified = report
            .get("verified")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        RunOutcome {
            run_id: run_id.to_owned(),
            task_class: task_class.to_owned(),
            plan,
            checks_json: serde_json::Value::Array(checks).to_string(),
            reviewer_verdict,
            cycles: counter("cycles"),
            tool_calls: counter("tool_calls"),
            model_calls: counter("model_calls"),
            context_chars: report.get("context_chars").and_then(|v| v.as_u64()),
            wall_ms,
            model_ids,
            status: report
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unverified")
                .to_owned(),
            verification_strength: if verified {
                VerificationStrength::ToolChecked
            } else {
                VerificationStrength::SelfReported
            },
            origin_lane: "phlow".to_owned(),
            content_label: UNCLASSIFIED_CONTENT_LABEL.to_owned(),
            evidence: "phlow runtime report; host verification gate result carried in \
                       verification_strength"
                .to_owned(),
        }
    }

    /// Apply the fail-closed lane defaults and check every bound.
    /// Called by [`OutcomeStore::record`]; a row that fails validation
    /// is refused, never silently repaired beyond the lane defaults.
    fn validated(mut self) -> Result<RunOutcome, SelfImproveError> {
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
        check_chars("plan", &self.plan, PLAN_CHARS_MAX)?;
        if self.checks_json.len() > CHECKS_JSON_BYTES_MAX {
            return Err(invalid("checks_json", "exceeds CHECKS_JSON_BYTES_MAX"));
        }
        let checks: serde_json::Value = serde_json::from_str(&self.checks_json)
            .map_err(|_| invalid("checks_json", "is not valid JSON"))?;
        if !checks.is_array() {
            return Err(invalid("checks_json", "is not a JSON array"));
        }
        if let Some(verdict) = &self.reviewer_verdict {
            check_chars("reviewer_verdict", verdict, REVIEWER_VERDICT_CHARS_MAX)?;
        }
        check_chars("status", &self.status, STATUS_CHARS_MAX)?;
        if self.model_ids.len() > MODEL_IDS_MAX {
            return Err(invalid("model_ids", "exceeds MODEL_IDS_MAX"));
        }
        for model in &self.model_ids {
            check_chars("model_id", model, MODEL_ID_CHARS_MAX)?;
        }
        check_chars("origin_lane", &self.origin_lane, LANE_LABEL_CHARS_MAX)?;
        check_chars("content_label", &self.content_label, LANE_LABEL_CHARS_MAX)?;
        check_chars("evidence", &self.evidence, EVIDENCE_CHARS_MAX)?;
        Ok(self)
    }
}

/// A recorded outcome row: the outcome plus its store identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedOutcome {
    /// Row id (`INTEGER PRIMARY KEY AUTOINCREMENT`).
    pub id: i64,
    /// UTC timestamp the row was appended.
    pub created_at: String,
    /// The recorded outcome.
    pub outcome: RunOutcome,
}

/// The outcome store: append rows, read them back, count per run.
/// Owns one synchronous SQLite [`Connection`] over the feedback
/// database file, like [`crate::FeedbackStore`]; wrap in a `Mutex`
/// to share across threads.
pub struct OutcomeStore {
    db_path: PathBuf,
    connection: Connection,
}

impl OutcomeStore {
    /// Open (creating parent directories) the feedback database file
    /// and initialize the outcome table and its append-only triggers.
    /// Taking the feedback path is deliberate: one store file, and the
    /// feedback table's schema is untouched by this DDL.
    pub fn open(db_path: &Path) -> Result<OutcomeStore, SelfImproveError> {
        let parent = db_path
            .parent()
            .ok_or_else(|| SelfImproveError::BadPath(db_path.to_path_buf()))?;
        std::fs::create_dir_all(parent).map_err(SelfImproveError::Io)?;
        let connection = Connection::open(db_path).map_err(SelfImproveError::Database)?;
        connection.execute_batch(OUTCOMES_DDL)?;
        Ok(OutcomeStore {
            db_path: db_path.to_path_buf(),
            connection,
        })
    }

    /// The database file path.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Append one run's outcome row. Returns the row id.
    ///
    /// A second row for the same `run_id` fails with
    /// [`SelfImproveError::OutcomeDuplicate`]: every run leaves
    /// exactly one row, and a duplicate attempt is a runtime bug to
    /// surface, not a write to merge.
    pub fn record(&self, outcome: &RunOutcome) -> Result<i64, SelfImproveError> {
        let outcome = outcome.clone().validated()?;
        let model_ids_json = serde_json::Value::Array(
            outcome
                .model_ids
                .iter()
                .map(|m| serde_json::Value::String(m.clone()))
                .collect(),
        )
        .to_string();
        match self.connection.execute(
            "INSERT INTO run_outcomes (run_id, task_class, plan, checks_json,
             reviewer_verdict, cycles, tool_calls, model_calls, context_chars,
             wall_ms, model_ids_json, status, verification_strength,
             origin_lane, content_label, evidence, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                outcome.run_id,
                outcome.task_class,
                outcome.plan,
                outcome.checks_json,
                outcome.reviewer_verdict,
                outcome.cycles as i64,
                outcome.tool_calls as i64,
                outcome.model_calls as i64,
                outcome.context_chars.map(|n| n as i64),
                outcome.wall_ms as i64,
                model_ids_json,
                outcome.status,
                outcome.verification_strength.as_str(),
                outcome.origin_lane,
                outcome.content_label,
                outcome.evidence,
                utc_now_iso(),
            ],
        ) {
            Ok(_) => Ok(self.connection.last_insert_rowid()),
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == ErrorCode::ConstraintViolation =>
            {
                Err(SelfImproveError::OutcomeDuplicate {
                    run_id: outcome.run_id.clone(),
                })
            }
            Err(e) => Err(SelfImproveError::Database(e)),
        }
    }

    /// How many rows exist for `run_id` (0 or 1 — the suite's
    /// "a run with no row fails" assertion reads this).
    pub fn count_for_run(&self, run_id: &str) -> Result<u64, SelfImproveError> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM run_outcomes WHERE run_id = ?",
            params![run_id],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    /// One run's recorded outcome, or `None` when it left no row.
    pub fn get(&self, run_id: &str) -> Result<Option<RecordedOutcome>, SelfImproveError> {
        let mut statement = self.connection.prepare(
            "SELECT id, run_id, task_class, plan, checks_json, reviewer_verdict,
             cycles, tool_calls, model_calls, context_chars, wall_ms,
             model_ids_json, status, verification_strength, origin_lane,
             content_label, evidence, created_at
             FROM run_outcomes WHERE run_id = ?",
        )?;
        let mut rows = statement.query_map(params![run_id], row_to_recorded)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Newest outcomes first, at most `limit` rows (capped at
    /// [`OUTCOME_LIST_LIMIT_MAX`]).
    pub fn list(&self, limit: u32) -> Result<Vec<RecordedOutcome>, SelfImproveError> {
        let limit = limit.min(OUTCOME_LIST_LIMIT_MAX);
        let mut statement = self.connection.prepare(
            "SELECT id, run_id, task_class, plan, checks_json, reviewer_verdict,
             cycles, tool_calls, model_calls, context_chars, wall_ms,
             model_ids_json, status, verification_strength, origin_lane,
             content_label, evidence, created_at
             FROM run_outcomes ORDER BY created_at DESC, id DESC LIMIT ?",
        )?;
        let rows = statement.query_map(params![limit], row_to_recorded)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

fn row_to_recorded(row: &rusqlite::Row) -> rusqlite::Result<RecordedOutcome> {
    let model_ids_json: String = row.get(11)?;
    let model_ids: Vec<String> = serde_json::from_str::<serde_json::Value>(&model_ids_json)
        .ok()
        .and_then(|v| {
            v.as_array().map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default();
    let strength_text: String = row.get(13)?;
    Ok(RecordedOutcome {
        id: row.get(0)?,
        created_at: row.get(17)?,
        outcome: RunOutcome {
            run_id: row.get(1)?,
            task_class: row.get(2)?,
            plan: row.get(3)?,
            checks_json: row.get(4)?,
            reviewer_verdict: row.get(5)?,
            cycles: row.get::<_, i64>(6)? as u64,
            tool_calls: row.get::<_, i64>(7)? as u64,
            model_calls: row.get::<_, i64>(8)? as u64,
            context_chars: row.get::<_, Option<i64>>(9)?.map(|n| n as u64),
            wall_ms: row.get::<_, i64>(10)? as u64,
            model_ids,
            status: row.get(12)?,
            verification_strength: VerificationStrength::parse(&strength_text)
                .unwrap_or(VerificationStrength::SelfReported),
            origin_lane: row.get(14)?,
            content_label: row.get(15)?,
            evidence: row.get(16)?,
        },
    })
}

fn invalid(field: &'static str, reason: &str) -> SelfImproveError {
    SelfImproveError::OutcomeInvalid {
        field,
        reason: reason.to_owned(),
    }
}

/// Reject over-long text fields before they reach the database (the
/// feedback store's discipline, same error shape).
fn check_chars(field: &'static str, value: &str, max_chars: usize) -> Result<(), SelfImproveError> {
    if value.chars().count() > max_chars {
        return Err(SelfImproveError::TooLong { field, max_chars });
    }
    Ok(())
}

/// UTC timestamp in `datetime.utcnow().isoformat()` shape, matching
/// the feedback store so one file sorts consistently.
fn utc_now_iso() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let days = (now.as_secs() / 86_400) as i64;
    let secs_of_day = now.as_secs() % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:06}",
        secs_of_day / 3_600,
        (secs_of_day % 3_600) / 60,
        secs_of_day % 60,
        now.subsec_micros(),
    )
}

/// Days since the Unix epoch to a civil date (Howard Hinnant's
/// algorithm), as in the feedback store.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month + 2) / 5 + 1;
    let month = if month < 10 { month + 3 } else { month - 9 };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn db_path(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "phlow-outcome-test-{prefix}-{}-{}",
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

    fn outcome(run_id: &str, status: &str) -> RunOutcome {
        RunOutcome {
            run_id: run_id.to_owned(),
            task_class: "flow_run".to_owned(),
            plan: "the plan".to_owned(),
            checks_json: "[{\"name\":\"tests\",\"status\":\"ok\"}]".to_owned(),
            reviewer_verdict: Some("approved".to_owned()),
            cycles: 1,
            tool_calls: 3,
            model_calls: 4,
            context_chars: None,
            wall_ms: 42,
            model_ids: vec!["model-a".to_owned()],
            status: status.to_owned(),
            verification_strength: VerificationStrength::ToolChecked,
            origin_lane: "phlow".to_owned(),
            content_label: "sfw".to_owned(),
            evidence: "host verification".to_owned(),
        }
    }

    #[test]
    fn every_run_leaves_exactly_one_row_including_killed() {
        let db = db_path("onerow");
        let store = OutcomeStore::open(&db).unwrap();
        // A normal run, a failed run, and a killed run closed as
        // abandoned: each leaves exactly one row.
        for (run_id, status) in [
            ("run-ok", "ok"),
            ("run-failed", "failed"),
            ("run-killed", "abandoned"),
        ] {
            store.record(&outcome(run_id, status)).unwrap();
            assert_eq!(store.count_for_run(run_id).unwrap(), 1);
        }
        // A second row for the same run is a refusal, not a merge.
        let dup = store.record(&outcome("run-ok", "ok"));
        assert!(matches!(
            dup,
            Err(SelfImproveError::OutcomeDuplicate { .. })
        ));
        assert_eq!(store.count_for_run("run-ok").unwrap(), 1);
        // A run with no row is visible as such (the suite assertion).
        assert_eq!(store.count_for_run("run-never-ran").unwrap(), 0);
        assert!(store.get("run-never-ran").unwrap().is_none());
        cleanup(&db);
    }

    #[test]
    fn rows_are_append_only_against_update_and_delete() {
        let db = db_path("appendonly");
        let store = OutcomeStore::open(&db).unwrap();
        store.record(&outcome("run-1", "ok")).unwrap();
        // Even a second, independent connection cannot rewrite history.
        let other = Connection::open(&db).unwrap();
        assert!(
            other
                .execute(
                    "UPDATE run_outcomes SET status = 'ok2' WHERE run_id = 'run-1'",
                    []
                )
                .is_err()
        );
        assert!(
            other
                .execute("DELETE FROM run_outcomes WHERE run_id = 'run-1'", [])
                .is_err()
        );
        assert_eq!(store.get("run-1").unwrap().unwrap().outcome.status, "ok");
        cleanup(&db);
    }

    #[test]
    fn unclassified_labels_default_fail_closed() {
        let db = db_path("labels");
        let store = OutcomeStore::open(&db).unwrap();
        let mut row = outcome("run-1", "ok");
        row.origin_lane = String::new();
        row.content_label = String::new();
        store.record(&row).unwrap();
        let read = store.get("run-1").unwrap().unwrap();
        assert_eq!(read.outcome.origin_lane, UNKNOWN_ORIGIN_LANE);
        assert_eq!(read.outcome.content_label, UNCLASSIFIED_CONTENT_LABEL);
        cleanup(&db);
    }

    #[test]
    fn outcome_without_strength_label_is_refused() {
        // PH-K3: an unknown strength string never reaches the store —
        // parse refuses it, and record only accepts the typed enum.
        assert!(VerificationStrength::parse("").is_none());
        assert!(VerificationStrength::parse("verified-ish").is_none());
        assert_eq!(
            VerificationStrength::parse("tool_checked"),
            Some(VerificationStrength::ToolChecked)
        );
    }

    #[test]
    fn from_report_extracts_the_run_record() {
        let report = serde_json::json!({
            "status": "ok",
            "verified": true,
            "task": "fix it",
            "roles": [
                {"role": "planner", "model": "m-plan", "status": "ok", "summary": "plan text"},
                {"role": "coder", "model": "m-code", "status": "ok", "summary": "did it"},
                {"role": "reviewer", "model": "m-plan", "status": "ok", "approved": true}
            ],
            "checks": [
                {"name": "tests", "status": "ok"},
                {"name": "lint", "status": "failed"}
            ],
            "cycles": 2,
            "tool_calls": 7,
            "model_calls": 9
        });
        let row = RunOutcome::from_report("run-9", "flow_run", &report, 1234);
        assert_eq!(row.plan, "plan text");
        assert_eq!(row.reviewer_verdict.as_deref(), Some("approved"));
        assert_eq!(
            row.model_ids,
            vec!["m-plan".to_owned(), "m-code".to_owned()]
        );
        assert_eq!(row.cycles, 2);
        assert_eq!(row.tool_calls, 7);
        assert_eq!(row.model_calls, 9);
        assert_eq!(row.wall_ms, 1234);
        assert_eq!(row.context_chars, None);
        assert_eq!(row.verification_strength, VerificationStrength::ToolChecked);
        let checks: serde_json::Value = serde_json::from_str(&row.checks_json).unwrap();
        assert_eq!(checks.as_array().unwrap().len(), 2);
        assert_eq!(checks[1]["status"], "failed");
    }

    #[test]
    fn malformed_checks_json_is_refused() {
        let db = db_path("badchecks");
        let store = OutcomeStore::open(&db).unwrap();
        let mut row = outcome("run-1", "ok");
        row.checks_json = "{\"not\":\"an array\"}".to_owned();
        assert!(matches!(
            store.record(&row),
            Err(SelfImproveError::OutcomeInvalid {
                field: "checks_json",
                ..
            })
        ));
        cleanup(&db);
    }

    #[test]
    fn outcome_table_coexists_with_feedback_table() {
        // Same file as the feedback store: opening both stores over
        // one path works, and neither table disturbs the other.
        let db = db_path("coexist");
        let feedback = crate::FeedbackStore::open(&db).unwrap();
        feedback.record("s1", 5, "", "", "done").unwrap();
        let outcomes = OutcomeStore::open(&db).unwrap();
        outcomes.record(&outcome("run-1", "ok")).unwrap();
        assert_eq!(feedback.average_rating().unwrap(), 5.0);
        assert_eq!(outcomes.count_for_run("run-1").unwrap(), 1);
        cleanup(&db);
    }
}
