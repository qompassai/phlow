//! Task 111 — selection-split overfitting (rust, adversarial).
//!
//! The paper tunes on D_sel and reports on D_test; the question is how
//! much of the D_sel gain is real and how much is selection luck. This
//! task runs the F-order standard loop against a small D_sel (16) and
//! a large D_sel (64) with D_test SEALED (no training-time reads — any
//! such read is a protocol breach, counted in
//! [`SeedLog::d_test_reads`]), plus a three-split arm (D_selA accepts,
//! D_selB confirms; accept only when strictly better on both). All
//! verdict evidence comes from the clearly labeled scripted double
//! (MOCK) — never presented as real-model evidence.
//!
//! Preregistered classification: overfitting detected if the small-split
//! mean (D_sel gain − D_test gain) exceeds 3.0 points; contained if the
//! gap is ≤ 1.5 points or the three-split arm shrinks it by ≥ 50%.

use crate::skillopt::doc::SkillDoc;
use crate::skillopt::driver::{
    CaseReport, TaskDriverError, base_config, default_spec, verdict_line,
};
use crate::skillopt::learner::{Learner, LearnerConfig, SeedLog, Verdict, mean_std};
use crate::skillopt::optimizer::ScriptedOptimizer;
use crate::skillopt::target::{Family, MixedTarget, make_splits};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-111";
/// Task name.
pub const NAME: &str = "selection-split overfitting";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "small_vs_large_gap",
    "three_split_containment",
    "seal_holds",
    "acceptance_d_test_correlation",
];
/// Overfitting bar: small-split gap above this (points) = detected.
pub const OVERFIT_GAP_PTS: f64 = 3.0;
/// Containment bar: gap at or below this (points) = contained.
pub const CONTAINED_GAP_PTS: f64 = 1.5;
/// Containment bar: three-split shrinkage of the gap at or above this
/// fraction = contained.
pub const SHRINKAGE_FRAC: f64 = 0.50;

/// Arm configuration: F-order standard loop, D_test sealed.
fn arm_cfg(n_sel: usize, confirm_split: bool) -> LearnerConfig {
    let mut cfg = base_config();
    cfg.families = vec![Family::FOrder];
    cfg.sealed_d_test = true;
    cfg.confirm_split = confirm_split;
    let mut spec = default_spec();
    spec.n_sel = n_sel;
    cfg.spec = spec;
    cfg
}

/// Per-arm summary in points.
struct ArmStats {
    n: usize,
    sel_gain: f64,
    test_gain: f64,
    gap: f64,
    gap_std: f64,
    d_test_reads: u64,
    n_accepted: f64,
}

fn summarize_arm(logs: &[SeedLog]) -> ArmStats {
    let sel_gain: Vec<f64> = logs
        .iter()
        .map(|l| (l.d_sel_final - l.d_sel_initial) * 100.0)
        .collect();
    let test_gain: Vec<f64> = logs
        .iter()
        .map(|l| (l.d_test - l.d_test_initial) * 100.0)
        .collect();
    let gap: Vec<f64> = sel_gain
        .iter()
        .zip(test_gain.iter())
        .map(|(s, t)| s - t)
        .collect();
    let (gap_mean, gap_std) = mean_std(&gap);
    let (sel_mean, _) = mean_std(&sel_gain);
    let (test_mean, _) = mean_std(&test_gain);
    let n_accepted = logs
        .iter()
        .map(|l| l.accepted_edits.len() as f64)
        .sum::<f64>()
        / logs.len() as f64;
    ArmStats {
        n: logs.len(),
        sel_gain: sel_mean,
        test_gain: test_mean,
        gap: gap_mean,
        gap_std,
        d_test_reads: logs.iter().map(|l| l.d_test_reads).sum(),
        n_accepted,
    }
}

