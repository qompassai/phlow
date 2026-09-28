//! task-101: edit-budget bound ablation (rust, V).
//!
//! **Seam:** the edit-application path: merge → global rank → truncate
//! to L_t → apply.
//!
//! **Dimension:** the paper's claim that *the existence of a bound*
//! matters more than its schedule: "L_t in {1, 2, 4, 8, 16} are all
//! competitive, but **removing the bound entirely** costs 2 to 4 points
//! on average" (arXiv 2605.23904v2 §III.3).
//!
//! **Scenarios:** full loop on F-order, 2 epochs, 5 seeds, under
//! L_t ∈ {1, cosine 4→2 (the paper default), 16, unbounded}. The
//! unbounded arm applies every proposed edit each step, with no gate.
//! The optimizer is fixed across arms (same seed stream) so only the
//! bound varies — the proposal stream is arm-independent; truncation
//! happens in the learner.
//!
//! **Pass criteria (pre-registered):** *replicates* = unbounded costs
//! ≥2.0 D_test points vs L_t=4 (mean over seeds) AND the L_t∈{1,4,16}
//! spread is <2.0 points; *null* = unbounded within ±1.0 of L_t=4;
//! *negative* = unbounded beats L_t=4 by ≥2.0. Per-step accepted-edit
//! counts and token churn are reported: the unbounded arm must show the
//! "rewrite everything" pathology (large churn per step).
//!
//! **Primary evidence:** the real [`ModelOptimizer`] via primo's local
//! Ollama HTTP API (`think: false`, `/api/generate`). The scripted mock
//! is the offline fallback/control only, labeled as such.
//!
//! Four cases: two validation, two adversarial.
//! - V1 `arms_complete_and_classified`: all four arms run and the
//!   preregistered verdict classifies. PASSES on any verdict —
//!   null/negative are first-class measurements.
//! - V2 `truncation_bound_holds`: bounded arms never apply more than
//!   L_t edits on an accepted step. PASSES.
//! - A1 `unbounded_churn_pathology`: the unbounded arm's mean churn per
//!   step exceeds the L_t=1 arm's — the pathology must be observable in
//!   the records, not smoothed away. PASSES iff measured.
//! - A2 `untrusted_doc_cannot_enter_loop`: an untrusted document is
//!   refused before any rollout. PASSES.

use crate::skillopt::driver::{
    ArmSummary, Backend, CaseReport, TaskDriverError, base_config, resolve_primary, run_arm_named,
    summarize, untrusted_is_refused, verdict_line,
};
use crate::skillopt::learner::{LtSchedule, SeedLog, Verdict, mean_std};
use crate::skillopt::optimizer::ScriptedOptimizer;
use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};

/// Task id.
pub const ID: &str = "task-101";
/// Human-readable name.
pub const NAME: &str = "edit-budget bound ablation";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order: two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "arms_complete_and_classified",
    "truncation_bound_holds",
    "unbounded_churn_pathology",
    "untrusted_doc_cannot_enter_loop",
];

/// The four arms: (name, schedule).
fn arms() -> Vec<(&'static str, LtSchedule)> {
    vec![
        ("lt1", LtSchedule::Constant(1)),
        ("lt4-cosine", LtSchedule::Cosine { from: 4, to: 2 }),
        ("lt16", LtSchedule::Constant(16)),
        ("unbounded", LtSchedule::Unbounded),
    ]
}

/// One arm's evidence: summary plus churn/applied-count series.
struct ArmData {
    summary: ArmSummary,
    /// Mean applied edits per step, over seeds.
    applied_per_step: f64,
    /// Mean churn chars per step, over seeds.
    churn_per_step: f64,
}

fn arm_data(name: &str, logs: Vec<SeedLog>) -> ArmData {
    let mut applied = Vec::new();
    let mut churn = Vec::new();
    for log in &logs {
        let n = log.steps.len().max(1) as f64;
        applied.push(log.steps.iter().map(|s| s.n_applied).sum::<usize>() as f64 / n);
        churn.push(log.steps.iter().map(|s| s.churn_chars).sum::<usize>() as f64 / n);
    }
    let (applied_per_step, _) = mean_std(&applied);
    let (churn_per_step, _) = mean_std(&churn);
    ArmData {
        summary: summarize(name, &logs),
        applied_per_step,
        churn_per_step,
    }
}

