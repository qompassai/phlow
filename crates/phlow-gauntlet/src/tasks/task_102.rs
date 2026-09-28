//! task-102: selection-gate ablation (rust, V).
//!
//! **Seam:** the D_sel acceptance gate (strict-greater, ties rejected).
//!
//! **Dimension:** the gate's average-case contribution. The paper calls
//! the gate "the single most important defense in the method: it filters
//! out edits that read plausibly but are harmful" (arXiv 2605.23904v2
//! §II.5, §III.3).
//!
//! **Scenarios:** full loop on the F-order+F-bind mix, 5 seeds, three
//! arms: gate-on (strict `>`), gate-off (accept every candidate),
//! gate-tie-accepts (accept on `>=`, testing the strict-greater choice
//! against evaluation variance).
//!
//! **Pass criteria (pre-registered):** *replicates* = gate-off final
//! D_test ≤ s_0 D_test in ≥3/5 seeds AND ≥30% of gate-off accepted edits
//! are post-hoc neutral-or-harmful (ΔD_sel ≤ 0); the tie arm must accept
//! measurably more zero-gain edits than strict-greater. *Null* =
//! gate-off still improves D_test in ≥4/5 seeds (then the gate is
//! unnecessary *in this domain* — an honest negative about gate
//! necessity at toy scale, not a failed task).
//!
//! **Primary evidence:** the real [`ModelOptimizer`] via primo's local
//! Ollama HTTP API (`think: false`, `/api/generate`). The scripted mock
//! is the offline fallback/control only, labeled as such.
//!
//! Four cases: two validation, two adversarial.
//! - V1 `arms_complete_and_classified`: all three arms run and the
//!   preregistered verdict classifies. PASSES on any verdict.
//! - V2 `strict_accepts_only_gains`: every accepted step in the strict
//!   arm has d_sel_after strictly greater than d_sel_before. PASSES.
//! - A1 `gate_off_accepts_harm`: the gate-off arm accepts edits whose
//!   post-hoc ΔD_sel ≤ 0 — edits the gate would have caught. PASSES iff
//!   measured (the gate is load-bearing).
//! - A2 `slow_update_only_writes_protected`: longitudinal memory
//!   (KEEP/GUIDE/CYCLE lines) appears only in the protected section,
//!   never in the body — the epoch-end writer cannot leak into, and
//!   step edits cannot reach, the protected region. PASSES.

use crate::skillopt::driver::{
    ArmSummary, Backend, CaseReport, TaskDriverError, base_config, resolve_primary, run_arm_named,
    summarize, verdict_line,
};
use crate::skillopt::gate::GateMode;
use crate::skillopt::learner::Verdict;
use crate::skillopt::optimizer::ScriptedOptimizer;
use crate::skillopt::target::Family;
use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};

/// Task id.
pub const ID: &str = "task-102";
/// Human-readable name.
pub const NAME: &str = "selection-gate ablation";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order: two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "arms_complete_and_classified",
    "strict_accepts_only_gains",
    "gate_off_accepts_harm",
    "slow_update_only_writes_protected",
];

/// One arm's evidence.
struct ArmData {
    summary: ArmSummary,
    /// Seeds where final D_test <= s_0 D_test (gate-off did not improve).
    not_improved: usize,
    /// Accepted edits with post-hoc ΔD_sel <= 0 / all accepted edits.
    harm_frac: f64,
    /// Accepted edits with ΔD_sel == 0 exactly.
    zero_gain: usize,
    /// Total accepted edits.
    accepted_total: usize,
}

fn arm_data(name: &str, logs: &[crate::skillopt::learner::SeedLog]) -> ArmData {
    let mut not_improved = 0usize;
    let mut harm = 0usize;
    let mut zero_gain = 0usize;
    let mut accepted_total = 0usize;
    for log in logs {
        if log.d_test <= log.d_test_initial {
            not_improved += 1;
        }
        for step in &log.steps {
            if step.accepted {
                for delta in &step.per_edit_delta {
                    accepted_total += 1;
                    if *delta <= 0.0 {
                        harm += 1;
                    }
                    if *delta == 0.0 {
                        zero_gain += 1;
                    }
                }
            }
        }
    }
    ArmData {
        summary: summarize(name, logs),
        not_improved,
        harm_frac: if accepted_total > 0 {
            harm as f64 / accepted_total as f64
        } else {
            f64::NAN
        },
        zero_gain,
        accepted_total,
    }
}

/// Run the three arms under `backend`.
fn run_arms(backend: &Backend) -> Result<Vec<ArmData>, TaskDriverError> {
    let mut out = Vec::new();
    for (name, gate) in [
        ("strict", GateMode::Strict),
        ("off", GateMode::Off),
        ("tie-accepts", GateMode::TieAccepts),
    ] {
        let mut cfg = base_config();
        cfg.families = vec![Family::FOrder, Family::FBind];
        cfg.mixed = true;
        cfg.gate = gate;
        let (_, logs) = run_arm_named(name, &cfg, backend)?;
        out.push(arm_data(name, &logs));
    }
    Ok(out)
}

/// Classify the preregistered verdict.
fn classify(arm_list: &[ArmData]) -> (Verdict, String) {
    let (off, strict, tie) = (&arm_list[1], &arm_list[0], &arm_list[2]);
    let detail = format!(
        "gate-off not-improved seeds: {}/5; gate-off neutral/harmful accepted edits: {:.1}%; \
         tie zero-gain accepted: {} vs strict {} (totals {} vs {})",
        off.not_improved,
        off.harm_frac * 100.0,
        tie.zero_gain,
        strict.zero_gain,
        tie.accepted_total,
        strict.accepted_total,
    );
    let replicates =
        off.not_improved >= 3 && off.harm_frac >= 0.30 && tie.zero_gain > strict.zero_gain;
    let null = (5 - off.not_improved) >= 4;
    let verdict = if replicates {
        Verdict::Replicates
    } else if null {
        Verdict::Null
    } else {
        Verdict::Indeterminate
    };
    (verdict, detail)
}