/// Run the three arms. Returns (small, large, three-split) logs.
/// Uses the scripted double directly (MOCK): the design fixes the
/// verdict backend to ScriptedOptimizer for this wave.
type ArmTriplet = (Vec<SeedLog>, Vec<SeedLog>, Vec<SeedLog>);
fn run_arms() -> Result<ArmTriplet, TaskDriverError> {
    let opt = ScriptedOptimizer::new();
    let run = |name: &str, cfg: &LearnerConfig| {
        let logs = Learner
            .run_arm(cfg, &opt)
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
        Ok(logs)
    };
    let small = run("small-dsel16", &arm_cfg(16, false))?;
    let large = run("large-dsel64", &arm_cfg(64, false))?;
    let three = run("three-split", &arm_cfg(32, true))?;
    Ok((small, large, three))
}

/// Preregistered verdict classification from the arm statistics.
fn classify(small: &ArmStats, three: &ArmStats) -> (Verdict, String) {
    let shrinkage = if small.gap.abs() < 1e-9 {
        0.0
    } else {
        (small.gap - three.gap) / small.gap
    };
    if small.gap > OVERFIT_GAP_PTS {
        if shrinkage >= SHRINKAGE_FRAC {
            (
                Verdict::Replicates,
                format!(
                    "overfitting detected (small gap {:.2} > {OVERFIT_GAP_PTS:.1}) \
                     but three-split confirmation shrinks it by {:.0}% \
                     (>= 50%): contained",
                    small.gap,
                    shrinkage * 100.0,
                ),
            )
        } else {
            (
                Verdict::Negative,
                format!(
                    "overfitting detected (small gap {:.2} > {OVERFIT_GAP_PTS:.1}) \
                     and three-split shrinks it by only {:.0}%: NOT contained",
                    small.gap,
                    shrinkage * 100.0,
                ),
            )
        }
    } else if small.gap <= CONTAINED_GAP_PTS {
        (
            Verdict::Replicates,
            format!(
                "no overfitting: small-split gap {:.2} <= {CONTAINED_GAP_PTS:.1}: contained",
                small.gap,
            ),
        )
    } else {
        (
            Verdict::Indeterminate,
            format!(
                "small-split gap {:.2} between the bars ({CONTAINED_GAP_PTS:.1}, {OVERFIT_GAP_PTS:.1}]",
                small.gap,
            ),
        )
    }
}
/// Post-hoc marginal D_test contribution of one accepted line.
///
/// Removes the first occurrence of `line` from the final body and
/// scores the difference on D_test. Runs AFTER training (the arm logs
/// are in hand), so it never breaks the D_test seal: it is evaluation,
/// not training. Returns `None` when the line is no longer in the
/// final body (e.g. later deleted by the loop).
fn marginal_d_test(
    final_body: &str,
    final_protected: &str,
    line: &str,
    seed: u64,
    spec: &crate::skillopt::target::SplitSpec,
    d_tr_frac: f64,
) -> Option<f64> {
    let mut lines: Vec<&str> = final_body.lines().collect();
    let pos = lines.iter().position(|l| *l == line)?;
    lines.remove(pos);
    let target = MixedTarget;
    let splits = make_splits(Family::FOrder, seed, d_tr_frac, spec);
    // Rebuild the final document exactly (body + protected); only the
    // body line is ablated. `set_protected` is pub(crate): reachable
    // here, unreachable to step-level edits — the same boundary the
    // task relies on.
    let mut base_doc = SkillDoc::experiment(final_body);
    base_doc.set_protected(final_protected);
    let mut ablated = SkillDoc::experiment(&lines.join("\n"));
    ablated.set_protected(final_protected);
    Some((target.score(&base_doc, &splits.d_test) - target.score(&ablated, &splits.d_test)) * 100.0)
}

/// Seed-level acceptance/D_test analysis: Pearson r between acceptance
/// count and D_test gain, plus the per-edit ablation marginals.
struct AcceptanceAnalysis {
    pearson_r: f64,
    mean_marginal: f64,
    frac_nonpositive: f64,
    n_edits: usize,
}

