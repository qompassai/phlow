//! The design's calibration report: current Brier score and ECE,
//! per-question-type calibration curves, the thresholds verdict,
//! and — always — the state of each source, so an absent or
//! signalless source is part of the record rather than silently
//! dropped.

use crate::collector::{Collection, SourceState};
use crate::metrics::{GateVerdict, KindMetrics, Metrics, SplitMetrics, gate_verdict};

/// Render the full report for a collection. The verdict judges
/// the held-out scope, per the design (the reserve is never
/// trained on, so it is the honest measure).
pub fn render_report(collection: &Collection, split: &SplitMetrics) -> String {
    let mut out = String::new();
    out.push_str("phlow calibration report\n");
    out.push_str("========================\n\n");
    push_sources(&mut out, collection);
    push_scope(&mut out, "held-out (validation reserve)", &split.held_out);
    push_scope(&mut out, "train (all other tuples)", &split.train);
    push_verdict(&mut out, &split.held_out);
    out
}

/// One metrics scope: overall numbers, per-kind numbers, curves.
fn push_scope(out: &mut String, title: &str, metrics: &Metrics) {
    out.push_str(&format!("## {title}\n\n"));
    out.push_str(&format!(
        "tuples: {} (labeled {}, distribution-labeled {})\n",
        metrics.tuples, metrics.labeled, metrics.distribution_labeled
    ));
    out.push_str(&format!(
        "brier: {}\n",
        format_score(metrics.brier, "no labeled tuples")
    ));
    out.push_str(&format!(
        "brier (full distribution): {}\n",
        format_score(
            metrics.brier_full_distribution,
            "no distribution-labeled tuples"
        )
    ));
    out.push_str(&format!(
        "ece: {}\n\n",
        format_score(metrics.ece, "no labeled tuples")
    ));
    for kind_metrics in metrics.per_kind.values() {
        push_kind(out, kind_metrics);
    }
}

/// One question kind's measurements and calibration curve.
fn push_kind(out: &mut String, kind: &KindMetrics) {
    out.push_str(&format!("### {}\n\n", kind.kind.label()));
    out.push_str(&format!(
        "tuples: {} (labeled {}, distribution-labeled {})\n",
        kind.tuples, kind.labeled, kind.distribution_labeled
    ));
    out.push_str(&format!(
        "brier: {} | brier (full distribution): {} | ece: {}\n",
        format_score(kind.brier, "n/a"),
        format_score(kind.brier_full_distribution, "n/a"),
        format_score(kind.ece, "n/a")
    ));
    if kind.labeled == 0 {
        out.push_str("calibration curve: no labeled tuples\n\n");
        return;
    }
    out.push_str("calibration curve (bin: count, mean confidence, accuracy):\n");
    for bin in &kind.curve.bins {
        match (bin.mean_confidence, bin.accuracy) {
            (Some(confidence), Some(accuracy)) => {
                out.push_str(&format!(
                    "  [{:.1}, {:.1}): {}, {:.3}, {:.3}\n",
                    bin.lower, bin.upper, bin.count, confidence, accuracy
                ));
            }
            _ => {
                out.push_str(&format!(
                    "  [{:.1}, {:.1}): 0, -, -\n",
                    bin.lower, bin.upper
                ));
            }
        }
    }
    out.push('\n');
}

/// The source states section.
fn push_sources(out: &mut String, collection: &Collection) {
    out.push_str("## sources\n\n");
    match &collection.gauntlet {
        Some(gauntlet) => {
            let passed = gauntlet.outcomes.values().filter(|o| o.passed).count();
            out.push_str(&format!(
                "gauntlet traces: {} decision traces, {} task outcomes ({} passed, {} failed), \
                 {} duplicate outcome records; {} held-out tuples\n",
                gauntlet.trace_records,
                gauntlet.outcomes.len(),
                passed,
                gauntlet.outcomes.len() - passed,
                gauntlet.duplicate_outcomes,
                gauntlet.tuples.iter().filter(|t| t.held_out).count()
            ));
            let unlabeled = gauntlet
                .tuples
                .iter()
                .filter(|t| t.ground_truth.is_none())
                .count();
            out.push_str(&format!(
                "gauntlet labels: {} labeled, {} pending (ground_truth null by design until a \
                 real label exists; pending tuples are counted, never scored)\n",
                gauntlet.tuples.len() - unlabeled,
                unlabeled
            ));
        }
        None => out.push_str("gauntlet traces: not configured\n"),
    }
    match collection.feedback.state {
        SourceState::Present => {
            out.push_str(&format!(
                "feedback store: present, {} entries",
                collection.feedback.entries.len()
            ));
            match collection.feedback.average_rating {
                Some(rating) => out.push_str(&format!(", average rating {rating:.2}")),
                None => out.push_str(", no ratings"),
            }
            out.push_str("; session-level signals only (no decision points) — yields no tuples\n");
        }
        SourceState::Absent => {
            out.push_str("feedback store: ABSENT (no feedback.db at the configured path)\n")
        }
    }
    match collection.approvals.state {
        SourceState::Present => {
            out.push_str(&format!(
                "consumed-approvals store: present, {} ids; replay-protection records with \
                 no decision content — yields no tuples\n",
                collection.approvals.consumed_ids.len()
            ));
        }
        SourceState::Absent => {
            out.push_str("consumed-approvals store: ABSENT (no store at the configured path)\n");
        }
    }
    out.push_str(
        "approval decision history: persisted nowhere in phlow today; the design's \
         2.0-weight approval source contributes no tuples until a producer writes it\n\n",
    );
}

/// The Phase A verdict section (design thresholds).
fn push_verdict(out: &mut String, held_out: &Metrics) {
    out.push_str("## phase A verdict (held-out scope)\n\n");
    out.push_str("thresholds: brier < 0.1 and ece < 0.05 = well-calibrated\n");
    match gate_verdict(held_out) {
        GateVerdict::WellCalibrated => {
            out.push_str("verdict: WELL-CALIBRATED — below both thresholds; no training needed\n");
        }
        GateVerdict::NotCalibrated => {
            out.push_str(
                "verdict: NOT CALIBRATED — at least one threshold missed; the design's \
                 answer is more high-quality tuples and fine-tuning on the weighted export, \
                 not lower standards\n",
            );
        }
        GateVerdict::InsufficientData => {
            out.push_str(
                "verdict: INSUFFICIENT DATA — no labeled held-out tuples, so calibration \
                 cannot be judged yet; labels, not estimates, are what is missing\n",
            );
        }
    }
}

/// A score with four decimals, or the honest absence text.
fn format_score(score: Option<f64>, absent: &str) -> String {
    match score {
        Some(value) => format!("{value:.4}"),
        None => absent.to_string(),
    }
}
