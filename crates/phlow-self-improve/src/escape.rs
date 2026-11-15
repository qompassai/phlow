//! Escapes and check proposals (PH-3).
//!
//! An *escape* is a defect the approved checks passed: a reviewer
//! or a later run found it after the run's checks all read `ok`.
//! Escapes are recorded append-only, clustered by the check that
//! would have caught them, and turned into check *proposals* — an
//! exact name and argv, validated against the two revisions that
//! bracket the defect (the proposed check must fail on the escaped
//! revision and pass on the fixed one).
//!
//! A proposal is not a check. Nothing in this module adds a check
//! to any config or admits one anywhere: the proposal path is
//! render a file, queue it through `phlow-approval` as a *pending*
//! record, and stop. Only an operator's verdict on that record —
//! and the operator's own separate act of editing their check
//! config — can admit a check (PH-9 denial 2, extended by §7).
//!
//! Recording is evidence-checked: an escape is refused unless the
//! escaped run has a PH-1 outcome row whose recorded checks are
//! non-empty and all `ok`. A defect a check caught is not an
//! escape, and the store will not call it one.
//!
//! The record lives in the **same SQLite file** as the outcome
//! store, in its own append-only table, carrying the escaped run's
//! task class and lane labels from birth.
//!
//! # Bounds
//!
//! - [`ESCAPE_ID_CHARS_MAX`], [`DEFECT_CHARS_MAX`],
//!   [`FOUND_BY_CHARS_MAX`], [`ESCAPE_CHECK_NAME_CHARS_MAX`],
//!   [`ESCAPE_ARGV_ITEMS_MAX`], [`ESCAPE_ARGV_ITEM_CHARS_MAX`],
//!   [`ESCAPE_LIST_LIMIT_MAX`], [`CHECK_VALIDATION_TIMEOUT`].

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rusqlite::{Connection, ErrorCode, params};
use serde_json::Value;

use crate::error::SelfImproveError;
use crate::outcome::{OutcomeStore, RUN_ID_CHARS_MAX, check_chars, utc_now_iso};

/// An escape id longer than this is rejected.
pub const ESCAPE_ID_CHARS_MAX: usize = 128;
/// A defect description longer than this is rejected.
pub const DEFECT_CHARS_MAX: usize = 2_000;
/// A found-by label longer than this is rejected.
pub const FOUND_BY_CHARS_MAX: usize = 256;
/// A proposed check name longer than this is rejected.
pub const ESCAPE_CHECK_NAME_CHARS_MAX: usize = 128;
/// Most argv items in one proposed check.
pub const ESCAPE_ARGV_ITEMS_MAX: usize = 16;
/// Longest single argv item, in characters.
pub const ESCAPE_ARGV_ITEM_CHARS_MAX: usize = 256;
/// `list` never returns more than this many escape records.
pub const ESCAPE_LIST_LIMIT_MAX: u32 = 1_000;
/// Defect examples kept per proposal cluster.
pub const ESCAPE_EXAMPLES_MAX: usize = 3;
/// Deadline for one validation run of a proposed check. A check
/// that cannot finish inside it is killed and reported as a
/// timeout, never left running.
pub const CHECK_VALIDATION_TIMEOUT: Duration = Duration::from_secs(120);

const ESCAPES_DDL: &str = "CREATE TABLE IF NOT EXISTS escapes (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    escape_id TEXT NOT NULL UNIQUE,
                    run_id TEXT NOT NULL,
                    task_class TEXT NOT NULL,
                    defect TEXT NOT NULL,
                    found_by TEXT NOT NULL,
                    check_name TEXT NOT NULL,
                    check_argv_json TEXT NOT NULL,
                    origin_lane TEXT NOT NULL,
                    content_label TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );
                CREATE TRIGGER IF NOT EXISTS escapes_no_update
                BEFORE UPDATE ON escapes
                BEGIN SELECT RAISE(ABORT, 'escapes is append-only'); END;
                CREATE TRIGGER IF NOT EXISTS escapes_no_delete
                BEFORE DELETE ON escapes
                BEGIN SELECT RAISE(ABORT, 'escapes is append-only'); END;";