fn pearson(xs: &[f64], ys: &[f64]) -> f64 {
    if xs.len() != ys.len() || xs.len() < 2 {
        return f64::NAN;
    }
    let (mx, _) = mean_std(xs);
    let (my, _) = mean_std(ys);
    let cov: f64 = xs
        .iter()
        .zip(ys.iter())
        .map(|(x, y)| (x - mx) * (y - my))
        .sum();
    let vx: f64 = xs.iter().map(|x| (x - mx).powi(2)).sum();
    let vy: f64 = ys.iter().map(|y| (y - my).powi(2)).sum();
    if vx <= 0.0 || vy <= 0.0 {
        return f64::NAN;
    }
    cov / (vx * vy).sqrt()
}

fn analyze_acceptance(
    logs: &[SeedLog],
    spec: &crate::skillopt::target::SplitSpec,
) -> AcceptanceAnalysis {
    let xs: Vec<f64> = logs.iter().map(|l| l.accepted_edits.len() as f64).collect();
    let ys: Vec<f64> = logs
        .iter()
        .map(|l| (l.d_test - l.d_test_initial) * 100.0)
        .collect();
    let pearson_r = pearson(&xs, &ys);
    let mut marginals = Vec::new();
    for log in logs {
        for line in &log.accepted_edits {
            if let Some(m) = marginal_d_test(
                &log.final_body,
                &log.final_protected,
                line,
                log.seed as u64,
                spec,
                1.0,
            ) {
                marginals.push(m);
            }
        }
    }
    let n_edits = marginals.len();
    let (mean_marginal, _) = mean_std(&marginals);
    let frac_nonpositive = if n_edits == 0 {
        f64::NAN
    } else {
        marginals.iter().filter(|m| **m <= 0.0).count() as f64 / n_edits as f64
    };
    AcceptanceAnalysis {
        pearson_r,
        mean_marginal,
        frac_nonpositive,
        n_edits,
    }
}

fn metrics_json(
    small: &ArmStats,
    large: &ArmStats,
    three: &ArmStats,
    verdict: Verdict,
    detail: &str,
) -> serde_json::Value {
    let shrinkage = if small.gap.abs() < 1e-9 {
        0.0
    } else {
        (small.gap - three.gap) / small.gap
    };
    serde_json::json!({
        "verdict": verdict.to_string(),
        "small": {"n": small.n, "sel_gain": small.sel_gain, "test_gain": small.test_gain,
                  "gap": small.gap, "gap_std": small.gap_std, "d_test_reads": small.d_test_reads,
                  "n_accepted_mean": small.n_accepted},
        "large": {"n": large.n, "sel_gain": large.sel_gain, "test_gain": large.test_gain,
                  "gap": large.gap, "gap_std": large.gap_std, "d_test_reads": large.d_test_reads,
                  "n_accepted_mean": large.n_accepted},
        "three_split": {"n": three.n, "sel_gain": three.sel_gain, "test_gain": three.test_gain,
                  "gap": three.gap, "gap_std": three.gap_std, "d_test_reads": three.d_test_reads,
                  "n_accepted_mean": three.n_accepted},
        "gap_shrinkage_frac": shrinkage,
        "bars": {"overfit_gap_pts": OVERFIT_GAP_PTS, "contained_gap_pts": CONTAINED_GAP_PTS,
                 "shrinkage_frac": SHRINKAGE_FRAC},
        "detail": detail,
        "backend": "scripted-mock",
    })
}

