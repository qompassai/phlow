//! task-107: cross-family transfer (rust, V).
//!
//! **Seam:** the frozen skill text — evolve on one family, freeze the
//! exact target-visible text (`final_body` + slow-update markers +
//! `final_protected`), and apply it unchanged to the other family's
//! fixtures.
//!
//! **Dimension:** the paper's transfer claim. This task measures
//! **content portability, not skill discovery**: the families are
//! orthogonal by construction (order tools vs exact-bind lines), so a
//! transferred skill cannot teach the target family anything new. The
//! honest question is how much of the in-domain gain survives the
//! family switch, and whether anything drops below the no-skill
//! baseline.
//!
//! **Pass criteria (pre-registered):** per direction — *negative* =
//! transferred D_test below the empty-skill baseline; *replicates* =
//! paper-like small positive transfer (≥ 1.0 point above baseline);
//! *null* = within ±1.0 point of baseline. The task passes on any
//! verdict.
//!
//! **Primary evidence:** the clearly labeled scripted double
//! ([`ScriptedOptimizer`]). No real model is used for this wave's
//! verdicts; nothing here is presented as real-model evidence.
//!
//! Four cases: two validation, two adversarial.
//! - V1 `transfer_classified`: F-order→F-bind and F-bind→F-order
//!   both run and classify. PASSES on any verdict.
//! - V2 `in_domain_gains_positive`: the source arm's own D_test gain
//!   is positive — the transfer machinery starts from real learning.
//! - A1 `never_below_baseline`: transferred D_test ≥ empty baseline
//!   in both directions. Below baseline = the negative finding.
//! - A2 `provenance_preserved`: the frozen-then-rebuilt document
//!   still carries `Provenance::Experiment` and an untrusted document
//!   is refused.

use crate::skillopt::doc::{SLOW_UPDATE_END, SLOW_UPDATE_START, SkillDoc};
use crate::skillopt::driver::{
    Backend, CaseReport, TaskDriverError, base_config, default_spec, run_arm_named,
    untrusted_is_refused, verdict_line,
};
use crate::skillopt::learner::{LearnerConfig, SeedLog, Verdict, mean_std};
use crate::skillopt::optimizer::ScriptedOptimizer;
use crate::skillopt::target::{BIND_EXACT, Family, MixedTarget, make_splits};
use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};

/// Task id.
pub const ID: &str = "task-107";
/// Human-readable name.
pub const NAME: &str = "cross-family transfer";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order: two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "transfer_classified",
    "in_domain_gains_positive",
    "never_below_baseline",
    "provenance_preserved",
];

/// Epochs for the source arm.
const EPOCHS: usize = 3;
/// Fixed probe fixtures for the target family.
const PROBE_SEED: u64 = 0x107_7AA5_FE20_0001;

/// The labeled scripted double for this wave's verdicts. Never a real
/// model; never presented as one.
fn backend() -> Backend {
    Backend::ScriptedFallback(ScriptedOptimizer::new())
}

fn source_cfg(family: Family) -> LearnerConfig {
    let mut cfg = base_config();
    cfg.families = vec![family];
    cfg.mixed = false;
    cfg.epochs = EPOCHS;
    cfg
}

/// Freeze the exact target-visible text of a learned skill: body +
/// slow-update markers + protected section. The protected lines are
/// target-inert bookkeeping (they never match either family's scoring
/// predicates), but the frozen bytes are exactly what the target saw.
fn freeze_text(log: &SeedLog) -> String {
    format!(
        "{}\n{SLOW_UPDATE_START}\n{}\n{SLOW_UPDATE_END}\n",
        log.final_body, log.final_protected
    )
}

/// One transfer direction: evolve on `source`, freeze, score the frozen
/// text on `target` fixtures. Returns (per-seed transfer scores, empty
/// baseline, in-domain mean gain in points, per-seed contamination
/// counts, per-seed decontaminated transfer scores).
///
/// The return tuple is (in-domain gains, transfer mean, transfer std,
/// per-seed contamination counts, per-seed decontaminated scores).
///
/// Contamination: frozen-body lines matching the TARGET family's
/// patterns (ORDER[p] lines inside an F-bind skill, the exact bind
/// line inside an F-order skill). These are D_sel-neutral ride-alongs
/// from the source arm's bundles — not target-family knowledge — so
/// the decontaminated score (those lines stripped) isolates genuine
/// content portability.
/// Output of [`transfer_direction`]: (in-domain gains, transfer
/// mean, transfer std, per-seed contamination counts, per-seed
/// decontaminated transfer scores).
type TransferOutcome = (Vec<f64>, f64, f64, Vec<usize>, Vec<f64>);