/// One escape, as the operator (or a reviewing surface acting for
/// the operator) reports it. The task class and lane labels are not
/// caller-supplied: the store copies them from the escaped run's
/// outcome row, so an escape cannot be re-labelled at record time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Escape {
    /// Caller-supplied unique id; one record per id, ever.
    pub escape_id: String,
    /// The run whose checks passed the defect.
    pub run_id: String,
    /// What the defect is, in the finder's words.
    pub defect: String,
    /// Who or what found it (`reviewer`, `later_run`, a name).
    pub found_by: String,
    /// The proposed check's name.
    pub check_name: String,
    /// The proposed check's exact argv.
    pub check_argv: Vec<String>,
}

/// A recorded escape row: the escape plus the labels and identity
/// the store bound to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedEscape {
    /// Row id (`INTEGER PRIMARY KEY AUTOINCREMENT`).
    pub id: i64,
    /// UTC timestamp the row was appended.
    pub created_at: String,
    /// The recorded escape.
    pub escape: Escape,
    /// The escaped run's task class, from its outcome row.
    pub task_class: String,
    /// The escaped run's Bloch origin lane, from its outcome row.
    pub origin_lane: String,
    /// The escaped run's Bloch content label, from its outcome row.
    pub content_label: String,
}

/// The escape store: append records, read them back. Owns one
/// synchronous SQLite [`Connection`] over the feedback database
/// file, like [`crate::OutcomeStore`].
pub struct EscapeStore {
    db_path: PathBuf,
    connection: Connection,
}

impl EscapeStore {
    /// Open (creating parent directories) the feedback database file
    /// and initialize the escape table and its append-only triggers.
    pub fn open(db_path: &Path) -> Result<EscapeStore, SelfImproveError> {
        let parent = db_path
            .parent()
            .ok_or_else(|| SelfImproveError::BadPath(db_path.to_path_buf()))?;
        std::fs::create_dir_all(parent).map_err(SelfImproveError::Io)?;
        let connection = Connection::open(db_path).map_err(SelfImproveError::Database)?;
        connection.execute_batch(ESCAPES_DDL)?;
        Ok(EscapeStore {
            db_path: db_path.to_path_buf(),
            connection,
        })
    }

