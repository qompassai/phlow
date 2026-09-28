//! Task 113 — edit-budget accounting evasion (rust, adversarial).
//!
//! The paper's edit budget is `L_t` ops per step and a per-edit payload
//! bound. This task attacks the accounting at `L_t = 2`: a huge
//! whole-document replace, a whole-document `insert_after` anchor, a
//! newline-joined multi-edit smuggle, and empty/duplicate anchor games.
//! Every one must be rejected with a TYPED error
//! ([`EditError::PayloadTooLarge`], [`EditError::MultiLinePayload`],
//! [`EditError::EmptySpan`], [`EditError::AnchorAmbiguous`]), and every
//! step of the constant / cosine / autonomous schedule arms must satisfy
//! `applied_ops ≤ L_t` and `tokens_per_edit ≤ PER_EDIT_TOKENS_MAX`.
//! Schedule comparison at the same bound: the spread of mean final
//! D_test across the three schedules must be < 1.5 points, or
//! schedule-insensitivity fails. All verdict evidence comes from the
//! clearly labeled scripted double (MOCK).

use crate::skillopt::doc::{Edit, EditError, EditOp, PER_EDIT_TOKENS_MAX, SkillDoc};
use crate::skillopt::driver::{CaseReport, TaskDriverError, base_config, verdict_line};
use crate::skillopt::learner::{Learner, LearnerConfig, LtSchedule, SeedLog, Verdict, mean_std};
use crate::skillopt::optimizer::ScriptedOptimizer;
use crate::skillopt::target::Family;
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-113";
/// Task name.
pub const NAME: &str = "edit-budget accounting evasion";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "reject_evasion_attacks",
    "schedule_spread",
    "accounting_invariants",
    "autonomous_schedule_behavior",
];
/// The edit budget under attack.
pub const L_T: usize = 2;
/// Schedule-insensitivity bar: spread of mean final D_test (points).
pub const SPREAD_PTS_MAX: f64 = 1.5;

fn mk_edit(op: EditOp) -> Edit {
    Edit {
        op,
        rationale: "task-113 attack fixture".to_string(),
        direction: "attack".to_string(),
    }
}

/// Attack body: three lines, the first repeated, so the duplicate-anchor
/// game has a genuinely ambiguous anchor.
const ATTACK_BODY: &str =
    "rule one: fill orders promptly\nrule two: confirm quantities\nrule one: fill orders promptly";

fn attack_doc() -> SkillDoc {
    SkillDoc::experiment(ATTACK_BODY)
}

/// The accounting-evasion attacks and the typed error each must produce.
/// Returns (attack_name, edit, expected-error predicate).
type Attack = (String, Edit, fn(&EditError) -> bool);
fn attacks() -> Vec<Attack> {
    vec![
        (
            "huge-whole-document-replace".to_string(),
            mk_edit(EditOp::Replace {
                old: ATTACK_BODY.to_string(),
                new: "x".repeat(500),
            }),
            (|e| matches!(e, EditError::PayloadTooLarge { .. })) as fn(&EditError) -> bool,
        ),
        (
            "whole-document-insert-anchor".to_string(),
            mk_edit(EditOp::InsertAfter {
                // A huge single-line anchor: anchors count toward the
                // payload, so bulk smuggled in the anchor (instead of the
                // line) still hits the token bound. (A literally
                // whole-document multi-line anchor is additionally
                // rejected by the one-line rule.)
                anchor: "x".repeat(500),
                line: "smuggled".to_string(),
            }),
            (|e| matches!(e, EditError::PayloadTooLarge { .. })) as fn(&EditError) -> bool,
        ),
        (
            "newline-joined-multi-edit".to_string(),
            mk_edit(EditOp::Append {
                line: "rule a: one\nrule b: two\nrule c: three".to_string(),
            }),
            (|e| matches!(e, EditError::MultiLinePayload { .. })) as fn(&EditError) -> bool,
        ),
        (
            "empty-anchor".to_string(),
            mk_edit(EditOp::InsertAfter {
                anchor: String::new(),
                line: "smuggled".to_string(),
            }),
            (|e| matches!(e, EditError::EmptySpan { .. })) as fn(&EditError) -> bool,
        ),
        (
            "duplicate-anchor".to_string(),
            mk_edit(EditOp::InsertAfter {
                anchor: "rule one: fill orders promptly".to_string(),
                line: "smuggled".to_string(),
            }),
            (|e| matches!(e, EditError::AnchorAmbiguous { .. })) as fn(&EditError) -> bool,
        ),
    ]
}

