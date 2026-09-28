//! task-106: loop convergence dynamics (rust, V).
//!
//! **Seam:** the composed loop end-to-end (all components on): mixed
//! F-order + F-bind, 3 epochs, 5 seeds.
//!
//! **Dimension:** the paper's headline dynamics — "The cumulative number
//! of accepted edits is small (median 2.5), and the gains ... each come
//! from a **single** accepted edit. The validation gate filters out the
//! vast majority of proposals" (arXiv 2605.23904v2 §III.3); artifact
//! "typically 300 to 2000 tokens" (§I). Does the composed system
//! actually behave this way, or do the components interact
//! pathologically?
//!
//! **Pass criteria (pre-registered, from the design doc):**
//! *replicates* = median accepted edits ≤4 per run; acceptance rate
//! <25% of proposals; final artifact within 300–2000 tokens;
//! D_test gain >0 in every seed; no seed regresses below s_0 on D_test.
//! *Negative* = any seed regresses below s_0, or acceptance rate >50%
//! (the gate is not filtering "the vast majority").
//!
//! **Primary evidence:** the clearly labeled scripted double
//! ([`ScriptedOptimizer`]). No real model is used for this wave's
//! verdicts; nothing here is presented as real-model evidence.
//!
//! Four cases: two validation, two adversarial.
//! - V1 `composed_dynamics_classified`: the mixed arm runs to
//!   completion; dynamics measured against the paper's numbers and
//!   classified. PASSES on any verdict — null/negative are first-class
//!   measurements.
//! - V2 `edit_ledger_legible`: every step record carries a veto label
//!   from the known set and the per-seed D_sel training curves render.
//! - A1 `never_regresses`: no step regresses D_sel and no seed ends
//!   below s_0 on D_test — the loop must never make things worse.
//!   FAILS loudly if violated.
//! - A2 `untrusted_doc_cannot_enter_loop`: an untrusted document is
//!   refused before any rollout. PASSES.

use crate::skillopt::doc::{SLOW_UPDATE_END, SLOW_UPDATE_START, SkillDoc};
use crate::skillopt::driver::{
    Backend, CaseReport, TaskDriverError, base_config, run_arm_named, summarize,
    untrusted_is_refused, verdict_line,
};
use crate::skillopt::learner::{LearnerConfig, SeedLog, Verdict, mean_std};
use crate::skillopt::optimizer::ScriptedOptimizer;
use crate::skillopt::target::{Family, MixedTarget, make_mixed_splits};
use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};

/// Task id.
pub const ID: &str = "task-106";
/// Human-readable name.
pub const NAME: &str = "loop convergence dynamics";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order: two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "composed_dynamics_classified",
    "edit_ledger_legible",
    "never_regresses",
    "untrusted_doc_cannot_enter_loop",
];

/// Epochs for the composed arm.
const EPOCHS: usize = 3;
/// Paper's artifact token window (§I: "typically 300 to 2000 tokens").
const TOKEN_LO: usize = 300;
const TOKEN_HI: usize = 2000;

/// The labeled scripted double for this wave's verdicts. Never a real
/// model; never presented as one.
fn backend() -> Backend {
    Backend::ScriptedFallback(ScriptedOptimizer::new())
}

/// The composed arm: mixed F-order + F-bind, 3 epochs, 5 seeds.
fn arm_cfg() -> LearnerConfig {
    let mut cfg = base_config();
    cfg.families = vec![Family::FOrder, Family::FBind];
    cfg.mixed = true;
    cfg.epochs = EPOCHS;
    cfg
}

fn run_composed() -> Result<(LearnerConfig, Vec<SeedLog>), TaskDriverError> {
    let cfg = arm_cfg();
    let backend = backend();
    let (_, logs) = run_arm_named("composed", &cfg, &backend)?;
    Ok((cfg, logs))
}

/// Exact frozen artifact text for one seed (the optimizer-visible
/// text: body + slow-update markers + protected section).
fn artifact_text(log: &SeedLog) -> String {
    format!(
        "{}\n{SLOW_UPDATE_START}\n{}\n{SLOW_UPDATE_END}\n",
        log.final_body, log.final_protected
    )
}

