//! Lesson cards (PH-6): per-task-class planner feedback, retrieved
//! as context — never prompt mutation.
//!
//! A lesson card aggregates one task class's recorded
//! plan-versus-actual divergences ([`crate::divergence`]) into the
//! three most common divergence kinds, with counts and examples.
//! Cards are **versioned and append-only**: building a card over a
//! changed run set appends the next version; rebuilding over an
//! unchanged set returns the current card and appends nothing. Old
//! versions stay readable forever — a card is evidence, and
//! evidence is not rewritten.
//!
//! Retrieval is scope-bound by the Bloch lane law: a card's
//! `content_label` is the most restrictive label among its source
//! records (any non-`sfw` source makes the card `nsfw`), and an
//! `nsfw` card is invisible to a retrieval scoped `sfw`. The runtime
//! hands the rendered card to the planner as run **context** (the
//! user-side message), never by editing the planner's system
//! prompt: lessons are retrieved data, and the prompt stays
//! operator-owned (PH-9 denial 1).
//!
//! # Bounds
//!
//! - [`LESSON_DIVERGENCES_MAX`] kinds per card (the plan's three),
//!   [`LESSON_EXAMPLES_PER_KIND_MAX`] examples per kind,
//!   [`LESSON_SOURCE_ROWS_MAX`] source rows per build,
//!   [`LESSON_RENDER_CHARS_MAX`] rendered characters.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, ErrorCode, params};
use serde_json::Value;

use crate::divergence::{DivergenceStore, RecordedDivergence};
use crate::error::SelfImproveError;
use crate::outcome::utc_now_iso;

/// Divergence kinds per card: the plan's "three most common".
pub const LESSON_DIVERGENCES_MAX: usize = 3;
/// Examples kept per divergence kind.
pub const LESSON_EXAMPLES_PER_KIND_MAX: usize = 2;
/// Most divergence rows one card build aggregates.
pub const LESSON_SOURCE_ROWS_MAX: u32 = 1_000;
/// Longest rendered card, in characters: the card rides in the
/// planner's context, which is itself budgeted.
pub const LESSON_RENDER_CHARS_MAX: usize = 4_000;

/// A planned file the run never changed.
pub const KIND_PLANNED_FILE_UNTOUCHED: &str = "planned_file_untouched";
/// A changed file the plan never named.
pub const KIND_UNPLANNED_FILE_CHANGED: &str = "unplanned_file_changed";
/// A planned step whose named files were never changed.
pub const KIND_PLANNED_STEP_SKIPPED: &str = "planned_step_skipped";
/// Cycles burned beyond the first, summed over runs.
pub const KIND_REPAIR_CYCLES: &str = "repair_cycles";

const LESSONS_DDL: &str = "CREATE TABLE IF NOT EXISTS lesson_cards (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    task_class TEXT NOT NULL,
                    version INTEGER NOT NULL,
                    built_over_runs INTEGER NOT NULL,
                    divergences_json TEXT NOT NULL,
                    origin_lane TEXT NOT NULL,
                    content_label TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    UNIQUE (task_class, version)
                );
                CREATE TRIGGER IF NOT EXISTS lesson_cards_no_update
                BEFORE UPDATE ON lesson_cards
                BEGIN SELECT RAISE(ABORT, 'lesson_cards is append-only'); END;
                CREATE TRIGGER IF NOT EXISTS lesson_cards_no_delete
                BEFORE DELETE ON lesson_cards
                BEGIN SELECT RAISE(ABORT, 'lesson_cards is append-only'); END;";

/// One divergence kind on a card: how often it occurred across the
/// card's source runs, with example evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LessonDivergence {
    /// The divergence kind (one of the `KIND_*` constants).
    pub kind: String,
    /// Total occurrences across the card's source runs.
    pub count: u64,
    /// Example evidence (file paths, step texts, run shapes),
    /// first-seen order, capped at [`LESSON_EXAMPLES_PER_KIND_MAX`].
    pub examples: Vec<String>,
}