/// Run all four arms under `backend`.
fn run_arms(backend: &Backend) -> Result<Vec<ArmData>, TaskDriverError> {
    let mut out = Vec::new();
    for (name, schedule) in arms() {
        let mut cfg = base_config();
        cfg.schedule = schedule;
        let (_, logs) = run_arm_named(name, &cfg, backend)?;
        out.push(arm_data(name, logs));
    }
    Ok(out)
}

/// Classify the preregistered verdict from the arm summaries.
/// Deltas are in D_test points (fraction × 100).
fn classify(arm_list: &[ArmData]) -> (Verdict, String) {
    let get = |name: &str| {
        arm_list
            .iter()
            .find(|a| a.summary.name == name)
            .map(|a| a.summary.mean)
            .unwrap_or(f64::NAN)
    };
    let (lt1, lt4, lt16, unb) = (get("lt1"), get("lt4-cosine"), get("lt16"), get("unbounded"));
    let cost = lt4 - unb;
    let spread = lt1.max(lt4).max(lt16) - lt1.min(lt4).min(lt16);
    let detail = format!(
        "unbounded cost vs L_t=4: {cost:+.2} pts; bounded spread: {spread:.2} pts \
         (lt1={lt1:.2}, lt4={lt4:.2}, lt16={lt16:.2}, unbounded={unb:.2})"
    );
    let verdict = if cost >= 2.0 && spread < 2.0 {
        Verdict::Replicates
    } else if cost.abs() <= 1.0 {
        Verdict::Null
    } else if cost <= -2.0 {
        Verdict::Negative
    } else {
        Verdict::Indeterminate
    };
    (verdict, detail)
}