/// Whitespace-token proxy for the paper's token window. Labeled as a
/// proxy: this is NOT a BPE count. Exact byte length is reported
/// alongside.
fn ws_tokens(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Median of a non-empty slice.
fn median(mut xs: Vec<usize>) -> f64 {
    xs.sort_unstable();
    let n = xs.len();
    if n % 2 == 1 {
        xs[n / 2] as f64
    } else {
        (xs[n / 2 - 1] + xs[n / 2]) as f64 / 2.0
    }
}

/// The composed-loop dynamics: the paper's headline numbers measured
/// on the scripted double.
struct Dynamics {
    median_accepted: f64,
    accept_rate: f64,
    artifact_bytes: usize,
    artifact_tokens: usize,
    gains: Vec<f64>,
    /// Fraction of each seed's D_test gain from its top-1 accepted
    /// edit (leave-one-out on the final skill).
    top1_fractions: Vec<f64>,
    /// Same fraction on D_sel via the gate's own per-edit deltas.
    top1_fractions_sel: Vec<f64>,
}

/// Leave-one-out single-edit gain: for each distinct accepted edit
/// line, remove it from the final skill and measure the D_test drop.
/// Returns the top-1 drop as a fraction of the seed's total D_test
/// gain (0 when the seed gained nothing).
fn top1_fraction_dtest(cfg: &LearnerConfig, log: &SeedLog) -> f64 {
    let total_gain = (log.d_test - log.d_test_initial) * 100.0;
    if total_gain <= 1e-9 {
        return 0.0;
    }
    let splits = make_mixed_splits(log.seed as u64, cfg.d_tr_frac, &cfg.spec);
    let mut seen = std::collections::HashSet::new();
    let mut best = 0.0f64;
    for edit in &log.accepted_edits {
        if !seen.insert(edit.clone()) {
            continue;
        }
        let body = log
            .final_body
            .lines()
            .filter(|l| *l != edit.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let doc = SkillDoc::experiment(&format!(
            "{body}\n{SLOW_UPDATE_START}\n{}\n{SLOW_UPDATE_END}\n",
            log.final_protected
        ));
        let drop = (log.d_test - MixedTarget.score(&doc, &splits.d_test)) * 100.0;
        if drop > best {
            best = drop;
        }
    }
    (best / total_gain).clamp(0.0, 1.0)
}

/// Top-1 fraction on D_sel from the gate's own per-edit deltas
/// (accepted steps only).
fn top1_fraction_dsel(log: &SeedLog) -> f64 {
    let mut deltas: Vec<f64> = log
        .steps
        .iter()
        .filter(|s| s.accepted)
        .flat_map(|s| s.per_edit_delta.iter().copied())
        .collect();
    if deltas.is_empty() {
        return 0.0;
    }
    deltas.sort_by(|a, b| b.partial_cmp(a).unwrap());
    let total: f64 = deltas.iter().sum();
    if total <= 1e-12 {
        return 0.0;
    }
    (deltas[0] / total).clamp(0.0, 1.0)
}

fn dynamics(cfg: &LearnerConfig, logs: &[SeedLog]) -> Dynamics {
    let accepted_per_seed: Vec<usize> = logs
        .iter()
        .map(|l| l.steps.iter().map(|s| s.n_applied).sum())
        .collect();
    let proposed: usize = logs
        .iter()
        .flat_map(|l| l.steps.iter())
        .map(|s| s.n_proposed)
        .sum();
    let applied: usize = accepted_per_seed.iter().sum();
    let gains: Vec<f64> = logs
        .iter()
        .map(|l| (l.d_test - l.d_test_initial) * 100.0)
        .collect();
    // Artifact size: exact bytes + whitespace-token proxy, max over
    // seeds (the paper's window is about the artifact, not the mean).
    let (artifact_bytes, artifact_tokens) = logs
        .iter()
        .map(|l| {
            let t = artifact_text(l);
            (t.len(), ws_tokens(&t))
        })
        .max()
        .unwrap_or((0, 0));
    let top1_fractions = logs.iter().map(|l| top1_fraction_dtest(cfg, l)).collect();
    let top1_fractions_sel = logs.iter().map(top1_fraction_dsel).collect();
    Dynamics {
        median_accepted: median(accepted_per_seed),
        accept_rate: if proposed > 0 {
            applied as f64 / proposed as f64
        } else {
            0.0
        },
        artifact_bytes,
        artifact_tokens,
        gains,
        top1_fractions,
        top1_fractions_sel,
    }
}

fn classify(d: &Dynamics) -> (Verdict, String) {
    let all_gain = d.gains.iter().all(|g| *g > 0.0);
    let no_regress = d.gains.iter().all(|g| *g >= -1e-9);
    let tokens_ok = (TOKEN_LO..=TOKEN_HI).contains(&d.artifact_tokens);
    let verdict = if !no_regress || d.accept_rate > 0.50 {
        Verdict::Negative
    } else if d.median_accepted <= 4.0
        && d.accept_rate < 0.25
        && tokens_ok
        && all_gain
        && no_regress
    {
        Verdict::Replicates
    } else {
        Verdict::Null
    };
    let top1 = mean_std(&d.top1_fractions).0 * 100.0;
    let detail = format!(
        "median accepted edits {:.1} (paper 2.5); acceptance rate {:.1}% (paper: vast majority filtered); artifact {} bytes / ~{} ws-tokens (paper 300-2000); D_test gains [{}] pts, all>0={all_gain}, none below s_0={no_regress}; top-1 edit share of D_test gain {top1:.0}% (paper: most of it)",
        d.median_accepted,
        d.accept_rate * 100.0,
        d.artifact_bytes,
        d.artifact_tokens,
        d.gains
            .iter()
            .map(|g| format!("{g:.1}"))
            .collect::<Vec<_>>()
            .join(","),
    );
    (verdict, detail)
}

fn case_composed_dynamics_classified() -> Result<CaseReport, TaskDriverError> {
    let (cfg, logs) = run_composed()?;
    let summary = summarize("composed", &logs);
    let d = dynamics(&cfg, &logs);
    let (verdict, detail) = classify(&d);
    let evidence = vec![
        "backend: scripted-double (ScriptedOptimizer; NOT a real model)".to_string(),
        format!(
            "arm composed: {} seeds, {} epochs, mean D_test {:.2} ± {:.2} pts",
            summary.n, EPOCHS, summary.mean, summary.std
        ),
        format!(
            "paper dynamics: median accepted {:.1}; acceptance {:.1}%; artifact ~{} tokens; top-1 D_test share {:.0}%; top-1 D_sel share {:.0}%",
            d.median_accepted,
            d.accept_rate * 100.0,
            d.artifact_tokens,
            mean_std(&d.top1_fractions).0 * 100.0,
            mean_std(&d.top1_fractions_sel).0 * 100.0,
        ),
        verdict_line("106", verdict, &detail),
    ];
    let metrics = serde_json::json!({
        "mean_d_test": summary.mean,
        "std_d_test": summary.std,
        "median_accepted_edits": d.median_accepted,
        "acceptance_rate": d.accept_rate,
        "artifact_bytes": d.artifact_bytes,
        "artifact_ws_tokens": d.artifact_tokens,
        "d_test_gains": d.gains,
        "top1_fraction_dtest": d.top1_fractions,
        "top1_fraction_dsel": d.top1_fractions_sel,
        "verdict": verdict.to_string(),
    });
    Ok(CaseReport::pass(CASES[0], metrics, evidence))
}

fn case_edit_ledger_legible() -> Result<CaseReport, TaskDriverError> {
    let (_, logs) = run_composed()?;
    let known = [
        "accepted:gate-strict",
        "accepted:gate-tie-accepts",
        "accepted:gate-off",
        "rejected:gate-strict",
        "rejected:empty",
        "rejected:apply-failed",
    ];
    let mut failures = Vec::new();
    let mut evidence = vec!["backend: scripted-double".to_string()];
    for log in &logs {
        for step in &log.steps {
            if !known.contains(&step.veto.as_str()) {
                failures.push(format!(
                    "seed {} step {}: unknown veto label '{}'",
                    log.seed, step.step, step.veto
                ));
            }
        }
        let curve: Vec<String> = log
            .steps
            .iter()
            .map(|s| format!("{:.2}", s.d_sel_after * 100.0))
            .collect();
        evidence.push(format!(
            "seed {}: d_sel curve [{}]; final d_test {:.2}",
            log.seed,
            curve.join(" "),
            log.d_test * 100.0
        ));
    }
    let metrics = serde_json::json!({
        "seeds": logs.len(),
        "steps_per_seed": logs.first().map(|l| l.steps.len()).unwrap_or(0),
    });
    let mut report = CaseReport::pass(CASES[1], metrics, evidence);
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

fn case_never_regresses() -> Result<CaseReport, TaskDriverError> {
    let (_, logs) = run_composed()?;
    let mut failures = Vec::new();
    for log in &logs {
        for step in &log.steps {
            if step.d_sel_after + 1e-12 < step.d_sel_before {
                failures.push(format!(
                    "seed {} step {}: D_sel regressed {:.4} -> {:.4} (veto {})",
                    log.seed, step.step, step.d_sel_before, step.d_sel_after, step.veto
                ));
            }
        }
        // The paper's headline invariant: no setting drops below the
        // no-skill baseline — here, no seed ends below its s_0.
        if log.d_test + 1e-12 < log.d_test_initial {
            failures.push(format!(
                "seed {}: D_test {:.4} below s_0 {:.4}",
                log.seed, log.d_test, log.d_test_initial
            ));
        }
    }
    let n_steps: usize = logs.iter().map(|l| l.steps.len()).sum();
    let evidence = vec![
        "backend: scripted-double".to_string(),
        format!(
            "checked {n_steps} steps across {} seeds (D_sel per step, D_test vs s_0 per seed); regressions: {}",
            logs.len(),
            failures.len()
        ),
    ];
    let metrics = serde_json::json!({ "steps": n_steps, "regressions": failures.len() });
    let mut report = CaseReport::pass(CASES[2], metrics, evidence);
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

fn case_untrusted_doc_cannot_enter_loop() -> Result<CaseReport, TaskDriverError> {
    let cfg = arm_cfg();
    match untrusted_is_refused(&cfg) {
        Ok(()) => Ok(CaseReport::pass(
            CASES[3],
            serde_json::json!({ "refused": true }),
            vec![
                "backend: scripted-double".to_string(),
                "untrusted document refused before any rollout".to_string(),
            ],
        )),
        Err(detail) => {
            let mut report = CaseReport::pass(
                CASES[3],
                serde_json::json!({ "refused": false }),
                vec!["backend: scripted-double".to_string()],
            );
            report.passed = false;
            report.failures = vec![detail];
            Ok(report)
        }
    }
}

/// Run one named case.
pub fn run_case(case: &'static str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "composed_dynamics_classified" => case_composed_dynamics_classified(),
        "edit_ledger_legible" => case_edit_ledger_legible(),
        "never_regresses" => case_never_regresses(),
        "untrusted_doc_cannot_enter_loop" => case_untrusted_doc_cannot_enter_loop(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("unknown case '{case}'"),
        }),
    }
}

/// Full task run: the four cases in order, first failure stops the
/// task with its evidence attached.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    let mut evidence = Vec::new();
    for case in CASES {
        match run_case(case) {
            Ok(report) => {
                evidence.extend(report.evidence.clone());
                if !report.passed {
                    return TaskOutcome::Fail {
                        where_: format!("task-106/{case}"),
                        how: report.failures.join("; "),
                        evidence: bound_evidence(evidence),
                    };
                }
            }
            Err(e) => {
                return TaskOutcome::Fail {
                    where_: format!("task-106/{case}"),
                    how: e.to_string(),
                    evidence: bound_evidence(evidence),
                };
            }
        }
    }
    TaskOutcome::Pass {
        evidence: bound_evidence(evidence),
    }
}