/// One versioned lesson card for one task class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LessonCard {
    /// The task class this card feeds.
    pub task_class: String,
    /// Version within the class: 1 for the first card, +1 per build
    /// over a changed run set.
    pub version: u64,
    /// How many divergence records this card aggregates.
    pub built_over_runs: u64,
    /// The most common divergence kinds, count-descending.
    pub divergences: Vec<LessonDivergence>,
    /// Bloch origin lane: the sources' lane, or `multiple` when the
    /// sources span lanes.
    pub origin_lane: String,
    /// Bloch content label: `sfw` only when every source record is
    /// `sfw`; otherwise `nsfw` (fail-closed).
    pub content_label: String,
}

/// The lesson-card store: build, append, and retrieve cards. Owns
/// one synchronous SQLite [`Connection`] over the feedback database
/// file, like [`crate::OutcomeStore`].
pub struct LessonStore {
    db_path: PathBuf,
    connection: Connection,
}

impl LessonStore {
    /// Open (creating parent directories) the feedback database file
    /// and initialize the card table and its append-only triggers.
    pub fn open(db_path: &Path) -> Result<LessonStore, SelfImproveError> {
        let parent = db_path
            .parent()
            .ok_or_else(|| SelfImproveError::BadPath(db_path.to_path_buf()))?;
        std::fs::create_dir_all(parent).map_err(SelfImproveError::Io)?;
        let connection = Connection::open(db_path).map_err(SelfImproveError::Database)?;
        connection.execute_batch(LESSONS_DDL)?;
        Ok(LessonStore {
            db_path: db_path.to_path_buf(),
            connection,
        })
    }

    /// The database file path.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Build the current card for `task_class` from its recorded
    /// divergences.
    ///
    /// Returns `None` when the class has no divergence records —
    /// no evidence, no card. When the newest existing card already
    /// aggregates exactly this run set, it is returned as-is and
    /// nothing is appended; otherwise the next version is appended
    /// and returned.
    pub fn build_card(
        &self,
        divergences: &DivergenceStore,
        task_class: &str,
    ) -> Result<Option<LessonCard>, SelfImproveError> {
        let rows = divergences.list_for_class(task_class, LESSON_SOURCE_ROWS_MAX)?;
        if rows.is_empty() {
            return Ok(None);
        }
        let built = aggregate(task_class, &rows);
        if let Some(current) = self.newest_any(task_class)?
            && current.built_over_runs == built.built_over_runs
            && current.divergences == built.divergences
            && current.origin_lane == built.origin_lane
            && current.content_label == built.content_label
        {
            return Ok(Some(current));
        }
        let version = self
            .newest_any(task_class)?
            .map(|card| card.version + 1)
            .unwrap_or(1);
        let card = LessonCard {
            version,
            ..built
        };
        let divergences_json = serde_json::Value::Array(
            card.divergences
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "kind": d.kind,
                        "count": d.count,
                        "examples": d.examples,
                    })
                })
                .collect(),
        )
        .to_string();
        match self.connection.execute(
            "INSERT INTO lesson_cards (task_class, version, built_over_runs,
             divergences_json, origin_lane, content_label, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                card.task_class,
                card.version as i64,
                card.built_over_runs as i64,
                divergences_json,
                card.origin_lane,
                card.content_label,
                utc_now_iso(),
            ],
        ) {
            Ok(_) => Ok(Some(card)),
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == ErrorCode::ConstraintViolation =>
            {
                Err(SelfImproveError::DivergenceInvalid {
                    field: "lesson_card",
                    reason: format!(
                        "card version {} for class {} already exists",
                        card.version, card.task_class
                    ),
                })
            }
            Err(e) => Err(SelfImproveError::Database(e)),
        }
    }

    /// The newest card for `task_class` visible under a retrieval
    /// scope, or `None`.
    ///
    /// Lane law: an `sfw` card is visible to every scope; an `nsfw`
    /// card is visible only to a scope that itself carries the
    /// `nsfw` label. Any other scope value is treated restrictively
    /// (fail-closed): it sees `sfw` cards only.
    pub fn current_card(
        &self,
        task_class: &str,
        scope_content_label: &str,
    ) -> Result<Option<LessonCard>, SelfImproveError> {
        let mut statement = self.connection.prepare(
            "SELECT task_class, version, built_over_runs, divergences_json,
             origin_lane, content_label
             FROM lesson_cards WHERE task_class = ? ORDER BY version DESC",
        )?;
        let rows = statement.query_map(params![task_class], row_to_card)?;
        for row in rows {
            let card = row?;
            if card.content_label == "sfw" || scope_content_label == "nsfw" {
                return Ok(Some(card));
            }
        }
        Ok(None)
    }

    /// The newest card for `task_class` regardless of scope (the
    /// build path's own read; retrieval goes through
    /// [`LessonStore::current_card`]).
    fn newest_any(&self, task_class: &str) -> Result<Option<LessonCard>, SelfImproveError> {
        let mut statement = self.connection.prepare(
            "SELECT task_class, version, built_over_runs, divergences_json,
             origin_lane, content_label
             FROM lesson_cards WHERE task_class = ? ORDER BY version DESC LIMIT 1",
        )?;
        let mut rows = statement.query_map(params![task_class], row_to_card)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }
}