/// Arm configuration: F-order standard loop at `L_t = 2`, schedule swapped.
fn arm_cfg(schedule: LtSchedule) -> LearnerConfig {
    let mut cfg = base_config();
    cfg.families = vec![Family::FOrder];
    cfg.schedule = schedule;
    cfg
}

/// Run one schedule arm. Returns (schedule_name, seed logs).
fn run_schedule(
    name: &str,
    schedule: LtSchedule,
) -> Result<(String, Vec<SeedLog>), TaskDriverError> {
    let cfg = arm_cfg(schedule);
    let opt = ScriptedOptimizer::new();
    let logs = Learner
        .run_arm(&cfg, &opt)
        .map_err(|e| TaskDriverError::Arm {
            arm: name.to_string(),
            detail: e.to_string(),
        })?;
    if logs.is_empty() {
        return Err(TaskDriverError::Arm {
            arm: name.to_string(),
            detail: "no seed logs".to_string(),
        });
    }
    Ok((name.to_string(), logs))
}

/// Run all three schedule arms at the same bound.
fn run_schedules() -> Result<Vec<(String, Vec<SeedLog>)>, TaskDriverError> {
    Ok(vec![
        run_schedule("constant", LtSchedule::Constant(L_T))?,
        run_schedule("cosine", LtSchedule::Cosine { from: L_T, to: L_T })?,
        run_schedule("autonomous", LtSchedule::Autonomous { cap: L_T })?,
    ])
}

/// Mean final D_test (points) per schedule arm.
fn schedule_means(arms: &[(String, Vec<SeedLog>)]) -> Vec<(String, f64)> {
    arms.iter()
        .map(|(name, logs)| {
            let vals: Vec<f64> = logs.iter().map(|l| l.d_test * 100.0).collect();
            let (m, _) = mean_std(&vals);
            (name.clone(), m)
        })
        .collect()
}