    /// The database file path.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Append one escape, evidence-checked against the escaped
    /// run's outcome row.
    ///
    /// Refusals, all typed: the run has no outcome row; the row's
    /// recorded checks are empty or not all `ok` (the defect was
    /// caught, or nothing ran — either way not an escape); a field
    /// is out of bounds; or the escape id already has a record.
    pub fn record(
        &self,
        outcomes: &OutcomeStore,
        escape: &Escape,
    ) -> Result<i64, SelfImproveError> {
        validate_escape(escape)?;
        let row = outcomes
            .get(&escape.run_id)?
            .ok_or_else(|| invalid("run_id", "no outcome row for this run"))?;
        let checks: Value = serde_json::from_str(&row.outcome.checks_json)
            .map_err(|_| invalid("checks_json", "the outcome row's checks are not JSON"))?;
        let checks = checks
            .as_array()
            .ok_or_else(|| invalid("checks_json", "the outcome row's checks are not an array"))?;
        if checks.is_empty() {
            return Err(invalid(
                "checks_json",
                "the escaped run recorded no checks; nothing was escaped from",
            ));
        }
        if let Some(caught) = checks.iter().find(|check| {
            check.get("status").and_then(|v| v.as_str()) != Some("ok")
        }) {
            let name = caught
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let status = caught
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            return Err(invalid(
                "checks_json",
                &format!(
                    "check {name} recorded {status}, not ok: the defect was caught, not escaped"
                ),
            ));
        }
        let argv_json = serde_json::Value::Array(
            escape
                .check_argv
                .iter()
                .map(|item| Value::String(item.clone()))
                .collect(),
        )
        .to_string();
        match self.connection.execute(
            "INSERT INTO escapes (escape_id, run_id, task_class, defect, found_by,
             check_name, check_argv_json, origin_lane, content_label, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                escape.escape_id,
                escape.run_id,
                row.outcome.task_class,
                escape.defect,
                escape.found_by,
                escape.check_name,
                argv_json,
                row.outcome.origin_lane,
                row.outcome.content_label,
                utc_now_iso(),
            ],
        ) {
            Ok(_) => Ok(self.connection.last_insert_rowid()),
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == ErrorCode::ConstraintViolation =>
            {
                Err(SelfImproveError::EscapeDuplicate {
                    escape_id: escape.escape_id.clone(),
                })
            }
            Err(e) => Err(SelfImproveError::Database(e)),
        }
    }

    /// Recorded escapes, newest first, at most `limit` rows (capped
    /// at [`ESCAPE_LIST_LIMIT_MAX`]).
    pub fn list(&self, limit: u32) -> Result<Vec<RecordedEscape>, SelfImproveError> {
        let limit = limit.min(ESCAPE_LIST_LIMIT_MAX);
        let mut statement = self.connection.prepare(
            "SELECT id, escape_id, run_id, task_class, defect, found_by, check_name,
             check_argv_json, origin_lane, content_label, created_at
             FROM escapes ORDER BY id DESC LIMIT ?",
        )?;
        let rows = statement.query_map(params![limit], row_to_recorded)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

fn validate_escape(escape: &Escape) -> Result<(), SelfImproveError> {
    check_chars("escape_id", &escape.escape_id, ESCAPE_ID_CHARS_MAX)
        .map_err(|_| invalid("escape_id", "exceeds ESCAPE_ID_CHARS_MAX"))?;
    if escape.escape_id.trim().is_empty() {
        return Err(invalid("escape_id", "must not be empty"));
    }
    check_chars("run_id", &escape.run_id, RUN_ID_CHARS_MAX)
        .map_err(|_| invalid("run_id", "exceeds RUN_ID_CHARS_MAX"))?;
    if escape.run_id.trim().is_empty() {
        return Err(invalid("run_id", "must not be empty"));
    }
    check_chars("defect", &escape.defect, DEFECT_CHARS_MAX)
        .map_err(|_| invalid("defect", "exceeds DEFECT_CHARS_MAX"))?;
    if escape.defect.trim().is_empty() {
        return Err(invalid("defect", "must not be empty"));
    }
    check_chars("found_by", &escape.found_by, FOUND_BY_CHARS_MAX)
        .map_err(|_| invalid("found_by", "exceeds FOUND_BY_CHARS_MAX"))?;
    if escape.found_by.trim().is_empty() {
        return Err(invalid("found_by", "must not be empty"));
    }
    check_chars(
        "check_name",
        &escape.check_name,
        ESCAPE_CHECK_NAME_CHARS_MAX,
    )
    .map_err(|_| invalid("check_name", "exceeds ESCAPE_CHECK_NAME_CHARS_MAX"))?;
    if escape.check_name.trim().is_empty()
        || escape.check_name.chars().any(char::is_control)
    {
        return Err(invalid(
            "check_name",
            "must be non-empty and free of control characters",
        ));
    }
    if escape.check_argv.is_empty() || escape.check_argv.len() > ESCAPE_ARGV_ITEMS_MAX {
        return Err(invalid(
            "check_argv",
            "must hold between 1 and ESCAPE_ARGV_ITEMS_MAX items",
        ));
    }
    for item in &escape.check_argv {
        check_chars("check_argv", item, ESCAPE_ARGV_ITEM_CHARS_MAX)
            .map_err(|_| invalid("check_argv", "an item exceeds ESCAPE_ARGV_ITEM_CHARS_MAX"))?;
        if item.is_empty() || item.contains('\0') {
            return Err(invalid(
                "check_argv",
                "items must be non-empty and NUL-free",
            ));
        }
    }
    Ok(())
}

fn invalid(field: &'static str, reason: &str) -> SelfImproveError {
    SelfImproveError::EscapeInvalid {
        field,
        reason: reason.to_owned(),
    }
}

fn row_to_recorded(row: &rusqlite::Row) -> rusqlite::Result<RecordedEscape> {
    let argv_json: String = row.get(7)?;
    let check_argv = serde_json::from_str::<Value>(&argv_json)
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
    Ok(RecordedEscape {
        id: row.get(0)?,
        created_at: row.get(10)?,
        escape: Escape {
            escape_id: row.get(1)?,
            run_id: row.get(2)?,
            defect: row.get(4)?,
            found_by: row.get(5)?,
            check_name: row.get(6)?,
            check_argv,
        },
        task_class: row.get(3)?,
        origin_lane: row.get(8)?,
        content_label: row.get(9)?,
    })
}

/// A cluster of escapes sharing one proposed check, plus the
/// proposal the cluster supports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscapeCluster {
    /// The proposed check's name (the cluster key).
    pub check_name: String,
    /// The proposed argv: the newest escape's argv in the cluster.
    pub check_argv: Vec<String>,
    /// Distinct argvs proposed across the cluster; above 1, the
    /// escapes disagree on the detector and the proposal says so.
    pub argv_variants: usize,
    /// Escape ids in the cluster, oldest first.
    pub escape_ids: Vec<String>,
    /// Escaped run ids, oldest first.
    pub run_ids: Vec<String>,
    /// Task classes the escapes span, sorted.
    pub task_classes: Vec<String>,
    /// Defect descriptions, oldest first, capped at
    /// [`ESCAPE_EXAMPLES_MAX`].
    pub defect_examples: Vec<String>,
}