/// Aggregate one class's divergence records into a card body
/// (version 0; the store assigns the real version on append).
fn aggregate(task_class: &str, rows: &[RecordedDivergence]) -> LessonCard {
    let mut counts: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut examples: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    let mut note = |kind: &'static str,
                    count: u64,
                    new_examples: &[String],
                    examples: &mut BTreeMap<&'static str, Vec<String>>| {
        *counts.entry(kind).or_default() += count;
        let kept = examples.entry(kind).or_default();
        for example in new_examples {
            if kept.len() < LESSON_EXAMPLES_PER_KIND_MAX && !kept.contains(example) {
                kept.push(example.clone());
            }
        }
    };
    for row in rows {
        let d = &row.divergence;
        note(
            KIND_PLANNED_FILE_UNTOUCHED,
            d.planned_files_untouched.len() as u64,
            &d.planned_files_untouched,
            &mut examples,
        );
        note(
            KIND_UNPLANNED_FILE_CHANGED,
            d.added_files.len() as u64,
            &d.added_files,
            &mut examples,
        );
        note(
            KIND_PLANNED_STEP_SKIPPED,
            d.skipped_steps.len() as u64,
            &d.skipped_steps,
            &mut examples,
        );
        let repair_examples: Vec<String> = if d.repair_cycles > 0 {
            vec![format!(
                "run {} used {} cycles",
                d.run_id,
                d.repair_cycles + 1
            )]
        } else {
            Vec::new()
        };
        note(
            KIND_REPAIR_CYCLES,
            d.repair_cycles,
            &repair_examples,
            &mut examples,
        );
    }
    let mut divergences: Vec<LessonDivergence> = counts
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(kind, count)| LessonDivergence {
            kind: kind.to_owned(),
            count,
            examples: examples.remove(kind).unwrap_or_default(),
        })
        .collect();
    // Most common first; kind name breaks ties so the card is a
    // deterministic function of the run set.
    divergences.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.kind.cmp(&b.kind)));
    divergences.truncate(LESSON_DIVERGENCES_MAX);
    let content_label = if rows
        .iter()
        .all(|row| row.divergence.content_label == "sfw")
    {
        "sfw"
    } else {
        "nsfw"
    };
    let first_lane = &rows[0].divergence.origin_lane;
    let origin_lane = if rows
        .iter()
        .all(|row| &row.divergence.origin_lane == first_lane)
    {
        first_lane.clone()
    } else {
        "multiple".to_owned()
    };
    LessonCard {
        task_class: task_class.to_owned(),
        version: 0,
        built_over_runs: rows.len() as u64,
        divergences,
        origin_lane,
        content_label: content_label.to_owned(),
    }
}