/// V1: the evasion attacks are rejected with the named typed errors.
fn case_reject_evasion_attacks() -> Result<CaseReport, TaskDriverError> {
    let mut evidence = Vec::new();
    let mut failures = Vec::new();
    for (name, edit, expects) in attacks() {
        let mut doc = attack_doc();
        match doc.apply(&edit) {
            Err(e) if expects(&e) => {
                evidence.push(format!("attack {name}: rejected with {e}"));
            }
            Err(e) => {
                failures.push(format!("attack {name}: wrong error type: {e}"));
            }
            Ok(()) => {
                failures.push(format!(
                    "attack {name}: APPLIED — accounting evasion succeeded"
                ));
            }
        }
    }
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "attacks": attacks().len(),
            "rejected": attacks().len() - failures.len(),
            "l_t": L_T,
            "per_edit_tokens_max": PER_EDIT_TOKENS_MAX,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: schedule comparison at the same bound — spread of mean final
/// D_test across constant / cosine / autonomous must be < 1.5 points.
fn case_schedule_spread() -> Result<CaseReport, TaskDriverError> {
    let arms = run_schedules()?;
    let means = schedule_means(&arms);
    let vals: Vec<f64> = means.iter().map(|(_, m)| *m).collect();
    let spread = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
        - vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let mut evidence: Vec<String> = means
        .iter()
        .map(|(n, m)| format!("schedule {n}: mean final D_test = {m:.2}"))
        .collect();
    evidence.push(format!(
        "spread = {spread:.2} points (bar: < {SPREAD_PTS_MAX})"
    ));
    let mut failures = Vec::new();
    for (name, logs) in &arms {
        if logs.len() < 5 {
            failures.push(format!("schedule {name}: fewer than 5 seeds"));
        }
    }
    let verdict = if spread < SPREAD_PTS_MAX {
        Verdict::Replicates
    } else {
        Verdict::Negative
    };
    let detail = format!(
        "schedule spread {spread:.2} points vs bar {SPREAD_PTS_MAX}: {}",
        if spread < SPREAD_PTS_MAX {
            "schedule-insensitive"
        } else {
            "SCHEDULE-SENSITIVE"
        }
    );
    evidence.push(verdict_line("113", verdict, &detail));
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "verdict": verdict.to_string(),
            "means": means.iter().map(|(n, m)| serde_json::json!({"schedule": n, "mean_d_test": m})).collect::<Vec<_>>(),
            "spread": spread,
            "spread_bar": SPREAD_PTS_MAX,
            "detail": detail,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1 (adversarial): the per-step accounting invariants hold on every
/// step of every arm — `applied_ops ≤ L_t` and
/// `tokens_per_edit ≤ PER_EDIT_TOKENS_MAX`. The attacker wants a step
/// that applies more ops than the budget or a heavier edit than the
/// bound; the audit walks the recorded steps.
fn case_accounting_invariants() -> Result<CaseReport, TaskDriverError> {
    let arms = run_schedules()?;
    let mut evidence = Vec::new();
    let mut failures = Vec::new();
    let mut steps_total = 0usize;
    for (name, logs) in &arms {
        for log in logs {
            for st in &log.steps {
                steps_total += 1;
                if st.n_applied > st.l_t {
                    failures.push(format!(
                        "schedule {name} seed {} step {}: applied {} ops > L_t {}",
                        log.seed, st.step, st.n_applied, st.l_t
                    ));
                }
                if st.n_applied > 0 && st.max_edit_tokens > PER_EDIT_TOKENS_MAX {
                    failures.push(format!(
                        "schedule {name} seed {} step {}: edit tokens {} > max {}",
                        log.seed, st.step, st.max_edit_tokens, PER_EDIT_TOKENS_MAX
                    ));
                }
            }
        }
    }
    evidence.push(format!(
        "audited {steps_total} steps across 3 schedules: applied_ops ≤ L_t and \
         tokens_per_edit ≤ {PER_EDIT_TOKENS_MAX} on every applied step"
    ));
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "steps_audited": steps_total,
            "violations": failures.len(),
            "per_edit_tokens_max": PER_EDIT_TOKENS_MAX,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2 (adversarial): the autonomous controller can never evade its cap.
/// Unit-check the update rule at the boundaries, then verify every
/// recorded step budget stays in `[1, cap]` on the arm.
fn case_autonomous_schedule_behavior() -> Result<CaseReport, TaskDriverError> {
    let mut evidence = Vec::new();
    let mut failures = Vec::new();
    // Unit: the controller rule, no double involved.
    let unit_cases = [
        (L_T, None, L_T, "first step starts at cap"),
        (L_T, Some(true), L_T, "accept at cap stays capped"),
        (L_T, Some(false), L_T - 1, "reject at cap steps down"),
        (1, Some(false), 1, "reject at floor stays floored"),
        (1, Some(true), 2, "accept at floor steps up"),
    ];
    for (current, prev, want, note) in unit_cases {
        let got = LtSchedule::autonomous_next(L_T, current, prev);
        if got != want {
            failures.push(format!("controller: {note}: got {got}, want {want}"));
        } else {
            evidence.push(format!("controller: {note}: {current} -> {got}"));
        }
    }
    // Config-level: cap 0 must be rejected before any step runs.
    let mut bad_cfg = arm_cfg(LtSchedule::Autonomous { cap: 0 });
    bad_cfg.seeds = vec![101];
    let bad_opt = ScriptedOptimizer::new();
    match Learner.run_arm(&bad_cfg, &bad_opt) {
        Err(e) if e.to_string().contains("autonomous cap 0") => {
            evidence.push("config: Autonomous{cap: 0} rejected: ".to_string() + &e.to_string());
        }
        Err(e) => {
            failures.push(format!("config: cap 0 rejected with wrong error: {e}"));
        }
        Ok(_) => {
            failures.push("config: Autonomous{cap: 0} was NOT rejected".to_string());
        }
    }
    // Arm-level: every step budget in [1, cap].
    let arms = run_schedules()?;
    let auto_logs = arms
        .iter()
        .find(|(n, _)| n == "autonomous")
        .map(|(_, l)| l)
        .ok_or_else(|| TaskDriverError::Fixture {
            what: "arm".to_string(),
            detail: "task-113: autonomous arm missing".to_string(),
        })?;
    let mut budgets = Vec::new();
    for log in auto_logs {
        for st in &log.steps {
            budgets.push(st.l_t);
            if st.l_t < 1 || st.l_t > L_T {
                failures.push(format!(
                    "seed {} step {}: budget {} outside [1, {L_T}]",
                    log.seed, st.step, st.l_t
                ));
            }
        }
    }
    let distinct: std::collections::BTreeSet<usize> = budgets.iter().cloned().collect();
    evidence.push(format!(
        "autonomous arm: {} steps, budgets observed {{{}}} (cap {L_T})",
        budgets.len(),
        distinct
            .iter()
            .map(|b| b.to_string())
            .collect::<Vec<_>>()
            .join(",")
    ));
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "unit_cases": unit_cases.len(),
            "steps": budgets.len(),
            "budgets_observed": distinct.into_iter().collect::<Vec<_>>(),
            "cap": L_T,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "reject_evasion_attacks" => case_reject_evasion_attacks(),
        "schedule_spread" => case_schedule_spread(),
        "accounting_invariants" => case_accounting_invariants(),
        "autonomous_schedule_behavior" => case_autonomous_schedule_behavior(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-113: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[1]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-113".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-113".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
