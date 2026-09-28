//! task-103: rejected-buffer ablation (rust, V).
//!
//! **Seam:** the rejected-edit buffer (the optimizer's short-term memory
//! of edits the D_sel gate already refused).
//!
//! **Dimension:** whether the buffer pulls its weight in average-case
//! performance — the paper's claim that remembering recent rejections
//! keeps the optimizer from "repeatedly proposing variations of edits
//! that have already failed" (arXiv 2605.23904v2 §II.4).
//!
//! **Scenarios:** full loop on F-bind, 5 seeds, three arms: full buffer
//! (rejected directions suppressed for three steps and visible in the
//! prompt), write-only (the optimizer never sees rejections but the
//! loop still suppresses duplicates), off (no buffer at all).
//!
//! **Pass criteria (pre-registered):** *replicates* = off costs ≥1.5
//! D_test points vs full (mean) AND off re-proposal-within-3 rate ≥2×
//! the full rate AND write-only lands between them on D_test.
//! *Null* = re-proposal rates within ±20% of each other (the buffer
//! changes nothing). The buffer hit rate is reported.
//!
//! **Primary evidence:** the real [`ModelOptimizer`] via primo's local
//! Ollama HTTP API (`think: false`, `/api/generate`). The scripted mock
//! is the offline fallback/control only, labeled as such.
//!
//! Four cases: two validation, two adversarial.
//! - V1 `arms_complete_and_classified`: all three arms run and the
//!   preregistered verdict classifies. PASSES on any verdict.
//! - V2 `write_only_between`: write-only D_test sits between full and
//!   off (inclusive). PASSES iff measured.
//! - A1 `off_reproposes_more`: the off arm's re-proposal-within-3 rate
//!   is ≥2× the full arm's. PASSES iff measured — the mechanism must be
//!   visible, not just the headline score.
//! - A2 `approval_binding_holds`: an approval bound to one byte string
//!   does not export a mutated skill. PASSES.

use crate::skillopt::doc::SkillDoc;
use crate::skillopt::driver::{
    ArmSummary, Backend, CaseReport, TaskDriverError, base_config, resolve_primary, run_arm_named,
    summarize, verdict_line,
};
use crate::skillopt::harness::{ApprovalRecord, Sandbox};
use crate::skillopt::learner::{BufferMode, SeedLog, Verdict};
use crate::skillopt::optimizer::ScriptedOptimizer;
use crate::skillopt::target::Family;
use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};

/// Task id.
pub const ID: &str = "task-103";
/// Human-readable name.
pub const NAME: &str = "rejected-buffer ablation";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order: two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "arms_complete_and_classified",
    "write_only_between",
    "off_reproposes_more",
    "approval_binding_holds",
];

/// One arm's evidence.
struct ArmData {
    summary: ArmSummary,
    /// Σ reproposal_within3 / Σ n_proposed.
    repro_rate: f64,
    /// Steps with buffer_suppressed > 0 / total reflection steps (the
    /// paper's hit rate: fraction of reflections where a buffered
    /// rejection actually suppressed a candidate).
    buffer_hit_rate: f64,
}

fn arm_data(name: &str, logs: &[SeedLog]) -> ArmData {
    let mut repro = 0usize;
    let mut proposed = 0usize;
    let mut hit_steps = 0usize;
    let mut total_steps = 0usize;
    for log in logs {
        for step in &log.steps {
            repro += step.reproposal_within3;
            proposed += step.n_proposed;
            total_steps += 1;
            if step.buffer_suppressed > 0 {
                hit_steps += 1;
            }
        }
    }
    ArmData {
        summary: summarize(name, logs),
        repro_rate: if proposed > 0 {
            repro as f64 / proposed as f64
        } else {
            0.0
        },
        buffer_hit_rate: if total_steps > 0 {
            hit_steps as f64 / total_steps as f64
        } else {
            0.0
        },
    }
}