/// Cluster recorded escapes by proposed check name. Deterministic:
/// clusters sort by size descending, then name ascending; members
/// keep record order.
pub fn cluster_escapes(escapes: &[RecordedEscape]) -> Vec<EscapeCluster> {
    // Input arrives newest-first (the store's list order); work
    // oldest-first so examples and the "newest argv" are stable.
    let mut by_name: std::collections::BTreeMap<String, Vec<&RecordedEscape>> =
        std::collections::BTreeMap::new();
    for escape in escapes.iter().rev() {
        by_name
            .entry(escape.escape.check_name.clone())
            .or_default()
            .push(escape);
    }
    let mut clusters: Vec<EscapeCluster> = by_name
        .into_iter()
        .map(|(check_name, members)| {
            let mut argvs: Vec<&Vec<String>> = Vec::new();
            for member in &members {
                if !argvs.contains(&&member.escape.check_argv) {
                    argvs.push(&member.escape.check_argv);
                }
            }
            let mut task_classes: Vec<String> = members
                .iter()
                .map(|m| m.task_class.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            task_classes.sort();
            EscapeCluster {
                check_argv: members
                    .last()
                    .map(|m| m.escape.check_argv.clone())
                    .unwrap_or_default(),
                argv_variants: argvs.len(),
                escape_ids: members.iter().map(|m| m.escape.escape_id.clone()).collect(),
                run_ids: members.iter().map(|m| m.escape.run_id.clone()).collect(),
                task_classes,
                defect_examples: members
                    .iter()
                    .take(ESCAPE_EXAMPLES_MAX)
                    .map(|m| m.escape.defect.clone())
                    .collect(),
                check_name,
            }
        })
        .collect();
    clusters.sort_by(|a, b| {
        b.escape_ids
            .len()
            .cmp(&a.escape_ids.len())
            .then_with(|| a.check_name.cmp(&b.check_name))
    });
    clusters
}

/// The result of running one proposed check against both revisions
/// that bracket its defect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckValidation {
    /// Exit code on the escaped revision (`None`: the run itself
    /// failed — spawn error, signal, or timeout; see `error`).
    pub escaped_exit: Option<i32>,
    /// Exit code on the fixed revision.
    pub fixed_exit: Option<i32>,
    /// Why a run failed, when one did.
    pub error: Option<String>,
    /// The PH-3 gate: the check fails on the escaped revision and
    /// passes on the fixed one.
    pub gate_holds: bool,
}

/// Run a proposed check's exact argv in both revision directories.
///
/// The argv is executed directly (no shell): item 0 is the program,
/// the rest are its arguments, and the revision directory is the
/// working directory. Each run is bounded by
/// [`CHECK_VALIDATION_TIMEOUT`]; a run past the deadline is killed
/// and reported, never left behind.
pub fn validate_check(
    argv: &[String],
    escaped_dir: &Path,
    fixed_dir: &Path,
) -> CheckValidation {
    let escaped = run_bounded(argv, escaped_dir);
    let fixed = run_bounded(argv, fixed_dir);
    let (escaped_exit, escaped_error) = match escaped {
        Ok(code) => (Some(code), None),
        Err(error) => (None, Some(error)),
    };
    let (fixed_exit, fixed_error) = match fixed {
        Ok(code) => (Some(code), None),
        Err(error) => (None, Some(error)),
    };
    let error = match (escaped_error, fixed_error) {
        (Some(a), Some(b)) => Some(format!("escaped run: {a}; fixed run: {b}")),
        (Some(a), None) => Some(format!("escaped run: {a}")),
        (None, Some(b)) => Some(format!("fixed run: {b}")),
        (None, None) => None,
    };
    CheckValidation {
        gate_holds: escaped_exit.is_some_and(|code| code != 0) && fixed_exit == Some(0),
        escaped_exit,
        fixed_exit,
        error,
    }
}

