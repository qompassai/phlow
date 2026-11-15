//! Lesson capture (PH-7): a finished run becomes a labelled lesson
//! record for bloch, with the lane decided by flowguard's own
//! trajectory label — never by the caller, and never by the outcome
//! row's own labels.
//!
//! ## The lane rule (the package's gate, verbatim)
//!
//! The capture API takes no lane parameter. Given the run's
//! flowguard trajectory label — a [`LabelSnapshot`], flowguard's
//! handoff type, carrying the audience the run's data ended under
//! and the trust rank it ran at — the lane is derived inside:
//!
//! - a **public** trajectory (readers [`ReaderSet::All`]) captures
//!   as `origin_lane = "general"`, `content_label = "sfw"`;
//! - **anything narrower, or no trajectory at all, captures as
//!   `origin_lane = "restricted"`, `content_label = "nsfw"`** —
//!   fail-closed, per the Bloch lane law (INDEX §2.1: the
//!   restricted lane exists to feed general systems without leaking
//!   restricted substance; INDEX §2.3: lane separation is
//!   structural, from birth, not applied by review).
//!
//! Deriving from the trajectory rather than copying the outcome
//! row's labels matters: rows recorded by the runtime carry the
//! placeholder labels `phlow` / `nsfw` (the runtime knows no lane),
//! and a caller-asserted lane would be exactly the hole the lane
//! law closes. The snapshot's trust rank is recorded as evidence
//! only — the audience decides the lane; a highly trusted run over
//! restricted readers is still restricted substance.
//!
//! ## What a capture is
//!
//! Captures are evidence, held to the same discipline as run
//! history: append-only (UPDATE/DELETE triggers, one capture per
//! run), derived entirely from the PH-1 outcome row plus the
//! trajectory evidence, and carrying their lane labels from birth.
//! A capture is **lesson-eligible** exactly when the run's
//! verification strength reaches `tool_checked` (INDEX §2.3 / BL-2:
//! `lesson_eligible = tool_checked+` — only verified outcomes
//! become lesson cards); weaker runs are still captured, marked
//! ineligible, because the capture log is the audit trail bloch
//! reconciles against, not only the lesson feed.
//!
//! The lesson text is a bounded digest of the outcome row (task
//! class, result, verification, check outcomes, plan excerpt).
//! The row stores the plan, not the task text — joining captures
//! to task descriptions is PH-13's export concern, stated here so
//! the digest is never mistaken for the task.
//!
//! Trajectory evidence is recorded as shape, not substance: the
//! audience class (`public` / `restricted` / `none`), the reader
//! count when restricted, and the trust rank name. Reader
//! identities never enter the record.
//!
//! Nothing here writes outside the outcome store's database, and
//! no path in this module applies a capture anywhere: bloch pulls
//! (its BL-3 intake reconciles per lane); phlow does not push.

use phlow_flowguard::label::{LabelSnapshot, ReaderSet};
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::SelfImproveError;
use crate::outcome::{OutcomeStore, VerificationStrength, check_chars};

/// The general lane (INDEX §2.1): work any estate caller may read.
pub const LANE_GENERAL: &str = "general";
/// The restricted lane (INDEX §2.1): read inside its own lane only.
pub const LANE_RESTRICTED: &str = "restricted";
/// The content label a public trajectory earns.
pub const CONTENT_SFW: &str = "sfw";
/// The content label every narrower audience fails closed to.
pub const CONTENT_NSFW: &str = "nsfw";

/// Maximum characters in a capture's lesson digest.
pub const CAPTURE_TEXT_CHARS_MAX: usize = 2_000;
/// Maximum plan characters quoted into the digest.
pub const CAPTURE_PLAN_CHARS_MAX: usize = 600;
/// Largest page [`CaptureStore::list_visible`] returns.
pub const CAPTURE_LIST_LIMIT_MAX: usize = 1_000;

/// Derive the (origin_lane, content_label) pair for a trajectory —
/// the one place the lane is decided (see the module docs). Pure,
/// so the safety case can pin every branch.
pub fn lane_labels_for_trajectory(
    trajectory: Option<&LabelSnapshot>,
) -> (&'static str, &'static str) {
    match trajectory {
        Some(snapshot) if snapshot.readers == ReaderSet::All => (LANE_GENERAL, CONTENT_SFW),
        _ => (LANE_RESTRICTED, CONTENT_NSFW),
    }
}

/// The BL-3 `result` vocabulary for a run's recorded status: a run
/// that finished is `ok`, one the operator set aside is
/// `abandoned`, everything else is `failed`.
pub fn result_for_status(status: &str) -> &'static str {
    match status {
        "ok" => "ok",
        "abandoned" => "abandoned",
        _ => "failed",
    }
}