/// Evidence lines shared by the driver and V1.
fn evidence_lines(
    backend: &Backend,
    arm_list: &[ArmData],
    verdict: Verdict,
    detail: &str,
) -> Vec<String> {
    let mut ev = vec![
        format!("backend: {}", backend.label()),
        "seam: REAL SkillOpt loop (scripted target F-order, fixed splits); \
         optimizer fixed across arms, only L_t varies"
            .to_string(),
        verdict_line("101", verdict, detail),
    ];
    for a in arm_list {
        ev.push(format!(
            "arm {}: D_test {:.2}±{:.2} pts (n={}); applied {:.2}/step; churn {:.0} chars/step",
            a.summary.name,
            a.summary.mean,
            a.summary.std,
            a.summary.n,
            a.applied_per_step,
            a.churn_per_step,
        ));
    }
    ev
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

fn case_arms_complete_and_classified() -> Result<CaseReport, TaskDriverError> {
    let backend = Backend::ScriptedFallback(ScriptedOptimizer::new());
    let arm_list = run_arms(&backend)?;
    let (verdict, detail) = classify(&arm_list);
    let metrics = serde_json::json!({
        "verdict": verdict.to_string(),
        "arms": arm_list.iter().map(|a| serde_json::json!({
            "name": a.summary.name,
            "d_test_mean_pts": a.summary.mean,
            "d_test_std_pts": a.summary.std,
            "applied_per_step": a.applied_per_step,
            "churn_per_step": a.churn_per_step,
        })).collect::<Vec<_>>(),
    });
    Ok(CaseReport::pass(
        "arms_complete_and_classified",
        metrics,
        evidence_lines(&backend, &arm_list, verdict, &detail),
    ))
}

fn case_truncation_bound_holds() -> Result<CaseReport, TaskDriverError> {
    let backend = Backend::ScriptedFallback(ScriptedOptimizer::new());
    let mut evidence = Vec::new();
    for (bound, schedule) in [
        (1usize, LtSchedule::Constant(1)),
        (16, LtSchedule::Constant(16)),
    ] {
        let mut cfg = base_config();
        cfg.schedule = schedule;
        let (_, logs) = run_arm_named("bound-check", &cfg, &backend)?;
        let mut violations = 0usize;
        let mut checked = 0usize;
        for log in &logs {
            for step in &log.steps {
                if step.accepted {
                    checked += 1;
                    if step.n_applied > bound {
                        violations += 1;
                    }
                }
            }
        }
        evidence.push(format!(
            "L_t={bound}: {checked} accepted steps, {violations} over-bound applications"
        ));
        if violations > 0 {
            return Ok(CaseReport::fail(
                "truncation_bound_holds",
                format!("L_t={bound}: {violations} accepted steps applied more than L_t edits"),
                evidence,
            ));
        }
    }
    Ok(CaseReport::pass(
        "truncation_bound_holds",
        serde_json::json!({"bounds": [1, 16], "violations": 0}),
        evidence,
    ))
}

fn case_unbounded_churn_pathology() -> Result<CaseReport, TaskDriverError> {
    let backend = Backend::ScriptedFallback(ScriptedOptimizer::new());
    let arm_list = run_arms(&backend)?;
    let (lt1, unb) = (
        arm_list.iter().find(|a| a.summary.name == "lt1").unwrap(),
        arm_list
            .iter()
            .find(|a| a.summary.name == "unbounded")
            .unwrap(),
    );
    let evidence = vec![
        format!("lt1 churn/step: {:.0} chars", lt1.churn_per_step),
        format!("unbounded churn/step: {:.0} chars", unb.churn_per_step),
        format!("lt1 applied/step: {:.2}", lt1.applied_per_step),
        format!("unbounded applied/step: {:.2}", unb.applied_per_step),
    ];
    if unb.churn_per_step > lt1.churn_per_step {
        Ok(CaseReport::pass(
            "unbounded_churn_pathology",
            serde_json::json!({
                "lt1_churn_per_step": lt1.churn_per_step,
                "unbounded_churn_per_step": unb.churn_per_step,
            }),
            evidence,
        ))
    } else {
        Ok(CaseReport::fail(
            "unbounded_churn_pathology",
            "unbounded arm does not churn more per step than L_t=1 — \
             the rewrite-everything pathology is not observable"
                .to_string(),
            evidence,
        ))
    }
}

fn case_untrusted_doc_cannot_enter_loop() -> Result<CaseReport, TaskDriverError> {
    let cfg = base_config();
    match untrusted_is_refused(&cfg) {
        Ok(()) => Ok(CaseReport::pass(
            "untrusted_doc_cannot_enter_loop",
            serde_json::json!({"refused": true}),
            vec![
                "import_untrusted document refused with SafetyBoundary before any rollout"
                    .to_string(),
            ],
        )),
        Err(detail) => Ok(CaseReport::fail(
            "untrusted_doc_cannot_enter_loop",
            detail,
            vec![],
        )),
    }
}

/// Run one case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "arms_complete_and_classified" => case_arms_complete_and_classified(),
        "truncation_bound_holds" => case_truncation_bound_holds(),
        "unbounded_churn_pathology" => case_unbounded_churn_pathology(),
        "untrusted_doc_cannot_enter_loop" => case_untrusted_doc_cannot_enter_loop(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("unknown case '{case}'"),
        }),
    }
}

// ---------------------------------------------------------------------------
// Task entry point
// ---------------------------------------------------------------------------

struct TaskFailure {
    where_: String,
    how: String,
    evidence: Vec<String>,
}

/// The primary evidence run: real model when reachable, else the labeled
/// scripted fallback. Any preregistered verdict (replicates/null/
/// negative/indeterminate) is a successful measurement — the task fails
/// only if the apparatus itself breaks.
fn run_inner() -> Result<Vec<String>, TaskFailure> {
    let backend = resolve_primary().map_err(|how| TaskFailure {
        where_: "resolve_backend".to_string(),
        how,
        evidence: Vec::new(),
    })?;
    let arm_list = run_arms(&backend).map_err(|e| TaskFailure {
        where_: "run_arms".to_string(),
        how: e.to_string(),
        evidence: vec![format!("backend: {}", backend.label())],
    })?;
    let (verdict, detail) = classify(&arm_list);
    let mut evidence = evidence_lines(&backend, &arm_list, verdict, &detail);
    if !backend.is_real() {
        evidence.push(
            "NOTE: primary evidence fell back to the scripted mock (primo Ollama \
             unreachable); re-run with GAUNTLET_OLLAMA_URL for model evidence"
                .to_string(),
        );
    }
    Ok(evidence)
}

/// Attempt the task.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_inner() {
        Ok(evidence) => TaskOutcome::Pass {
            evidence: bound_evidence(evidence),
        },
        Err(failure) => TaskOutcome::Fail {
            where_: failure.where_,
            how: failure.how,
            evidence: bound_evidence(failure.evidence),
        },
    }
}