fn run_bounded(argv: &[String], dir: &Path) -> Result<i32, String> {
    let Some((program, args)) = argv.split_first() else {
        return Err("empty argv".to_owned());
    };
    let mut child = Command::new(program)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn {program}: {e}"))?;
    let deadline = Instant::now() + CHECK_VALIDATION_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return status
                    .code()
                    .ok_or_else(|| format!("{program} ended without an exit code (signal)"));
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "{program} exceeded the {}s validation deadline and was killed",
                        CHECK_VALIDATION_TIMEOUT.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(format!("wait on {program}: {e}")),
        }
    }
}

/// Queue one validated check proposal through `phlow-approval`.
///
/// The proposal enters the queue as a **pending** record whose
/// scope names the proposed check (tool `phlow_check:<name>`, risk
/// `process`, no permission change — admitting a check grants the
/// loop nothing) and whose summary carries the exact argv and the
/// validation evidence. This function never decides the record and
/// phlow has no other path from a proposal to an admitted check:
/// only an operator verdict, followed by the operator's own config
/// edit, admits one.
pub fn queue_check_proposal(
    queue: &mut phlow_approval::ApprovalQueue,
    cluster: &EscapeCluster,
    validation: &CheckValidation,
) -> Result<phlow_approval::Record, phlow_approval::Error> {
    assert!(
        validation.gate_holds,
        "only a gate-passing proposal may be queued"
    );
    let summary = proposal_summary(cluster, validation);
    let request = phlow_approval::Request::from_json(&serde_json::json!({
        "tool": format!("phlow_check:{}", cluster.check_name),
        "risk": "process",
        "paths": [],
        "endpoints": [],
        "permissions_before": [],
        "permissions_after": [],
        "summary": summary,
    }))?;
    let run_id = cluster
        .run_ids
        .first()
        .map(String::as_str)
        .unwrap_or("unknown");
    let id = queue.request(run_id, request, phlow_approval::DEFAULT_TTL)?;
    Ok(queue
        .get(&id)
        .expect("a just-admitted queue record is readable"))
}

/// The queue summary for a proposal: name, exact argv, cluster
/// size, and the gate evidence, bounded at the approval crate's
/// summary cap.
fn proposal_summary(cluster: &EscapeCluster, validation: &CheckValidation) -> String {
    let argv = cluster.check_argv.join(" ");
    let mut summary = format!(
        "Admit check '{}' (argv: {}) proposed from {} recorded escape(s). \
         Validation gate: exit {} on the escaped revision, exit {} on the fixed revision. \
         Admission is the operator's act: this record is the queue entry, \
         and the check enters no config by itself.",
        cluster.check_name,
        argv,
        cluster.escape_ids.len(),
        validation.escaped_exit.unwrap_or(-1),
        validation.fixed_exit.unwrap_or(-1),
    );
    while summary.len() > phlow_approval::SUMMARY_BYTES_MAX {
        summary.pop();
    }
    summary
}

