//! The canary report and the split-run log.
//!
//! Report format is the design's JSONL: one header line (canary
//! version, model, artifact hash, timestamp, verdict), then one line
//! per probe with its bounded evidence. Split runs — repeated runs of
//! one probe that disagree — are part of the report and are also
//! appended to a standing JSONL log: they are the calibration labels
//! the deferred disagreement resolver will be built on, so they are
//! logged from this first build (2026-10-07 amendment obligation).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use serde::Serialize;

use crate::error::CanaryError;
use crate::probe::ProbeResult;
use crate::verdict::{Verdict, write_atomic_owner_only};

/// One run's outcome inside a split, as logged.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunOutcome {
    /// Whether this run passed.
    pub passed: bool,
    /// This run's mean top-option confidence, where measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean_confidence: Option<f64>,
}

/// A probe whose repeated runs disagreed (e.g. 2/3), with the
/// confidence distribution across those runs. Split runs fail the
/// probe (the rule is 3/3), and they are logged either way.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SplitRun {
    /// The probe that split.
    pub probe_id: String,
    /// Per-run outcomes, in execution order.
    pub runs: Vec<RunOutcome>,
    /// Variance of the per-run mean confidences (runs that measured
    /// one), the disagreement signal the resolver will consume.
    pub mean_confidence_variance: f64,
}

/// The full record of one battery run.
#[derive(Debug, Clone, PartialEq)]
pub struct CanaryReport {
    /// Canary version that produced this report, e.g. "canary-v1".
    pub canary_version: String,
    /// Model identity as the operator names it.
    pub model_id: String,
    /// SHA-256 of the exact artifact tested.
    pub model_hash: String,
    /// Run time, seconds since the unix epoch (supplied by the
    /// caller so library runs are reproducible in tests).
    pub timestamp_unix: u64,
    /// The fail-closed verdict.
    pub verdict: Verdict,
    /// Final per-probe results, in registry order.
    pub results: Vec<ProbeResult>,
    /// Split runs observed during this battery, if any.
    pub split_runs: Vec<SplitRun>,
    /// Total backend calls the battery made (budget accounting).
    pub backend_calls: usize,
    /// Wall-clock duration of the battery in milliseconds.
    pub elapsed_ms: u64,
}

/// The JSONL header line.
#[derive(Debug, Serialize)]
struct HeaderLine<'a> {
    canary_version: &'a str,
    model: &'a str,
    model_hash: &'a str,
    timestamp_unix: u64,
    verdict: &'a str,
    failed: &'a [String],
}

/// One JSONL probe line.
#[derive(Debug, Serialize)]
struct ProbeLine<'a> {
    probe_id: &'a str,
    passed: bool,
    evidence: &'a crate::probe::ProbeEvidence,
}

/// One line of the standing split-run log.
#[derive(Debug, Serialize)]
struct SplitLogLine<'a> {
    canary_version: &'a str,
    model: &'a str,
    model_hash: &'a str,
    timestamp_unix: u64,
    probe_id: &'a str,
    runs: &'a [RunOutcome],
    mean_confidence_variance: f64,
}

impl CanaryReport {
    /// Render the report as JSONL: header line, then one line per
    /// probe. Deterministic field order (struct serialization), so
    /// equal reports render to equal bytes.
    pub fn to_jsonl(&self) -> String {
        let mut out = String::new();
        let header = HeaderLine {
            canary_version: &self.canary_version,
            model: &self.model_id,
            model_hash: &self.model_hash,
            timestamp_unix: self.timestamp_unix,
            verdict: self.verdict.as_str(),
            failed: self.verdict.failed(),
        };
        push_json_line(&mut out, &header);
        for result in &self.results {
            let line = ProbeLine {
                probe_id: &result.probe_id,
                passed: result.passed,
                evidence: &result.evidence,
            };
            push_json_line(&mut out, &line);
        }
        out
    }

    /// Write the JSONL report to `path`, atomically, owner-only.
    pub fn write_jsonl(&self, path: &Path) -> Result<(), CanaryError> {
        write_atomic_owner_only(path, self.to_jsonl().as_bytes())
    }

    /// Append this report's split runs to the standing split log at
    /// `path` (created owner-only if absent). A report with no splits
    /// appends nothing. This is the amendment's logging obligation:
    /// splits are calibration labels and are kept from the first
    /// build, even though the resolver that consumes them is deferred.
    pub fn append_split_log(&self, path: &Path) -> Result<(), CanaryError> {
        if self.split_runs.is_empty() {
            return Ok(());
        }
        let mut text = String::new();
        for split in &self.split_runs {
            let line = SplitLogLine {
                canary_version: &self.canary_version,
                model: &self.model_id,
                model_hash: &self.model_hash,
                timestamp_unix: self.timestamp_unix,
                probe_id: &split.probe_id,
                runs: &split.runs,
                mean_confidence_variance: split.mean_confidence_variance,
            };
            push_json_line(&mut text, &line);
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| CanaryError::Io {
                reason: format!("cannot open split log {}: {e}", path.display()),
            })?;
        restrict_owner_only(path)?;
        file.write_all(text.as_bytes())
            .map_err(|e| CanaryError::Io {
                reason: format!("cannot append split log {}: {e}", path.display()),
            })?;
        Ok(())
    }
}

/// Restrict a file to owner-only access on unix (no-op elsewhere).
fn restrict_owner_only(path: &Path) -> Result<(), CanaryError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
            CanaryError::Io {
                reason: format!("cannot restrict {}: {e}", path.display()),
            }
        })?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Serialize one value as a JSON line and append it. Serialization
/// of these closed structs cannot fail in practice; if it ever did,
/// the line is skipped rather than panicking mid-report — the
/// header's verdict line is written first and is what gates read.
fn push_json_line<T: Serialize>(out: &mut String, value: &T) {
    if let Ok(text) = serde_json::to_string(value) {
        out.push_str(&text);
        out.push('\n');
    }
}