fn transfer_direction(source: Family, target: Family) -> Result<TransferOutcome, TaskDriverError> {
    let cfg = source_cfg(source);
    let backend = backend();
    let name = format!("src-{source:?}");
    let (_, logs) = run_arm_named(&name, &cfg, &backend)?;
    let spec = default_spec();
    let probe = make_splits(target, PROBE_SEED, 1.0, &spec);
    let baseline = MixedTarget.score(&SkillDoc::experiment(""), &probe.d_test);
    let mut scores = Vec::with_capacity(logs.len());
    let mut contam = Vec::with_capacity(logs.len());
    let mut clean_scores = Vec::with_capacity(logs.len());
    for log in &logs {
        let doc = SkillDoc::experiment(&freeze_text(log));
        scores.push(MixedTarget.score(&doc, &probe.d_test));
        let lines: Vec<&str> = log.final_body.lines().collect();
        contam.push(lines.iter().filter(|l| is_target_line(l, target)).count());
        let stripped = lines
            .iter()
            .filter(|l| !is_target_line(l, target))
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let clean_doc = SkillDoc::experiment(&format!(
            "{stripped}\n{SLOW_UPDATE_START}\n{}\n{SLOW_UPDATE_END}\n",
            log.final_protected
        ));
        clean_scores.push(MixedTarget.score(&clean_doc, &probe.d_test));
    }
    let gains: Vec<f64> = logs
        .iter()
        .map(|l| (l.d_test - l.d_test_initial) * 100.0)
        .collect();
    Ok((scores, baseline, mean_std(&gains).0, contam, clean_scores))
}

/// True iff `line` is a target-family rule line (the contamination
/// pattern for cross-family transfer).
fn is_target_line(line: &str, target: Family) -> bool {
    match target {
        Family::FOrder => line.starts_with("ORDER["),
        Family::FBind => line == BIND_EXACT,
        Family::FLedger => line.starts_with("LEDGER:"),
    }
}

/// Classify on the DECONTAMINATED transfer scores (target-family
/// ride-along lines stripped). The raw score is confounded by bundle
/// contamination; the paper-like question is whether the *content*
/// ports, not whether the bundle smuggled target-family lines.
fn classify_direction(decontaminated: &[f64], baseline: f64) -> (Verdict, String) {
    let (mean, std) = mean_std(&decontaminated.iter().map(|s| s * 100.0).collect::<Vec<_>>());
    let base_pts = baseline * 100.0;
    let verdict = if mean < base_pts {
        Verdict::Negative
    } else if mean >= base_pts + 1.0 {
        Verdict::Replicates
    } else {
        Verdict::Null
    };
    (
        verdict,
        format!("decontaminated transfer {mean:.2} ± {std:.2} pts vs baseline {base_pts:.2}"),
    )
}

fn case_transfer_classified() -> Result<CaseReport, TaskDriverError> {
    let mut evidence = vec![
        "backend: scripted-double (ScriptedOptimizer; NOT a real model)".to_string(),
        "scope: content portability, not skill discovery".to_string(),
    ];
    let mut metrics = serde_json::Map::new();
    for (source, target) in [
        (Family::FOrder, Family::FBind),
        (Family::FBind, Family::FOrder),
    ] {
        let (scores, baseline, in_gain, contam, clean) = transfer_direction(source, target)?;
        let (verdict, detail) = classify_direction(&clean, baseline);
        let dir = format!("{source:?}->{target:?}");
        let clean_mean = mean_std(&clean).0 * 100.0;
        let (raw_mean, raw_std) = mean_std(&scores.iter().map(|x| x * 100.0).collect::<Vec<_>>());
        evidence.push(format!(
            "{dir}: in-domain gain {in_gain:.2} pts; raw transfer {raw_mean:.2} ± {raw_std:.2} pts (confounded); {detail}"
        ));
        evidence.push(format!(
            "{dir}: cross-family ride-along lines per seed {contam:?}; decontaminated transfer {clean_mean:.2} pts"
        ));
        evidence.push(verdict_line("107", verdict, &format!("{dir}: {detail}")));
        metrics.insert(
            dir,
            serde_json::json!({
                "transfer_mean": mean_std(&scores).0,
                "baseline": baseline,
                "in_domain_gain": in_gain,
                "contamination_per_seed": contam,
                "decontaminated_transfer_mean": clean_mean / 100.0,
                "verdict": verdict.to_string(),
            }),
        );
    }
    Ok(CaseReport::pass(
        CASES[0],
        serde_json::Value::Object(metrics),
        evidence,
    ))
}

