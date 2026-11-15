//! Skill trust from run rows (PH-8): recompute each skill's
//! standing from the outcomes its runs left, the way PH-5 recomputes
//! reviewer reliability from labelled runs.
//!
//! ## What is measured
//!
//! No surface loads skills into a run today, so the store's
//! `skills` column is empty for almost every row in history (see
//! [`RunOutcome::skills`]); this module is the recording path plus
//! the recompute, and the report it renders says so. What it
//! measures, per skill, over the runs that recorded the skill:
//!
//! - **runs / successes / failures**: a run is a success when its
//!   recorded status is `ok`, a failure when it is `failed`. Other
//!   statuses (`abandoned`, `unverified`, …) are neither — a run
//!   set aside is not evidence for or against the skills it used.
//! - **factual failures**: a `failed` run whose verification
//!   strength reaches `tool_checked` — the outcome was checked by
//!   a tool, not merely asserted by the run itself. This is the
//!   only failure class that demotes, and a single factual failure
//!   demotes the skill: a skill whose failure was verified is a
//!   skill that misled the run.
//! - **Wilson score interval** (95%) on the success rate, the
//!   same small-sample discipline as PH-5's reviewer calibration.
//!
//! ## States
//!
//! - **Demoted** — at least one factual failure. The skill is
//!   proposed for removal from the run's skill set (a proposal, in
//!   the PH-5 posture: this module records evidence, it does not
//!   edit any skill or config).
//! - **Trusted** — at least [`MIN_SKILL_RUNS`] runs and the Wilson
//!   lower bound at or above [`TRUST_FLOOR`].
//! - **Provisional** — everything else: too few runs, or a lower
//!   bound below the floor, and no factual failure.
//!
//! The thresholds are named proposals, not tuned constants: a
//! fleet with more runs per skill should revisit them. A
//! self-reported failure never demotes — an unverified run cannot
//! indict a skill — and a run recorded before PH-8 (no skills on
//! its row) is excluded from every per-skill count, not attributed
//! to any skill.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::Connection;

use crate::budget::OUTCOME_ROWS_MAX;
use crate::error::SelfImproveError;
use crate::outcome::VerificationStrength;

/// Runs a skill must have before it can be Trusted.
pub const MIN_SKILL_RUNS: usize = 5;
/// Wilson lower bound a skill's success rate must reach to be Trusted.
pub const TRUST_FLOOR: f64 = 0.5;
/// z for the 95% Wilson interval.
const WILSON_Z: f64 = 1.96;

/// One run's trust-relevant evidence: what the outcome row says
/// about the skills it recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTrustRow {
    /// The run's id (for joining back to the row).
    pub run_id: String,
    /// Recorded run status (`ok`, `failed`, …).
    pub status: String,
    /// The run's verification strength.
    pub verification_strength: VerificationStrength,
    /// Skills the run recorded loading (empty for pre-PH-8 rows).
    pub skills: Vec<String>,
}

/// A skill's standing, recomputed from its runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillState {
    /// A factual failure was verified against this skill.
    Demoted,
    /// Enough runs, Wilson lower bound at or above [`TRUST_FLOOR`].
    Trusted,
    /// Too few runs or a lower bound below the floor, no factual failure.
    Provisional,
}

/// Per-skill trust evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillEvidence {
    /// The skill's name, as recorded on the rows.
    pub skill: String,
    /// Runs that recorded this skill.
    pub runs: usize,
    /// Of those, status `ok`.
    pub successes: usize,
    /// Of those, status `failed`.
    pub failures: usize,
    /// Of the failures, strength `tool_checked` or stronger.
    pub factual_failures: usize,
    /// Wilson 95% lower bound on the success rate.
    pub wilson_lower: f64,
    /// The recomputed state.
    pub state: SkillState,
}

/// Wilson score interval (95%) for `successes` of `trials`: the
/// small-sample success-rate interval PH-5 uses for reviewers,
/// here per skill. Zero trials is `(0.0, 0.0)` — no evidence, no
/// claim.
pub fn wilson_interval(successes: usize, trials: usize) -> (f64, f64) {
    if trials == 0 {
        return (0.0, 0.0);
    }
    let n = trials as f64;
    let phat = successes as f64 / n;
    let z2 = WILSON_Z * WILSON_Z;
    let denominator = 1.0 + z2 / n;
    let centre = phat + z2 / (2.0 * n);
    let margin = WILSON_Z * ((phat * (1.0 - phat) + z2 / (4.0 * n)) / n).sqrt();
    (
        (centre - margin) / denominator,
        (centre + margin) / denominator,
    )
}