/// Whether a run's verification strength makes its capture a
/// lesson candidate: `tool_checked` or stronger (BL-2 notation).
pub fn lesson_eligible(strength: VerificationStrength) -> bool {
    matches!(
        strength,
        VerificationStrength::ToolChecked
            | VerificationStrength::IndependentlyCrossChecked
            | VerificationStrength::OwnerJudged
    )
}

/// One run's lesson capture: the outcome row's evidence plus the
/// lane labels derived from its trajectory, from birth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LessonCapture {
    /// The captured run (joins `run_outcomes.run_id`).
    pub run_id: String,
    /// The run's task class, copied from its outcome row.
    pub task_class: String,
    /// BL-3 result vocabulary: `ok`, `failed`, or `abandoned`.
    pub result: &'static str,
    /// The run's verification strength label.
    pub verification_strength: String,
    /// True exactly when the strength reaches `tool_checked`.
    pub lesson_eligible: bool,
    /// `general` or `restricted`, derived — never asserted.
    pub origin_lane: &'static str,
    /// `sfw` or `nsfw`, derived with the lane.
    pub content_label: &'static str,
    /// Trajectory audience shape: `public`, `restricted`, `none`.
    pub trajectory_audience: &'static str,
    /// Reader count when the trajectory was restricted.
    pub trajectory_readers: Option<usize>,
    /// The trajectory's trust rank name, as evidence (never the
    /// lane input — the audience is).
    pub trajectory_trust: Option<String>,
    /// Bounded digest of the outcome row (see module docs).
    pub lesson_text: String,
}

/// A capture as stored: identity and capture timestamp included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCapture {
    /// Row id in `lesson_captures`.
    pub id: i64,
    /// UTC timestamp the capture was recorded.
    pub created_at: String,
    /// The capture itself.
    pub capture: LessonCapture,
}

/// Append-only store over the `lesson_captures` table of a feedback
/// database (the same file as the outcome store, so a capture and
/// its run reconcile in one place).
pub struct CaptureStore {
    conn: Connection,
}