fn case_in_domain_gains_positive() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = vec!["backend: scripted-double".to_string()];
    for family in [Family::FOrder, Family::FBind] {
        let cfg = source_cfg(family);
        let backend = backend();
        let (summary, logs) = run_arm_named(&format!("src-{family:?}"), &cfg, &backend)?;
        let gain = mean_std(
            &logs
                .iter()
                .map(|l| (l.d_test - l.d_test_initial) * 100.0)
                .collect::<Vec<_>>(),
        )
        .0;
        evidence.push(format!(
            "{family:?}: mean D_test {:.2} ± {:.2} pts, gain {gain:.2} pts",
            summary.mean, summary.std
        ));
        if gain <= 0.0 {
            failures.push(format!(
                "{family:?}: in-domain gain not positive ({gain:.2})"
            ));
        }
    }
    let mut report = CaseReport::pass(CASES[1], serde_json::json!({}), evidence);
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

fn case_never_below_baseline() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = vec!["backend: scripted-double".to_string()];
    for (source, target) in [
        (Family::FOrder, Family::FBind),
        (Family::FBind, Family::FOrder),
    ] {
        let (scores, baseline, _, _, _) = transfer_direction(source, target)?;
        let dir = format!("{source:?}->{target:?}");
        for (i, s) in scores.iter().enumerate() {
            if *s < baseline {
                failures.push(format!(
                    "{dir} seed {i}: transfer {s:.4} below baseline {baseline:.4}"
                ));
            }
        }
        evidence.push(format!(
            "{dir}: min transfer {:.4} vs baseline {:.4}",
            scores.iter().cloned().fold(f64::INFINITY, f64::min),
            baseline
        ));
    }
    let mut report = CaseReport::pass(CASES[2], serde_json::json!({}), evidence);
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

fn case_provenance_preserved() -> Result<CaseReport, TaskDriverError> {
    let cfg = source_cfg(Family::FOrder);
    let backend = backend();
    let (_, logs) = run_arm_named("src-FOrder", &cfg, &backend)?;
    let mut failures = Vec::new();
    for log in &logs {
        let doc = SkillDoc::experiment(&freeze_text(log));
        if doc.provenance() != crate::skillopt::doc::Provenance::Experiment {
            failures.push(format!(
                "seed {}: rebuilt doc lost Experiment provenance",
                log.seed
            ));
        }
    }
    if let Err(detail) = untrusted_is_refused(&cfg) {
        failures.push(detail);
    }
    let evidence = vec![
        "backend: scripted-double".to_string(),
        format!(
            "frozen-then-rebuilt docs carry Provenance::Experiment across {} seeds",
            logs.len()
        ),
    ];
    let mut report = CaseReport::pass(CASES[3], serde_json::json!({}), evidence);
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// Run one named case.
pub fn run_case(case: &'static str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "transfer_classified" => case_transfer_classified(),
        "in_domain_gains_positive" => case_in_domain_gains_positive(),
        "never_below_baseline" => case_never_below_baseline(),
        "provenance_preserved" => case_provenance_preserved(),
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
    let mut evidence = vec![
        "backend: scripted-double (ScriptedOptimizer; NOT a real model — \
         wave-106-110 verdicts use scripted doubles only)"
            .to_string(),
        "scope: content portability, not skill discovery — families are \
         orthogonal by construction"
            .to_string(),
    ];
    for (source, target) in [
        (Family::FOrder, Family::FBind),
        (Family::FBind, Family::FOrder),
    ] {
        let (scores, baseline, in_gain, contam, clean) = transfer_direction(source, target)
            .map_err(|e| TaskFailure {
                where_: "transfer".to_string(),
                how: e.to_string(),
                evidence: vec!["backend: scripted-double".to_string()],
            })?;
        let (verdict, detail) = classify_direction(&scores, baseline);
        let dir = format!("{source:?}->{target:?}");
        evidence.push(format!("{dir}: in-domain gain {in_gain:.2} pts"));
        evidence.push(format!(
            "{dir}: ride-along lines per seed {contam:?}; decontaminated {decon:.2} pts",
            decon = mean_std(&clean).0 * 100.0
        ));
        evidence.push(verdict_line("107", verdict, &format!("{dir}: {detail}")));
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