/// Render the check-proposal file: one section per cluster, with
/// its evidence, its validation state, and its queue state. The
/// header states the standing rule — nothing here is admitted.
pub fn render_check_proposals(sections: &[ProposalSection]) -> String {
    let mut text = String::from(
        "# Check proposals from escapes (PH-3)\n\n\
         Status: PROPOSALS ONLY. No check named here is admitted, configured, or run \
         by the loop. A proposal enters the approved set only when an operator \
         approves its phlow-approval queue record and adds the check to their own \
         config by hand.\n",
    );
    if sections.is_empty() {
        text.push_str("\nNo escapes recorded; no proposals.\n");
        return text;
    }
    for section in sections {
        let cluster = &section.cluster;
        text.push_str(&format!("\n## {}\n\n", cluster.check_name));
        text.push_str(&format!("- argv: `{}`\n", cluster.check_argv.join(" ")));
        text.push_str(&format!(
            "- escapes: {} ({})\n",
            cluster.escape_ids.len(),
            cluster.escape_ids.join(", ")
        ));
        text.push_str(&format!("- escaped runs: {}\n", cluster.run_ids.join(", ")));
        text.push_str(&format!(
            "- task classes: {}\n",
            cluster.task_classes.join(", ")
        ));
        if cluster.argv_variants > 1 {
            text.push_str(&format!(
                "- argv variants proposed: {} — the escapes disagree on the detector; \
                 the argv above is the newest proposal\n",
                cluster.argv_variants
            ));
        }
        for defect in &cluster.defect_examples {
            text.push_str(&format!("- defect: {defect}\n"));
        }
        match &section.validation {
            Some(validation) => {
                text.push_str(&format!(
                    "- validation: exit {} on the escaped revision, exit {} on the fixed \
                     revision — gate {}\n",
                    validation
                        .escaped_exit
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "no exit".to_owned()),
                    validation
                        .fixed_exit
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "no exit".to_owned()),
                    if validation.gate_holds {
                        "HOLDS"
                    } else {
                        "FAILS"
                    },
                ));
                if let Some(error) = &validation.error {
                    text.push_str(&format!("- validation error: {error}\n"));
                }
            }
            None => {
                text.push_str(
                    "- validation: NOT VALIDATED — no revision pair was supplied, \
                     so this proposal cannot queue\n",
                );
            }
        }
        match &section.queue_state {
            Some((id, state)) => {
                text.push_str(&format!("- queue: record {id}, state {state}\n"));
            }
            None => {
                text.push_str("- queue: not queued\n");
            }
        }
    }
    text
}