/// Run the three arms under `backend`.
fn run_arms(backend: &Backend) -> Result<Vec<ArmData>, TaskDriverError> {
    let mut out = Vec::new();
    for (name, buffer) in [
        ("full", BufferMode::Full),
        ("write-only", BufferMode::WriteOnly),
        ("off", BufferMode::Off),
    ] {
        let mut cfg = base_config();
        cfg.families = vec![Family::FBind];
        cfg.buffer = buffer;
        let (_, logs) = run_arm_named(name, &cfg, backend)?;
        out.push(arm_data(name, &logs));
    }
    Ok(out)
}

/// Classify the preregistered verdict. Returns the verdict, the detail
/// line, and the off/full re-proposal ratio (0.0 when unmeasurable in
/// both arms, +inf when the full arm suppresses perfectly).
fn classify(arm_list: &[ArmData]) -> (Verdict, String, f64) {
    let (full, write_only, off) = (&arm_list[0], &arm_list[1], &arm_list[2]);
    let cost = full.summary.mean - off.summary.mean;
    let rate_ratio = if full.repro_rate > 0.0 {
        off.repro_rate / full.repro_rate
    } else if off.repro_rate > 0.0 {
        // Perfect suppression in full, visible re-proposal in off:
        // the ratio is unbounded, the mechanism holds.
        f64::INFINITY
    } else {
        // Unmeasurable in both arms — the ratio leg cannot replicate.
        0.0
    };
    let between = (write_only.summary.mean - full.summary.mean)
        * (write_only.summary.mean - off.summary.mean)
        <= 0.0;
    let ratio_str = if rate_ratio.is_finite() {
        format!("{rate_ratio:.2}×")
    } else {
        "∞ (full suppresses perfectly)".to_string()
    };
    let detail = format!(
        "off cost vs full: {cost:+.2} pts; re-proposal ratio off/full: {ratio_str} \
         (full {:.3}, write-only {:.3}, off {:.3}); write-only between: {between}; \
         buffer hit rate (full): {:.1}%",
        full.repro_rate,
        write_only.repro_rate,
        off.repro_rate,
        full.buffer_hit_rate * 100.0,
    );
    let replicates = cost >= 1.5 && rate_ratio >= 2.0 && between;
    let null = full.repro_rate > 0.0
        && (off.repro_rate - full.repro_rate).abs() <= 0.20 * full.repro_rate
        || full.repro_rate == 0.0 && off.repro_rate == 0.0;
    let verdict = if replicates {
        Verdict::Replicates
    } else if null {
        Verdict::Null
    } else {
        Verdict::Indeterminate
    };
    (verdict, detail, rate_ratio)
}