fn row_to_card(row: &rusqlite::Row) -> rusqlite::Result<LessonCard> {
    let divergences_json: String = row.get(3)?;
    let divergences = serde_json::from_str::<Value>(&divergences_json)
        .ok()
        .and_then(|v| {
            v.as_array().map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(LessonDivergence {
                            kind: item.get("kind")?.as_str()?.to_owned(),
                            count: item.get("count")?.as_u64()?,
                            examples: item
                                .get("examples")?
                                .as_array()?
                                .iter()
                                .filter_map(|e| e.as_str().map(str::to_owned))
                                .collect(),
                        })
                    })
                    .collect()
            })
        })
        .unwrap_or_default();
    Ok(LessonCard {
        task_class: row.get(0)?,
        version: row.get::<_, i64>(1)? as u64,
        built_over_runs: row.get::<_, i64>(2)? as u64,
        divergences,
        origin_lane: row.get(4)?,
        content_label: row.get(5)?,
    })
}

/// Render a card as the planner-context text: a header naming the
/// evidence base, then one line per divergence kind with its count
/// and examples. Bounded at [`LESSON_RENDER_CHARS_MAX`]; the cut is
/// marked with an ellipsis.
pub fn render_card(card: &LessonCard) -> String {
    let mut text = format!(
        "Lessons from {} recorded runs of task class \"{}\" (lesson card v{}): \
         these divergences between plan and outcome recur — plan against them.",
        card.built_over_runs, card.task_class, card.version
    );
    for divergence in &card.divergences {
        let label = match divergence.kind.as_str() {
            KIND_PLANNED_FILE_UNTOUCHED => "planned files left unchanged",
            KIND_UNPLANNED_FILE_CHANGED => "files changed that the plan never named",
            KIND_PLANNED_STEP_SKIPPED => "planned steps whose named files were never changed",
            KIND_REPAIR_CYCLES => "repair cycles burned beyond the first",
            other => other,
        };
        text.push_str(&format!("\n- {label} ({} times)", divergence.count));
        if !divergence.examples.is_empty() {
            text.push_str(&format!(": e.g. {}", divergence.examples.join("; ")));
        }
    }
    if text.chars().count() > LESSON_RENDER_CHARS_MAX {
        let cut: String = text.chars().take(LESSON_RENDER_CHARS_MAX - 1).collect();
        return format!("{cut}…");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::divergence::{DivergenceStore, PlanDivergence};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn db_path(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "phlow-lessons-test-{prefix}-{}-{}",
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

    fn divergence(run_id: &str, class: &str, label: &str) -> PlanDivergence {
        PlanDivergence {
            run_id: run_id.to_owned(),
            task_class: class.to_owned(),
            planned_steps: 3,
            executed_steps: 1,
            skipped_steps: vec!["Replace the value in `src/a.rs`.".to_owned()],
            untracked_steps: 1,
            planned_files_untouched: vec!["src/a.rs".to_owned()],
            added_files: vec!["src/b.rs".to_owned()],
            repair_cycles: 1,
            origin_lane: "phlow".to_owned(),
            content_label: label.to_owned(),
        }
    }

    fn seeded(db: &Path, class: &str, labels: &[&str]) -> (DivergenceStore, LessonStore) {
        let divergences = DivergenceStore::open(db).unwrap();
        for (index, label) in labels.iter().enumerate() {
            divergences
                .record(&divergence(&format!("run-{index}"), class, label))
                .unwrap();
        }
        (divergences, LessonStore::open(db).unwrap())
    }

    #[test]
    fn card_aggregates_the_three_most_common_divergences() {
        let db = db_path("aggregate");
        let (divergences, lessons) = seeded(&db, "code_fix", &["nsfw", "nsfw"]);
        let card = lessons
            .build_card(&divergences, "code_fix")
            .unwrap()
            .expect("card built");
        assert_eq!(card.version, 1);
        assert_eq!(card.built_over_runs, 2);
        // Four kinds occurred; the card keeps the three most common.
        // Counts: untouched 2, added 2, skipped 2, repair 2 — ties
        // break by kind name, deterministically.
        assert_eq!(card.divergences.len(), LESSON_DIVERGENCES_MAX);
        assert!(card.divergences.iter().all(|d| d.count == 2));
        assert_eq!(card.divergences[0].kind, KIND_PLANNED_FILE_UNTOUCHED);
        assert_eq!(card.content_label, "nsfw");
        assert_eq!(card.origin_lane, "phlow");
        // Rebuilding over the same run set appends nothing.
        let again = lessons
            .build_card(&divergences, "code_fix")
            .unwrap()
            .expect("card still current");
        assert_eq!(again.version, 1);
        // A new run changes the set: version 2 is appended.
        divergences
            .record(&divergence("run-2", "code_fix", "nsfw"))
            .unwrap();
        let next = lessons
            .build_card(&divergences, "code_fix")
            .unwrap()
            .expect("card rebuilt");
        assert_eq!(next.version, 2);
        assert_eq!(next.built_over_runs, 3);
        cleanup(&db);
    }

    #[test]
    fn no_records_no_card() {
        let db = db_path("empty");
        let divergences = DivergenceStore::open(&db).unwrap();
        let lessons = LessonStore::open(&db).unwrap();
        assert!(lessons.build_card(&divergences, "nope").unwrap().is_none());
        assert!(lessons.current_card("nope", "nsfw").unwrap().is_none());
        cleanup(&db);
    }

    #[test]
    fn lane_law_labels_and_scope_visibility() {
        // Mixed labels: one sfw source, one nsfw source — the card
        // is nsfw (fail-closed) and invisible to an sfw scope.
        let db = db_path("mixed");
        let (divergences, lessons) = seeded(&db, "code_fix", &["sfw", "nsfw"]);
        let card = lessons.build_card(&divergences, "code_fix").unwrap().unwrap();
        assert_eq!(card.content_label, "nsfw");
        assert!(lessons.current_card("code_fix", "sfw").unwrap().is_none());
        assert!(lessons.current_card("code_fix", "nsfw").unwrap().is_some());
        // An unrecognized scope is restrictive, never permissive.
        assert!(lessons.current_card("code_fix", "weird").unwrap().is_none());
        cleanup(&db);

        // All-sfw sources: the card is sfw and visible to both.
        let db = db_path("sfw");
        let (divergences, lessons) = seeded(&db, "analysis", &["sfw", "sfw"]);
        let card = lessons.build_card(&divergences, "analysis").unwrap().unwrap();
        assert_eq!(card.content_label, "sfw");
        assert!(lessons.current_card("analysis", "sfw").unwrap().is_some());
        assert!(lessons.current_card("analysis", "nsfw").unwrap().is_some());
        cleanup(&db);
    }

    #[test]
    fn cards_are_append_only() {
        let db = db_path("appendonly");
        let (divergences, lessons) = seeded(&db, "code_fix", &["nsfw"]);
        lessons.build_card(&divergences, "code_fix").unwrap();
        let other = Connection::open(&db).unwrap();
        assert!(other
            .execute("UPDATE lesson_cards SET version = 9 WHERE task_class = 'code_fix'", [])
            .is_err());
        assert!(other
            .execute("DELETE FROM lesson_cards WHERE task_class = 'code_fix'", [])
            .is_err());
        cleanup(&db);
    }

    #[test]
    fn render_is_bounded_and_names_the_evidence() {
        let db = db_path("render");
        let (divergences, lessons) = seeded(&db, "code_fix", &["nsfw"]);
        let card = lessons.build_card(&divergences, "code_fix").unwrap().unwrap();
        let text = render_card(&card);
        assert!(text.contains("task class \"code_fix\""));
        assert!(text.contains("lesson card v1"));
        assert!(text.chars().count() <= LESSON_RENDER_CHARS_MAX);
        cleanup(&db);
    }
}