const CAPTURES_DDL: &str = "
CREATE TABLE IF NOT EXISTS lesson_captures (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL UNIQUE,
    task_class TEXT NOT NULL,
    result TEXT NOT NULL,
    verification_strength TEXT NOT NULL,
    lesson_eligible INTEGER NOT NULL,
    origin_lane TEXT NOT NULL,
    content_label TEXT NOT NULL,
    trajectory_audience TEXT NOT NULL,
    trajectory_readers INTEGER,
    trajectory_trust TEXT,
    lesson_text TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TRIGGER IF NOT EXISTS lesson_captures_no_update
BEFORE UPDATE ON lesson_captures
BEGIN SELECT RAISE(ABORT, 'lesson_captures is append-only'); END;
CREATE TRIGGER IF NOT EXISTS lesson_captures_no_delete
BEFORE DELETE ON lesson_captures
BEGIN SELECT RAISE(ABORT, 'lesson_captures is append-only'); END;
";

impl CaptureStore {
    /// Open (creating if needed) the capture table in the database
    /// at `path`. Parent directories are created, matching
    /// [`OutcomeStore::open`].
    ///
    /// # Errors
    /// [`SelfImproveError::Database`] on any database failure.
    pub fn open(path: &std::path::Path) -> Result<Self, SelfImproveError> {
        let parent = path
            .parent()
            .ok_or_else(|| SelfImproveError::BadPath(path.to_path_buf()))?;
        std::fs::create_dir_all(parent).map_err(SelfImproveError::Io)?;
        let conn = Connection::open(path)?;
        conn.execute_batch(CAPTURES_DDL)?;
        Ok(Self { conn })
    }

    /// Capture one run: read its outcome row, derive the lane from
    /// `trajectory`, and append the capture.
    ///
    /// The capture is evidence-checked like every store in this
    /// crate: a run with no outcome row is refused (there is
    /// nothing to capture), and a run already captured is refused
    /// (one run, one capture — the record is the audit trail).
    ///
    /// # Errors
    /// [`SelfImproveError::CaptureInvalid`] when the run has no
    /// outcome row; [`SelfImproveError::CaptureDuplicate`] when it
    /// is already captured; [`SelfImproveError::Database`] on
    /// database failure.
    pub fn capture(
        &self,
        outcomes: &OutcomeStore,
        run_id: &str,
        trajectory: Option<&LabelSnapshot>,
    ) -> Result<RecordedCapture, SelfImproveError> {
        check_chars("run_id", run_id, crate::outcome::RUN_ID_CHARS_MAX)?;
        let recorded = outcomes
            .get(run_id)?
            .ok_or_else(|| SelfImproveError::CaptureInvalid {
                field: "run_id",
                reason: format!("no outcome row for run {run_id} — nothing to capture"),
            })?;
        let row = &recorded.outcome;
        let (origin_lane, content_label) = lane_labels_for_trajectory(trajectory);
        let (audience, readers, trust) = match trajectory {
            None => ("none", None, None),
            Some(snapshot) => match &snapshot.readers {
                ReaderSet::All => ("public", None, Some(snapshot.trust.clone())),
                ReaderSet::Only(set) => {
                    ("restricted", Some(set.len()), Some(snapshot.trust.clone()))
                }
            },
        };
        let capture = LessonCapture {
            run_id: row.run_id.clone(),
            task_class: row.task_class.clone(),
            result: result_for_status(&row.status),
            verification_strength: row.verification_strength.as_str().to_owned(),
            lesson_eligible: lesson_eligible(row.verification_strength),
            origin_lane,
            content_label,
            trajectory_audience: audience,
            trajectory_readers: readers,
            trajectory_trust: trust,
            lesson_text: lesson_digest(row),
        };
        let created_at = crate::outcome::utc_now_iso();
        let insert = self.conn.execute(
            "INSERT INTO lesson_captures (
                run_id, task_class, result, verification_strength,
                lesson_eligible, origin_lane, content_label,
                trajectory_audience, trajectory_readers, trajectory_trust,
                lesson_text, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                capture.run_id,
                capture.task_class,
                capture.result,
                capture.verification_strength,
                i64::from(capture.lesson_eligible),
                capture.origin_lane,
                capture.content_label,
                capture.trajectory_audience,
                capture.trajectory_readers.map(|n| n as i64),
                capture.trajectory_trust,
                capture.lesson_text,
                created_at,
            ],
        );
        match insert {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Err(SelfImproveError::CaptureDuplicate {
                    run_id: run_id.to_owned(),
                });
            }
            Err(error) => return Err(error.into()),
        }
        let id = self.conn.last_insert_rowid();
        Ok(RecordedCapture {
            id,
            created_at,
            capture,
        })
    }

    /// Fetch one run's capture, if it has been captured.
    ///
    /// # Errors
    /// [`SelfImproveError::Database`] on database failure.
    pub fn get(&self, run_id: &str) -> Result<Option<RecordedCapture>, SelfImproveError> {
        check_chars("run_id", run_id, crate::outcome::RUN_ID_CHARS_MAX)?;
        let found = self
            .conn
            .query_row(
                "SELECT id, run_id, task_class, result, verification_strength,
                        lesson_eligible, origin_lane, content_label,
                        trajectory_audience, trajectory_readers,
                        trajectory_trust, lesson_text, created_at
                 FROM lesson_captures WHERE run_id = ?1",
                params![run_id],
                read_capture,
            )
            .optional()?;
        Ok(found)
    }

    /// Captures visible to a caller scoped at `scope_content_label`,
    /// oldest first: an `nsfw` capture is invisible — and uncounted —
    /// to an `sfw` scope (Bloch lane law). `limit` is clamped to
    /// [`CAPTURE_LIST_LIMIT_MAX`].
    ///
    /// # Errors
    /// [`SelfImproveError::Database`] on database failure.
    pub fn list_visible(
        &self,
        scope_content_label: &str,
        limit: usize,
    ) -> Result<Vec<RecordedCapture>, SelfImproveError> {
        let limit = i64::try_from(limit.min(CAPTURE_LIST_LIMIT_MAX)).unwrap_or(i64::MAX);
        let mut statement = self.conn.prepare(
            "SELECT id, run_id, task_class, result, verification_strength,
                    lesson_eligible, origin_lane, content_label,
                    trajectory_audience, trajectory_readers,
                    trajectory_trust, lesson_text, created_at
             FROM lesson_captures
             WHERE content_label = 'sfw' OR ?1 = 'nsfw'
             ORDER BY id ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![scope_content_label, limit], read_capture)?;
        let mut captures = Vec::new();
        for row in rows {
            captures.push(row?);
        }
        Ok(captures)
    }
}