/// Load the trust-relevant columns of every outcome row, oldest
/// first (the loader reads the store read-only and touches no other
/// table). A database written before PH-8 has no `skills_json`
/// column: its rows load with empty skills — they are not evidence
/// for or against any skill, and the report says so.
///
/// # Errors
/// [`SelfImproveError::Database`] on any database failure.
pub fn load_skill_rows(db_path: &Path) -> Result<Vec<SkillTrustRow>, SelfImproveError> {
    let conn = Connection::open(db_path)?;
    let has_skills: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('run_outcomes') \
         WHERE name = 'skills_json'",
        [],
        |row| row.get(0),
    )?;
    let mut statement = if has_skills > 0 {
        conn.prepare(
            "SELECT run_id, status, verification_strength, skills_json \
             FROM run_outcomes ORDER BY id ASC LIMIT ?",
        )?
    } else {
        conn.prepare(
            "SELECT run_id, status, verification_strength \
             FROM run_outcomes ORDER BY id ASC LIMIT ?",
        )?
    };
    let limit = i64::try_from(OUTCOME_ROWS_MAX).unwrap_or(i64::MAX);
    let mapped = statement.query_map([limit], |row| {
        let strength_text: String = row.get(2)?;
        let skills: Vec<String> = if has_skills > 0 {
            let json: String = row.get(3)?;
            crate::outcome::parse_string_array(&json)
        } else {
            Vec::new()
        };
        Ok(SkillTrustRow {
            run_id: row.get(0)?,
            status: row.get(1)?,
            verification_strength: VerificationStrength::parse(&strength_text)
                .unwrap_or(VerificationStrength::SelfReported),
            skills,
        })
    })?;
    let mut rows = Vec::new();
    for row in mapped {
        rows.push(row?);
    }
    Ok(rows)
}

/// Whether a failure was verified strongly enough to indict a
/// skill: `failed` with a strength of `tool_checked` or stronger.
fn is_factual_failure(row: &SkillTrustRow) -> bool {
    row.status == "failed"
        && matches!(
            row.verification_strength,
            VerificationStrength::ToolChecked
                | VerificationStrength::IndependentlyCrossChecked
                | VerificationStrength::OwnerJudged
        )
}

/// Recompute every skill's evidence from the loaded rows. Skills
/// are named in the order they first appear across the rows.
pub fn skill_evidence(rows: &[SkillTrustRow]) -> Vec<SkillEvidence> {
    let mut by_skill: BTreeMap<&str, (usize, usize, usize, usize)> = BTreeMap::new();
    let mut order: Vec<&str> = Vec::new();
    for row in rows {
        for skill in &row.skills {
            let entry = by_skill.entry(skill.as_str()).or_insert_with(|| {
                order.push(skill.as_str());
                (0, 0, 0, 0)
            });
            entry.0 += 1;
            if row.status == "ok" {
                entry.1 += 1;
            } else if row.status == "failed" {
                entry.2 += 1;
                if is_factual_failure(row) {
                    entry.3 += 1;
                }
            }
        }
    }
    order
        .into_iter()
        .map(|skill| {
            let (runs, successes, failures, factual_failures) = by_skill[skill];
            let (wilson_lower, _) = wilson_interval(successes, runs);
            let state = if factual_failures > 0 {
                SkillState::Demoted
            } else if runs >= MIN_SKILL_RUNS && wilson_lower >= TRUST_FLOOR {
                SkillState::Trusted
            } else {
                SkillState::Provisional
            };
            SkillEvidence {
                skill: skill.to_owned(),
                runs,
                successes,
                failures,
                factual_failures,
                wilson_lower,
                state,
            }
        })
        .collect()
}