fn evidence_lines(
    backend: &Backend,
    arm_list: &[ArmData],
    verdict: Verdict,
    detail: &str,
) -> Vec<String> {
    let mut ev = vec![
        format!("backend: {}", backend.label()),
        "seam: REAL SkillOpt loop (scripted target F-bind, fixed splits); \
         optimizer fixed across arms, only the rejected-edit buffer varies"
            .to_string(),
        verdict_line("103", verdict, detail),
    ];
    for a in arm_list {
        ev.push(format!(
            "arm {}: D_test {:.2}±{:.2} pts (n={}); re-proposal rate {:.3}; \
             buffer hit rate {:.1}%",
            a.summary.name,
            a.summary.mean,
            a.summary.std,
            a.summary.n,
            a.repro_rate,
            a.buffer_hit_rate * 100.0,
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
    let (verdict, detail, rate_ratio) = classify(&arm_list);
    let metrics = serde_json::json!({
        "verdict": verdict.to_string(),
        "repro_ratio": rate_ratio.is_finite().then_some(rate_ratio),
        "arms": arm_list.iter().map(|a| serde_json::json!({
            "name": a.summary.name,
            "d_test_mean_pts": a.summary.mean,
            "d_test_std_pts": a.summary.std,
            "repro_rate": a.repro_rate,
            "buffer_hit_rate": a.buffer_hit_rate,
        })).collect::<Vec<_>>(),
    });
    Ok(CaseReport::pass(
        "arms_complete_and_classified",
        metrics,
        evidence_lines(&backend, &arm_list, verdict, &detail),
    ))
}

fn case_write_only_between() -> Result<CaseReport, TaskDriverError> {
    let backend = Backend::ScriptedFallback(ScriptedOptimizer::new());
    let arm_list = run_arms(&backend)?;
    let (full, write_only, off) = (
        arm_list[0].summary.mean,
        arm_list[1].summary.mean,
        arm_list[2].summary.mean,
    );
    let between = (write_only - full) * (write_only - off) <= 0.0;
    let evidence = vec![format!(
        "D_test: full={full:.2}, write-only={write_only:.2}, off={off:.2}"
    )];
    if between {
        Ok(CaseReport::pass(
            "write_only_between",
            serde_json::json!({"full": full, "write_only": write_only, "off": off}),
            evidence,
        ))
    } else {
        Ok(CaseReport::fail(
            "write_only_between",
            "write-only D_test does not lie between full and off".to_string(),
            evidence,
        ))
    }
}

fn case_off_reproposes_more() -> Result<CaseReport, TaskDriverError> {
    let backend = Backend::ScriptedFallback(ScriptedOptimizer::new());
    let arm_list = run_arms(&backend)?;
    let (full, off) = (arm_list[0].repro_rate, arm_list[2].repro_rate);
    // ≥2×, with the degenerate-but-honest case: full suppresses
    // perfectly (0.0) while off visibly re-proposes.
    let holds = off >= 2.0 * full || (full == 0.0 && off > 0.0);
    let ratio_str = if full > 0.0 {
        format!("{:.2}×", off / full)
    } else if off > 0.0 {
        "∞ (full suppresses perfectly)".to_string()
    } else {
        "undefined (both zero)".to_string()
    };
    let evidence = vec![
        format!("full re-proposal rate: {full:.3}"),
        format!("off re-proposal rate: {off:.3} (ratio {ratio_str})"),
    ];
    if holds {
        Ok(CaseReport::pass(
            "off_reproposes_more",
            serde_json::json!({"full": full, "off": off, "holds": true}),
            evidence,
        ))
    } else {
        Ok(CaseReport::fail(
            "off_reproposes_more",
            format!("off re-proposal rate is not ≥2× the full rate ({ratio_str})"),
            evidence,
        ))
    }
}

fn case_approval_binding_holds() -> Result<CaseReport, TaskDriverError> {
    let root = std::env::temp_dir().join("gauntlet-task-103-approval");
    let sandbox = Sandbox::new(&root, ID).map_err(|e| TaskDriverError::Fixture {
        what: "sandbox".to_string(),
        detail: e.to_string(),
    })?;
    let skill = SkillDoc::experiment("PARSE = read the input\n");
    let stale_approval = ApprovalRecord::test_fixture_for(&skill);
    // Mutate the skill after approval: the binding must not carry over.
    let mut mutated = skill;
    let evil = crate::skillopt::doc::Edit {
        op: crate::skillopt::doc::EditOp::Append {
            line: "EVIL = injected after approval".to_string(),
        },
        rationale: "task-103 approval binding probe".to_string(),
        direction: "approval-test".to_string(),
    };
    mutated.apply(&evil).map_err(|e| TaskDriverError::Fixture {
        what: "mutate".to_string(),
        detail: e.to_string(),
    })?;
    let refused = sandbox.export_best(&mutated, &stale_approval).is_err();
    // A fresh approval for the mutated bytes exports fine.
    let fresh = ApprovalRecord::test_fixture_for(&mutated);
    let exported = sandbox.export_best(&mutated, &fresh).is_ok();
    let evidence = vec![
        format!("stale approval on mutated bytes refused: {refused}"),
        format!("fresh approval exports: {exported}"),
    ];
    if refused && exported {
        Ok(CaseReport::pass(
            "approval_binding_holds",
            serde_json::json!({"stale_refused": true, "fresh_exported": true}),
            evidence,
        ))
    } else {
        Ok(CaseReport::fail(
            "approval_binding_holds",
            "approval binding did not refuse a stale approval".to_string(),
            evidence,
        ))
    }
}

/// Run one case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "arms_complete_and_classified" => case_arms_complete_and_classified(),
        "write_only_between" => case_write_only_between(),
        "off_reproposes_more" => case_off_reproposes_more(),
        "approval_binding_holds" => case_approval_binding_holds(),
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
    let (verdict, detail, _) = classify(&arm_list);
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