/// Read one `lesson_captures` row (column order fixed by the
/// SELECTs above).
fn read_capture(row: &rusqlite::Row<'_>) -> rusqlite::Result<RecordedCapture> {
    let result: String = row.get(3)?;
    let origin_lane: String = row.get(6)?;
    let content_label: String = row.get(7)?;
    let audience: String = row.get(8)?;
    let readers: Option<i64> = row.get(9)?;
    Ok(RecordedCapture {
        id: row.get(0)?,
        created_at: row.get(12)?,
        capture: LessonCapture {
            run_id: row.get(1)?,
            task_class: row.get(2)?,
            // The stored labels are this crate's own constants; an
            // unknown value in the table means the file was written
            // by something else — fail closed to the restrictive
            // reading rather than trusting the bytes.
            result: match result.as_str() {
                "ok" => "ok",
                "abandoned" => "abandoned",
                _ => "failed",
            },
            verification_strength: row.get(4)?,
            lesson_eligible: row.get::<_, i64>(5)? != 0,
            origin_lane: if origin_lane == LANE_GENERAL {
                LANE_GENERAL
            } else {
                LANE_RESTRICTED
            },
            content_label: if content_label == CONTENT_SFW {
                CONTENT_SFW
            } else {
                CONTENT_NSFW
            },
            trajectory_audience: match audience.as_str() {
                "public" => "public",
                "restricted" => "restricted",
                _ => "none",
            },
            trajectory_readers: readers.map(|n| n as usize),
            trajectory_trust: row.get(10)?,
            lesson_text: row.get(11)?,
        },
    })
}