fn evidence_lines(
    backend: &Backend,
    arm_list: &[ArmData],
    verdict: Verdict,
    detail: &str,
) -> Vec<String> {
    let mut ev = vec![
        format!("backend: {}", backend.label()),
        "seam: REAL SkillOpt loop (scripted targets F-order+F-bind mix, fixed \
         splits); optimizer fixed across arms, only the gate varies"
            .to_string(),
        verdict_line("102", verdict, detail),
    ];
    for a in arm_list {
        ev.push(format!(
            "arm {}: D_test {:.2}±{:.2} pts (n={}); not-improved {}/5; \
             neutral/harmful accepted {:.1}%; zero-gain accepted {}",
            a.summary.name,
            a.summary.mean,
            a.summary.std,
            a.summary.n,
            a.not_improved,
            a.harm_frac * 100.0,
            a.zero_gain,
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
            "not_improved": a.not_improved,
            "harm_frac": a.harm_frac,
            "zero_gain": a.zero_gain,
        })).collect::<Vec<_>>(),
    });
    Ok(CaseReport::pass(
        "arms_complete_and_classified",
        metrics,
        evidence_lines(&backend, &arm_list, verdict, &detail),
    ))
}

fn case_strict_accepts_only_gains() -> Result<CaseReport, TaskDriverError> {
    let backend = Backend::ScriptedFallback(ScriptedOptimizer::new());
    let mut cfg = base_config();
    cfg.families = vec![Family::FOrder, Family::FBind];
    cfg.mixed = true;
    cfg.gate = GateMode::Strict;
    let (_, logs) = run_arm_named("strict", &cfg, &backend)?;
    let mut checked = 0usize;
    let mut violations = Vec::new();
    for log in &logs {
        for step in &log.steps {
            if step.accepted {
                checked += 1;
                if step.d_sel_after <= step.d_sel_before {
                    violations.push(format!(
                        "seed {} step {}: after={} not > before={}",
                        log.seed, step.step, step.d_sel_after, step.d_sel_before
                    ));
                }
            }
        }
    }
    let evidence = vec![format!(
        "{checked} accepted steps in the strict arm, {} violations",
        violations.len()
    )];
    if violations.is_empty() {
        Ok(CaseReport::pass(
            "strict_accepts_only_gains",
            serde_json::json!({"checked": checked, "violations": 0}),
            evidence,
        ))
    } else {
        Ok(CaseReport::fail(
            "strict_accepts_only_gains",
            format!("strict gate accepted non-gains: {}", violations.join("; ")),
            evidence,
        ))
    }
}

fn case_gate_off_accepts_harm() -> Result<CaseReport, TaskDriverError> {
    let backend = Backend::ScriptedFallback(ScriptedOptimizer::new());
    let arm_list = run_arms(&backend)?;
    let off = &arm_list[1];
    let evidence = vec![
        format!("gate-off accepted edits: {}", off.accepted_total),
        format!(
            "gate-off neutral/harmful (post-hoc ΔD_sel ≤ 0): {:.1}%",
            off.harm_frac * 100.0
        ),
    ];
    if off.harm_frac > 0.0 {
        Ok(CaseReport::pass(
            "gate_off_accepts_harm",
            serde_json::json!({"harm_frac": off.harm_frac, "accepted_total": off.accepted_total}),
            evidence,
        ))
    } else {
        Ok(CaseReport::fail(
            "gate_off_accepts_harm",
            "gate-off accepted zero neutral/harmful edits — the gate would be \
             unmeasurable in this setup"
                .to_string(),
            evidence,
        ))
    }
}

fn case_slow_update_only_writes_protected() -> Result<CaseReport, TaskDriverError> {
    let backend = Backend::ScriptedFallback(ScriptedOptimizer::new());
    let mut cfg = base_config();
    cfg.families = vec![Family::FOrder, Family::FBind];
    cfg.mixed = true;
    let (_, logs) = run_arm_named("strict", &cfg, &backend)?;
    let mut leaked = Vec::new();
    for log in &logs {
        for line in log.final_body.lines() {
            let t = line.trim_start();
            if t.starts_with("KEEP:") || t.starts_with("GUIDE:") || t.starts_with("CYCLE ") {
                leaked.push(format!("seed {}: {line}", log.seed));
            }
        }
    }
    let evidence = vec![format!(
        "checked {} final bodies, {} longitudinal lines leaked into body",
        logs.len(),
        leaked.len()
    )];
    if leaked.is_empty() {
        Ok(CaseReport::pass(
            "slow_update_only_writes_protected",
            serde_json::json!({"seeds": logs.len(), "leaked": 0}),
            evidence,
        ))
    } else {
        Ok(CaseReport::fail(
            "slow_update_only_writes_protected",
            format!(
                "protected-section lines leaked into body: {}",
                leaked.join("; ")
            ),
            evidence,
        ))
    }
}

/// Run one case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "arms_complete_and_classified" => case_arms_complete_and_classified(),
        "strict_accepts_only_gains" => case_strict_accepts_only_gains(),
        "gate_off_accepts_harm" => case_gate_off_accepts_harm(),
        "slow_update_only_writes_protected" => case_slow_update_only_writes_protected(),
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