/// One rendered proposal: its cluster, its validation (when a
/// revision pair was supplied), and its queue state (when queued).
#[derive(Debug, Clone)]
pub struct ProposalSection {
    /// The escape cluster behind the proposal.
    pub cluster: EscapeCluster,
    /// Validation against the operator-supplied revision pair.
    pub validation: Option<CheckValidation>,
    /// `(queue record id, state)` when the proposal was queued.
    pub queue_state: Option<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::{RunOutcome, VerificationStrength};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "phlow-escape-test-{prefix}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("test setup: create temp dir");
        dir
    }

    fn outcome(run_id: &str, checks_json: &str) -> RunOutcome {
        RunOutcome {
            run_id: run_id.to_owned(),
            task_class: "code_fix".to_owned(),
            plan: "plan".to_owned(),
            checks_json: checks_json.to_owned(),
            reviewer_verdict: Some("approved".to_owned()),
            cycles: 1,
            tool_calls: 1,
            model_calls: 1,
            context_chars: None,
            wall_ms: 1,
            model_ids: Vec::new(),
            status: "ok".to_owned(),
            verification_strength: VerificationStrength::ToolChecked,
            origin_lane: "phlow".to_owned(),
            content_label: "nsfw".to_owned(),
            evidence: "test".to_owned(),
        }
    }

    fn escape(escape_id: &str, run_id: &str) -> Escape {
        Escape {
            escape_id: escape_id.to_owned(),
            run_id: run_id.to_owned(),
            defect: "wrong constant shipped".to_owned(),
            found_by: "reviewer".to_owned(),
            check_name: "osc_bounds".to_owned(),
            check_argv: vec!["grep".to_owned(), "-q".to_owned(), "65507".to_owned()],
        }
    }

    fn seeded(dir: &Path, checks_json: &str) -> (OutcomeStore, EscapeStore) {
        let db = dir.join("feedback.db");
        let outcomes = OutcomeStore::open(&db).unwrap();
        outcomes.record(&outcome("run-1", checks_json)).unwrap();
        (outcomes, EscapeStore::open(&db).unwrap())
    }

    #[test]
    fn an_escape_requires_a_clean_run() {
        let dir = temp_dir("clean");
        // All checks ok: the escape records, with labels copied from
        // the outcome row.
        let (outcomes, escapes) = seeded(
            &dir,
            "[{\"name\":\"tests\",\"status\":\"ok\"},{\"name\":\"lint\",\"status\":\"ok\"}]",
        );
        escapes.record(&outcomes, &escape("esc-1", "run-1")).unwrap();
        let rows = escapes.list(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].task_class, "code_fix");
        assert_eq!(rows[0].content_label, "nsfw");
        // Duplicate id: refused.
        assert!(matches!(
            escapes.record(&outcomes, &escape("esc-1", "run-1")),
            Err(SelfImproveError::EscapeDuplicate { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);

        // Adversarial: a failed check means the defect was caught —
        // recording it as an escape is refused.
        let dir = temp_dir("caught");
        let (outcomes, escapes) = seeded(
            &dir,
            "[{\"name\":\"tests\",\"status\":\"failed\",\"returncode\":1}]",
        );
        let refused = escapes.record(&outcomes, &escape("esc-2", "run-1"));
        assert!(matches!(
            refused,
            Err(SelfImproveError::EscapeInvalid {
                field: "checks_json",
                ..
            })
        ));
        assert!(escapes.list(10).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);

        // Adversarial: no checks at all, and an unknown run.
        let dir = temp_dir("empty");
        let (outcomes, escapes) = seeded(&dir, "[]");
        assert!(escapes.record(&outcomes, &escape("esc-3", "run-1")).is_err());
        assert!(matches!(
            escapes.record(&outcomes, &escape("esc-4", "run-ghost")),
            Err(SelfImproveError::EscapeInvalid {
                field: "run_id",
                ..
            })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn escapes_are_append_only() {
        let dir = temp_dir("appendonly");
        let (outcomes, escapes) = seeded(&dir, "[{\"name\":\"t\",\"status\":\"ok\"}]");
        escapes.record(&outcomes, &escape("esc-1", "run-1")).unwrap();
        let other = Connection::open(dir.join("feedback.db")).unwrap();
        assert!(other
            .execute("UPDATE escapes SET defect = 'rewritten' WHERE escape_id = 'esc-1'", [])
            .is_err());
        assert!(other
            .execute("DELETE FROM escapes WHERE escape_id = 'esc-1'", [])
            .is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clustering_groups_by_check_and_counts_argv_variants() {
        let dir = temp_dir("cluster");
        let (outcomes, escapes) = seeded(&dir, "[{\"name\":\"t\",\"status\":\"ok\"}]");
        outcomes.record(&outcome("run-2", "[{\"name\":\"t\",\"status\":\"ok\"}]")).unwrap();
        escapes.record(&outcomes, &escape("esc-1", "run-1")).unwrap();
        let mut second = escape("esc-2", "run-2");
        second.check_argv = vec!["grep".to_owned(), "-q".to_owned(), "65507|1".to_owned()];
        escapes.record(&outcomes, &second).unwrap();
        let clusters = cluster_escapes(&escapes.list(10).unwrap());
        assert_eq!(clusters.len(), 1);
        let cluster = &clusters[0];
        assert_eq!(cluster.check_name, "osc_bounds");
        assert_eq!(cluster.escape_ids, vec!["esc-1", "esc-2"]);
        assert_eq!(cluster.argv_variants, 2);
        // The proposal names the newest escape's argv.
        assert_eq!(cluster.check_argv, second.check_argv);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validation_gate_needs_failure_then_success() {
        let dir = temp_dir("validate");
        let escaped = dir.join("escaped");
        let fixed = dir.join("fixed");
        std::fs::create_dir_all(&escaped).unwrap();
        std::fs::create_dir_all(&fixed).unwrap();
        std::fs::write(escaped.join("check.sh"), "exit 1\n").unwrap();
        std::fs::write(fixed.join("check.sh"), "exit 0\n").unwrap();
        let argv = vec!["sh".to_owned(), "check.sh".to_owned()];
        let validation = validate_check(&argv, &escaped, &fixed);
        assert_eq!(validation.escaped_exit, Some(1));
        assert_eq!(validation.fixed_exit, Some(0));
        assert!(validation.gate_holds);
        // Adversarial: a check that passes on the escaped revision
        // would not have caught the defect — the gate fails.
        std::fs::write(escaped.join("check.sh"), "exit 0\n").unwrap();
        let validation = validate_check(&argv, &escaped, &fixed);
        assert!(!validation.gate_holds);
        // A missing program is an error, never a pass.
        let validation = validate_check(
            &vec!["phlow-no-such-program".to_owned()],
            &escaped,
            &fixed,
        );
        assert!(!validation.gate_holds);
        assert!(validation.error.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