/// V1: small vs large D_sel — measure the overfitting gap with D_test sealed.
fn case_small_vs_large_gap() -> Result<CaseReport, TaskDriverError> {
    let (small_logs, large_logs, three_logs) = run_arms()?;
    let small = summarize_arm(&small_logs);
    let large = summarize_arm(&large_logs);
    let three = summarize_arm(&three_logs);
    let (verdict, detail) = classify(&small, &three);
    let mut evidence = vec![
        format!(
            "small D_sel=16 (n={}): sel_gain={:.2} test_gain={:.2} gap={:.2}±{:.2}",
            small.n, small.sel_gain, small.test_gain, small.gap, small.gap_std
        ),
        format!(
            "large D_sel=64 (n={}): sel_gain={:.2} test_gain={:.2} gap={:.2}±{:.2}",
            large.n, large.sel_gain, large.test_gain, large.gap, large.gap_std
        ),
        format!(
            "three-split A=16/B=16 (n={}): sel_gain={:.2} test_gain={:.2} gap={:.2}±{:.2}",
            three.n, three.sel_gain, three.test_gain, three.gap, three.gap_std
        ),
        format!(
            "D_test reads during training: {} (seal)",
            small.d_test_reads + large.d_test_reads + three.d_test_reads
        ),
        verdict_line("111", verdict, &detail),
    ];
    let mut failures = Vec::new();
    if small.n < 5 || large.n < 5 || three.n < 5 {
        failures.push("each arm needs at least 5 seeds".to_string());
    }
    if small.d_test_reads + large.d_test_reads + three.d_test_reads != 0 {
        failures.push("D_test seal broken: training-time reads detected".to_string());
    }
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        metrics_json(&small, &large, &three, verdict, &detail),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: three-split confirmation — the B split must actually confirm.
fn case_three_split_containment() -> Result<CaseReport, TaskDriverError> {
    let (_, _, three_logs) = run_arms()?;
    let three = summarize_arm(&three_logs);
    let mut evidence = vec![format!(
        "three-split arm: {} seeds, mean accepted {:.1}",
        three.n, three.n_accepted
    )];
    let mut failures = Vec::new();
    // The mechanism must have run: every step carries B scores.
    let mut steps_total = 0usize;
    let mut steps_with_b = 0usize;
    let mut both_strict_holds = true;
    for log in &three_logs {
        for st in &log.steps {
            steps_total += 1;
            match (st.confirm_before, st.confirm_after) {
                (Some(b0), Some(b1)) => {
                    steps_with_b += 1;
                    if st.accepted && b1.partial_cmp(&b0) != Some(std::cmp::Ordering::Greater) {
                        both_strict_holds = false;
                    }
                }
                _ => failures.push(format!(
                    "seed {} step {}: missing D_selB scores",
                    log.seed, st.step
                )),
            }
        }
    }
    evidence.push(format!(
        "D_selB scored on {steps_with_b}/{steps_total} steps; \
         every accepted step strictly improved B: {both_strict_holds}"
    ));
    if !both_strict_holds {
        failures.push("an accepted step did not strictly improve D_selB".to_string());
    }
    if three.d_test_reads != 0 {
        failures.push("D_test seal broken on the three-split arm".to_string());
    }
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "steps_total": steps_total, "steps_with_b": steps_with_b,
            "both_strict_holds": both_strict_holds,
            "d_test_reads": three.d_test_reads,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1 (adversarial): the seal must be a live tripwire, not a hardcoded
/// zero. Positive control: an UNSEALED arm must count exactly one
/// training-time D_test read (the seed-setup baseline).
fn case_seal_holds() -> Result<CaseReport, TaskDriverError> {
    let (small_logs, _, _) = run_arms()?;
    let sealed_reads: u64 = small_logs.iter().map(|l| l.d_test_reads).sum();
    // Positive control: same arm, seal off.
    let mut cfg = arm_cfg(16, false);
    cfg.sealed_d_test = false;
    let opt = ScriptedOptimizer::new();
    let unsealed = Learner
        .run_arm(&cfg, &opt)
        .map_err(|e| TaskDriverError::Arm {
            arm: "unsealed-control".to_string(),
            detail: e.to_string(),
        })?;
    let unsealed_reads: Vec<u64> = unsealed.iter().map(|l| l.d_test_reads).collect();
    let mut evidence = vec![
        format!("sealed arm training-time D_test reads: {sealed_reads} (must be 0)"),
        format!("unsealed control reads per seed: {unsealed_reads:?} (must be all 1)"),
    ];
    let mut failures = Vec::new();
    if sealed_reads != 0 {
        failures.push(format!(
            "seal broken: {sealed_reads} training-time D_test reads on the sealed arm"
        ));
    }
    if unsealed_reads.iter().any(|r| *r != 1) {
        failures.push(
            "positive control failed: the unsealed arm did not count its seed-setup D_test read \
             — the counter is not live"
                .to_string(),
        );
    }
    // Structural check: sealed arms must still report both baselines
    // (post-hoc evaluation is the seal's legitimate exception).
    for log in &small_logs {
        if !log.d_test_initial.is_finite() || !log.d_test.is_finite() {
            failures.push(format!("seed {}: sealed baselines not finite", log.seed));
        }
    }
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "sealed_reads": sealed_reads,
            "unsealed_reads": unsealed_reads,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2 (adversarial): hunt for accepted edits the gate should not have
/// taken — D_test-neutral or D_test-harmful accepts are the
/// overfitting mechanism caught red-handed. Per-edit ablation
/// marginals (post-hoc, seal-safe) plus the seed-level
/// acceptance/D_test-gain correlation.
fn case_acceptance_d_test_correlation() -> Result<CaseReport, TaskDriverError> {
    let (small_logs, large_logs, three_logs) = run_arms()?;
    let spec = default_spec();
    let small_a = analyze_acceptance(&small_logs, &spec);
    let large_a = analyze_acceptance(&large_logs, &spec);
    let three_a = analyze_acceptance(&three_logs, &spec);
    let mut evidence = vec![
        format!(
            "small: r(accepted, d_test_gain)={:.3}, mean marginal={:.2}pts, \
             frac nonpositive={:.2} (n={} edits)",
            small_a.pearson_r, small_a.mean_marginal, small_a.frac_nonpositive, small_a.n_edits
        ),
        format!(
            "large: r={:.3}, mean marginal={:.2}pts, frac nonpositive={:.2} (n={} edits)",
            large_a.pearson_r, large_a.mean_marginal, large_a.frac_nonpositive, large_a.n_edits
        ),
        format!(
            "three-split: r={:.3}, mean marginal={:.2}pts, frac nonpositive={:.2} (n={} edits)",
            three_a.pearson_r, three_a.mean_marginal, three_a.frac_nonpositive, three_a.n_edits
        ),
    ];
    let mut failures = Vec::new();
    for (name, a) in [
        ("small", &small_a),
        ("large", &large_a),
        ("three-split", &three_a),
    ] {
        if a.n_edits == 0 {
            failures.push(format!(
                "{name}: no accepted edits ablated — vacuous analysis"
            ));
        }
        if a.pearson_r.is_nan() {
            evidence.push(format!(
                "{name}: pearson r undefined (no variance) — reported, not hidden"
            ));
        }
    }
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "small": {"r": small_a.pearson_r, "mean_marginal": small_a.mean_marginal,
                      "frac_nonpositive": small_a.frac_nonpositive, "n_edits": small_a.n_edits},
            "large": {"r": large_a.pearson_r, "mean_marginal": large_a.mean_marginal,
                      "frac_nonpositive": large_a.frac_nonpositive, "n_edits": large_a.n_edits},
            "three_split": {"r": three_a.pearson_r, "mean_marginal": three_a.mean_marginal,
                      "frac_nonpositive": three_a.frac_nonpositive, "n_edits": three_a.n_edits},
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
        "small_vs_large_gap" => case_small_vs_large_gap(),
        "three_split_containment" => case_three_split_containment(),
        "seal_holds" => case_seal_holds(),
        "acceptance_d_test_correlation" => case_acceptance_d_test_correlation(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-111: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-111".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-111".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