/// The capture's lesson digest: a bounded, deterministic summary of
/// the outcome row. The row carries the plan, not the task text —
/// the digest says what the run *was* and how it ended, never
/// invents the task.
fn lesson_digest(row: &crate::outcome::RunOutcome) -> String {
    let checks: Vec<serde_json::Value> =
        serde_json::from_str(&row.checks_json).unwrap_or_default();
    let check_summary = if checks.is_empty() {
        "none recorded".to_owned()
    } else {
        checks
            .iter()
            .map(|check| {
                let name = check.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let status = check
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                format!("{name}={status}")
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let plan_excerpt: String = row.plan.chars().take(CAPTURE_PLAN_CHARS_MAX).collect();
    let mut text = format!(
        "task class {}; result {}; verification {}; reviewer verdict {}; cycles {}; checks: {}. plan: {}",
        row.task_class,
        result_for_status(&row.status),
        row.verification_strength.as_str(),
        row.reviewer_verdict.as_deref().unwrap_or("(none)"),
        row.cycles,
        check_summary,
        plan_excerpt,
    );
    if text.chars().count() > CAPTURE_TEXT_CHARS_MAX {
        text = text.chars().take(CAPTURE_TEXT_CHARS_MAX).collect();
        text.push('…');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::RunOutcome;

    fn store_pair() -> (std::path::PathBuf, OutcomeStore, CaptureStore) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "phlow-capture-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("test setup: create temp dir");
        let path = dir.join("feedback.db");
        let outcomes = OutcomeStore::open(&path).expect("outcome store");
        let captures = CaptureStore::open(&path).expect("capture store");
        (path, outcomes, captures)
    }

    fn record_run(
        outcomes: &OutcomeStore,
        run_id: &str,
        status: &str,
        strength: VerificationStrength,
    ) {
        outcomes
            .record(&RunOutcome {
                run_id: run_id.to_owned(),
                task_class: "code_fix".to_owned(),
                plan: "fix the thing".to_owned(),
                checks_json: r#"[{"name":"cargo-test","status":"ok"}]"#.to_owned(),
                reviewer_verdict: Some("approved".to_owned()),
                cycles: 1,
                tool_calls: 2,
                model_calls: 3,
                context_chars: None,
                wall_ms: 10,
                model_ids: vec!["coder-x".to_owned()],
                skills: Vec::new(),
                status: status.to_owned(),
                verification_strength: strength,
                // The row's own labels claim the general lane on
                // purpose: captures must ignore them (the lane comes
                // from the trajectory), and the tests below prove it.
                origin_lane: "general".to_owned(),
                content_label: "sfw".to_owned(),
                evidence: "test fixture".to_owned(),
            })
            .expect("record run");
    }

    fn snapshot(readers: ReaderSet) -> LabelSnapshot {
        LabelSnapshot {
            readers,
            trust: "operator".to_owned(),
        }
    }

    #[test]
    fn public_trajectory_captures_general_and_sfw() {
        let (_path, outcomes, captures) = store_pair();
        record_run(&outcomes, "run-pub", "ok", VerificationStrength::ToolChecked);
        let recorded = captures
            .capture(&outcomes, "run-pub", Some(&snapshot(ReaderSet::All)))
            .expect("capture");
        assert_eq!(recorded.capture.origin_lane, LANE_GENERAL);
        assert_eq!(recorded.capture.content_label, CONTENT_SFW);
        assert_eq!(recorded.capture.trajectory_audience, "public");
        assert_eq!(recorded.capture.trajectory_readers, None);
        assert_eq!(recorded.capture.trajectory_trust.as_deref(), Some("operator"));
        assert!(recorded.capture.lesson_eligible);
        assert_eq!(recorded.capture.result, "ok");
    }

    #[test]
    fn restricted_trajectory_captures_restricted_and_nsfw() {
        let (_path, outcomes, captures) = store_pair();
        record_run(&outcomes, "run-res", "failed", VerificationStrength::SelfReported);
        let mut set = std::collections::BTreeSet::new();
        set.insert("lane-member@example".to_owned());
        set.insert("second@example".to_owned());
        let recorded = captures
            .capture(&outcomes, "run-res", Some(&snapshot(ReaderSet::Only(set))))
            .expect("capture");
        assert_eq!(recorded.capture.origin_lane, LANE_RESTRICTED);
        assert_eq!(recorded.capture.content_label, CONTENT_NSFW);
        assert_eq!(recorded.capture.trajectory_audience, "restricted");
        // Shape only: the count is recorded, never the identities.
        assert_eq!(recorded.capture.trajectory_readers, Some(2));
        assert!(!recorded.capture.lesson_text.contains("lane-member@example"));
        assert!(!recorded.capture.lesson_eligible);
        assert_eq!(recorded.capture.result, "failed");
    }

    #[test]
    fn missing_trajectory_fails_closed_to_restricted() {
        let (_path, outcomes, captures) = store_pair();
        record_run(&outcomes, "run-none", "ok", VerificationStrength::OwnerJudged);
        let recorded = captures.capture(&outcomes, "run-none", None).expect("capture");
        assert_eq!(recorded.capture.origin_lane, LANE_RESTRICTED);
        assert_eq!(recorded.capture.content_label, CONTENT_NSFW);
        assert_eq!(recorded.capture.trajectory_audience, "none");
        assert!(recorded.capture.lesson_eligible);
    }

    #[test]
    fn capture_requires_an_outcome_row_and_happens_once() {
        let (_path, outcomes, captures) = store_pair();
        let refused = captures.capture(&outcomes, "run-ghost", None);
        assert!(matches!(
            refused,
            Err(SelfImproveError::CaptureInvalid { field: "run_id", .. })
        ));
        record_run(&outcomes, "run-once", "ok", VerificationStrength::ToolChecked);
        captures.capture(&outcomes, "run-once", None).expect("first");
        let duplicate = captures.capture(&outcomes, "run-once", None);
        assert!(matches!(
            duplicate,
            Err(SelfImproveError::CaptureDuplicate { .. })
        ));
    }

    #[test]
    fn nsfw_captures_are_invisible_to_an_sfw_scope() {
        let (_path, outcomes, captures) = store_pair();
        record_run(&outcomes, "run-sfw", "ok", VerificationStrength::ToolChecked);
        record_run(&outcomes, "run-nsfw", "ok", VerificationStrength::ToolChecked);
        captures
            .capture(&outcomes, "run-sfw", Some(&snapshot(ReaderSet::All)))
            .expect("sfw capture");
        captures.capture(&outcomes, "run-nsfw", None).expect("nsfw capture");
        let sfw_view = captures.list_visible(CONTENT_SFW, 10).expect("sfw view");
        assert_eq!(sfw_view.len(), 1);
        assert_eq!(sfw_view[0].capture.run_id, "run-sfw");
        let nsfw_view = captures.list_visible(CONTENT_NSFW, 10).expect("nsfw view");
        assert_eq!(nsfw_view.len(), 2);
    }

    #[test]
    fn captures_are_append_only() {
        let (path, outcomes, captures) = store_pair();
        record_run(&outcomes, "run-lock", "ok", VerificationStrength::ToolChecked);
        captures.capture(&outcomes, "run-lock", None).expect("capture");
        let conn = Connection::open(&path).expect("raw conn");
        assert!(
            conn.execute("UPDATE lesson_captures SET result = 'ok'", [])
                .is_err()
        );
        assert!(conn.execute("DELETE FROM lesson_captures", []).is_err());
    }

    #[test]
    fn abandoned_runs_capture_as_abandoned_not_failed() {
        let (_path, outcomes, captures) = store_pair();
        record_run(
            &outcomes,
            "run-abandoned",
            "abandoned",
            VerificationStrength::SelfReported,
        );
        let recorded = captures
            .capture(&outcomes, "run-abandoned", None)
            .expect("capture");
        assert_eq!(recorded.capture.result, "abandoned");
    }
}