/// Render the trust report: method, the empty-evidence gap, and
/// the per-skill table. Pure — the caller owns the write.
pub fn render_trust_report(rows: &[SkillTrustRow], evidence: &[SkillEvidence]) -> String {
    let with_skills = rows.iter().filter(|row| !row.skills.is_empty()).count();
    let mut out = String::new();
    out.push_str("# Skill trust from run rows\n\n");
    out.push_str(&format!(
        "Evidence: {} run rows loaded; {} record at least one skill. \
         Rows from before PH-8 record no skills and are excluded from \
         every per-skill count below — they are not evidence for or \
         against any skill, and nothing is backfilled.\n\n",
        rows.len(),
        with_skills,
    ));
    out.push_str(
        "Method: a run is a success when its recorded status is `ok`, \
         a failure when it is `failed`; other statuses are neither. A \
         **factual failure** is a `failed` run whose verification \
         strength is `tool_checked` or stronger — the outcome was \
         checked by a tool, not merely asserted. One factual failure \
         demotes a skill regardless of its successes; a self-reported \
         failure never demotes. A skill is **Trusted** with at least \
         5 runs and a Wilson 95% lower bound at or above 0.5; anything \
         else with no factual failure is **Provisional**. These \
         thresholds are named proposals (MIN_SKILL_RUNS = 5, \
         TRUST_FLOOR = 0.5), to be revisited as the corpus grows. \
         Demotion is a proposal the operator applies by hand: this \
         report records evidence, it edits nothing.\n\n",
    );
    if evidence.is_empty() {
        out.push_str(
            "No skill has any runs in this store. That is the honest \
             state today: no surface loads skills into a run yet, so \
             the `skills` column is empty for the rows here and the \
             table below has nothing to rank. The recompute above is \
             exercised by the crate's safety-case tests, not by live \
             evidence.\n",
        );
        return out;
    }
    out.push_str(
        "| Skill | Runs | Successes | Failures | Factual failures | Wilson lower | State |\n\
         |---|---|---|---|---|---|---|\n",
    );
    for item in evidence {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {:.2} | {} |\n",
            item.skill,
            item.runs,
            item.successes,
            item.failures,
            item.factual_failures,
            item.wilson_lower,
            match item.state {
                SkillState::Demoted => "Demoted",
                SkillState::Trusted => "Trusted",
                SkillState::Provisional => "Provisional",
            },
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        run_id: &str,
        status: &str,
        strength: VerificationStrength,
        skills: &[&str],
    ) -> SkillTrustRow {
        SkillTrustRow {
            run_id: run_id.to_owned(),
            status: status.to_owned(),
            verification_strength: strength,
            skills: skills.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn wilson_interval_brackets_small_samples() {
        // 5 of 5: the lower bound clears the trust floor; 3 of 5 does not.
        let (lower, upper) = wilson_interval(5, 5);
        assert!(lower >= TRUST_FLOOR);
        assert!(upper <= 1.0 + f64::EPSILON);
        let (lower, _) = wilson_interval(3, 5);
        assert!(lower < TRUST_FLOOR);
        assert_eq!(wilson_interval(0, 0), (0.0, 0.0));
    }

    #[test]
    fn one_factual_failure_demotes_despite_many_successes() {
        let mut rows: Vec<SkillTrustRow> = (0..9)
            .map(|i| {
                row(
                    &format!("run-ok-{i}"),
                    "ok",
                    VerificationStrength::ToolChecked,
                    &["good"],
                )
            })
            .collect();
        rows.push(row(
            "run-bad",
            "failed",
            VerificationStrength::ToolChecked,
            &["good"],
        ));
        let evidence = skill_evidence(&rows);
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].runs, 10);
        assert_eq!(evidence[0].successes, 9);
        assert_eq!(evidence[0].factual_failures, 1);
        assert_eq!(evidence[0].state, SkillState::Demoted);
    }

    #[test]
    fn a_self_reported_failure_does_not_demote() {
        let mut rows: Vec<SkillTrustRow> = (0..9)
            .map(|i| {
                row(
                    &format!("run-ok-{i}"),
                    "ok",
                    VerificationStrength::ToolChecked,
                    &["good"],
                )
            })
            .collect();
        rows.push(row(
            "run-unproven",
            "failed",
            VerificationStrength::SelfReported,
            &["good"],
        ));
        let evidence = skill_evidence(&rows);
        assert_eq!(evidence[0].failures, 1);
        assert_eq!(evidence[0].factual_failures, 0);
        assert_eq!(evidence[0].state, SkillState::Trusted);
    }

    #[test]
    fn too_few_runs_stays_provisional() {
        let rows: Vec<SkillTrustRow> = (0..4)
            .map(|i| {
                row(
                    &format!("run-{i}"),
                    "ok",
                    VerificationStrength::ToolChecked,
                    &["new"],
                )
            })
            .collect();
        let evidence = skill_evidence(&rows);
        assert_eq!(evidence[0].state, SkillState::Provisional);
    }

    #[test]
    fn abandoned_runs_are_neither_success_nor_failure() {
        let rows = vec![
            row(
                "run-ok",
                "ok",
                VerificationStrength::ToolChecked,
                &["mixed"],
            ),
            row(
                "run-set-aside",
                "abandoned",
                VerificationStrength::ToolChecked,
                &["mixed"],
            ),
        ];
        let evidence = skill_evidence(&rows);
        assert_eq!(evidence[0].runs, 2);
        assert_eq!(evidence[0].successes, 1);
        assert_eq!(evidence[0].failures, 0);
    }

    #[test]
    fn loader_reads_a_pre_ph8_database_with_empty_skills() {
        let dir =
            std::env::temp_dir().join(format!("phlow-skill-trust-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("test setup: create temp dir");
        let path = dir.join("feedback.db");
        {
            let conn = Connection::open(&path).expect("conn");
            conn.execute_batch(
                "CREATE TABLE run_outcomes (
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
                INSERT INTO run_outcomes (run_id, task_class, plan, checks_json,
                    reviewer_verdict, cycles, tool_calls, model_calls,
                    context_chars, wall_ms, model_ids_json, status,
                    verification_strength, origin_lane, content_label,
                    evidence, created_at)
                VALUES ('run-old', 'flow_run', 'p', '[]', 'approved', 1, 0, 0,
                    NULL, 7, '[\"m\"]', 'ok', 'tool_checked', 'phlow', 'nsfw',
                    'old', '2026-01-01T00:00:00');",
            )
            .expect("pre-ph8 table");
        }
        let rows = load_skill_rows(&path).expect("loader");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].skills.is_empty());
        assert!(skill_evidence(&rows).is_empty());
    }
}
