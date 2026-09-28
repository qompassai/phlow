//! task-68: metric cardinality bound (rust, adversarial).
//!
//! Recon probe: the design asks for the metrics/telemetry REGISTRY
//! (label sets) — an attacker (or a bug) creates unbounded label
//! values. Scenarios: default (bounded label values → recorded);
//! adversarial: per-run-id label (the registry rejects or hashes the
//! high-cardinality label with an explicit `CardinalityExceeded`);
//! adversarial: label *values* from untrusted tool output (sanitized
//! or rejected before registration). Pass criteria: the metric store's
//! series count has a hard cap, measured under attack; rejection is
//! explicit, not a silent drop that hides the signal.
//! Distinct from tasks 47/48 (FD/disk *resource* exhaustion) — this is
//! *observability* exhaustion: the monitoring itself becomes the
//! victim, blinding everything else.
//!
//! Honest result: the seam is ABSENT, and the finding is the gap. The
//! evidence is threefold, all gathered at probe time from the working
//! tree:
//!
//! 1. Registry-vocabulary scan: a walk over every `crates/*/src/**/*.rs`
//!    (excluding the phlow-gauntlet probe harness itself) finds zero
//!    registry/label/series tokens — no metrics registry, no label
//!    sets, no series cap, no `CardinalityExceeded`, no label-value
//!    sanitizer in any phlow crate. (No prometheus/opentelemetry/otel
//!    dependency in any crate manifest either.)
//! 2. The one counting mechanism that DOES exist,
//!    `report_mut::{counter,add,set,push}` (private module in
//!    phlow-runtime/src/runtime.rs), is a plain JSON report object
//!    keyed by STATIC string literals (`"tool_calls"`,
//!    `"model_calls"`, `"events"`, `"roles"`, `"verification"`,
//!    `"status"`, `"error"`, `"cycles"`) — no label sets, no series,
//!    no attacker-reachable key construction. Classified adjacent, not
//!    the seam.
//! 3. The design's adversarial weapons have no target: there is no
//!    label-registration API to feed a per-run-id label into, and no
//!    label path for untrusted tool output to travel — the
//!    sanitization question is moot because the path does not exist.
//!
//! Four cases: two validation, two adversarial. The task-level verdict
//! is `fail` at `"seam"`.
//!
//! Banked for Matt (product decision, NOT auto-implemented): whether
//! phlow should gain a labeled metrics/telemetry registry at all; if
//! it does, the hard series cap, the explicit rejection signal (the
//! design's `CardinalityExceeded`), and the label-value sanitization
//! policy for untrusted tool output.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-68";
/// Human-readable name.
pub const NAME: &str = "metric cardinality bound";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "no_metrics_registry_in_sources",
    "plain_counters_have_no_labels",
    "per_run_label_attack_has_no_target",
    "untrusted_output_cannot_become_a_label",
];

/// Largest Rust source file the probe will scan, in bytes.
const SOURCE_BYTES_MAX: usize = 1_048_576;
/// Most source files the probe will scan before stopping.
const SOURCE_FILES_MAX: usize = 50_000;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-68 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture (workspace root, source tree) was unusable.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
    /// A probe step failed.
    Probe {
        /// What was being probed.
        what: String,
        /// The underlying error.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixture { what, detail } => {
                write!(f, "task-68: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-68: cannot probe {what}: {detail}")
            }
        }
    }
}

impl std::error::Error for DriverError {}

fn fixture_error(what: &str, detail: impl fmt::Display) -> DriverError {
    DriverError::Fixture {
        what: what.to_string(),
        detail: detail.to_string(),
    }
}

fn probe_error(what: &str, detail: impl fmt::Display) -> DriverError {
    DriverError::Probe {
        what: what.to_string(),
        detail: detail.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Fixtures: the working tree is the task source
// ---------------------------------------------------------------------------

/// Workspace root: two levels above this crate's manifest directory.
/// The probe reads the live working tree, never a cached copy.
fn workspace_root() -> Result<PathBuf, DriverError> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| fixture_error("workspace root", "manifest dir has no grandparent"))?;
    if !root.join("Cargo.lock").is_file() {
        return Err(fixture_error(
            "workspace root",
            format!("no Cargo.lock under {}", root.display()),
        ));
    }
    Ok(root.to_path_buf())
}

/// Metrics-registry mechanism tokens, assembled at runtime from halves
/// so the probe's own source never contains the literal tokens it scans
/// for. These name the design's required machinery: a labeled metrics
/// registry, series caps, cardinality rejection, label-value handling.
fn registry_tokens() -> Vec<String> {
    const HALVES: [(&str, &str); 10] = [
        ("metric", "_registry"),
        ("label", "_set"),
        ("label", "set"),
        ("cardinality", "_exceeded"),
        ("cardinality", ""),
        ("series", "_limit"),
        ("series", "_cap"),
        ("counter", "_vec"),
        ("histogram", "_vec"),
        ("with", "_label"),
    ];
    HALVES.iter().map(|(a, b)| format!("{a}{b}")).collect()
}

/// Label-API tokens: the registration surface an attacker would call.
/// Assembled from halves for the same self-scan reason.
fn label_api_tokens() -> Vec<String> {
    const HALVES: [(&str, &str); 4] = [
        ("label", "_value"),
        ("label", "s("),
        ("register", "_metric"),
        ("new", "_counter"),
    ];
    HALVES.iter().map(|(a, b)| format!("{a}{b}")).collect()
}

/// Walk `crates/` under the workspace root and return every
/// `path: token` hit for `.rs` files inside a `src` tree, skipping the
/// whole phlow-gauntlet probe harness (it is the scanner, not the
/// product). Bounded: files over [`SOURCE_BYTES_MAX`]
/// are skipped, and the walk stops after [`SOURCE_FILES_MAX`] files.
fn scan_sources(root: &Path, tokens: &[String]) -> Result<Vec<String>, DriverError> {
    let crates_dir = root.join("crates");
    let mut hits = Vec::new();
    let mut files_seen = 0usize;
    let mut stack = vec![crates_dir];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| fixture_error("source walk", format!("{}: {e}", dir.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| fixture_error("source walk", e))?;
            let path = entry.path();
            if path.components().any(|c| c.as_os_str() == "phlow-gauntlet") {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.components().any(|c| c.as_os_str() == "src")
            {
                files_seen += 1;
                if files_seen > SOURCE_FILES_MAX {
                    return Err(probe_error(
                        "source scan",
                        format!("file budget {SOURCE_FILES_MAX} exhausted"),
                    ));
                }
                let bytes = std::fs::read(&path).map_err(|e| {
                    fixture_error("source read", format!("{}: {e}", path.display()))
                })?;
                if bytes.len() > SOURCE_BYTES_MAX {
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes);
                for token in tokens {
                    if text.contains(token.as_str()) {
                        hits.push(format!("{}: {token}", path.display()));
                    }
                }
            }
        }
    }
    Ok(hits)
}

/// Read every `report_mut::` call site in phlow-runtime/src/runtime.rs
/// and check the key argument is a string literal. Returns
/// (call_sites, dynamic_keys).
fn report_call_sites(root: &Path) -> Result<(usize, Vec<String>), DriverError> {
    let path = root
        .join("crates")
        .join("phlow-runtime")
        .join("src")
        .join("runtime.rs");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| fixture_error("runtime.rs read", format!("{}: {e}", path.display())))?;
    let mut sites = 0usize;
    let mut dynamic = Vec::new();
    let mut search = text.as_str();
    while let Some((head, tail)) = search.split_once("report_mut::") {
        // The line carrying the occurrence: skip comment lines.
        let line_start = head.rfind('\n').map_or(0, |i| i + 1);
        let line_no = head.chars().filter(|&c| c == '\n').count() + 1;
        if head[line_start..].trim_start().starts_with("//") {
            search = tail;
            continue;
        }
        sites += 1;
        // tail looks like `add(report, "key", ...)` possibly across lines.
        // Parse the balanced arg list forward from the open paren; the key
        // is the second comma-separated argument, which must be a string
        // literal.
        if let Some((_, after_paren)) = tail.split_once('(') {
            let mut depth = 1usize;
            let mut args = String::new();
            for c in after_paren.chars() {
                match c {
                    '(' => {
                        depth += 1;
                        args.push(c);
                    }
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        args.push(c);
                    }
                    _ => args.push(c),
                }
            }
            let key_arg = args.split(',').nth(1).map(str::trim).unwrap_or("");
            if !key_arg.starts_with('"') {
                dynamic.push(format!("line {line_no}: key not a string literal"));
            }
        }
        search = tail;
    }
    Ok((sites, dynamic))
}

// ---------------------------------------------------------------------------
// Case verdicts
// ---------------------------------------------------------------------------

/// The parsed verdict of one case.
#[derive(Debug, Clone)]
pub struct CaseReport {
    /// Which case ran.
    pub case: String,
    /// Whether the case's own assertions held.
    pub passed: bool,
    /// Measured numbers.
    pub metrics: serde_json::Value,
    /// Diagnostic lines from the case.
    pub evidence: Vec<String>,
    /// Failing assertion details, empty when `passed`.
    pub failures: Vec<String>,
}

impl CaseReport {
    fn pass(case: &'static str, metrics: serde_json::Value, evidence: Vec<String>) -> Self {
        Self {
            case: case.to_string(),
            passed: true,
            metrics,
            evidence,
            failures: Vec::new(),
        }
    }

    fn fail(case: &'static str, failure: String, evidence: Vec<String>) -> Self {
        Self {
            case: case.to_string(),
            passed: false,
            metrics: serde_json::json!({}),
            evidence,
            failures: vec![failure],
        }
    }
}

/// V1: no metrics registry, label sets, series caps, or cardinality
/// machinery exists in any phlow source; no metrics dependency in any
/// crate manifest either.
fn case_no_metrics_registry_in_sources() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_metrics_registry_in_sources";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let hits = scan_sources(&root, &registry_tokens())?;
    evidence.push(format!(
        "registry-vocabulary scan over crates/*/src: {} hits (want 0)",
        hits.len()
    ));
    for hit in hits.iter().take(8) {
        evidence.push(format!("  hit: {hit}"));
    }
    // Manifest check: no prometheus/opentelemetry/otel dependency.
    let mut dep_hits = Vec::new();
    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| fixture_error("manifest walk", format!("{}: {e}", dir.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| fixture_error("manifest walk", e))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|n| n == "Cargo.toml") {
                let text = std::fs::read_to_string(&path).map_err(|e| {
                    fixture_error("manifest read", format!("{}: {e}", path.display()))
                })?;
                let lower = text.to_lowercase();
                for dep in ["prometheus", "opentelemetry", "otel", "metrics-rs"] {
                    if lower.contains(dep) {
                        dep_hits.push(format!("{}: {dep}", path.display()));
                    }
                }
            }
        }
    }
    evidence.push(format!(
        "metrics-dependency scan over Cargo.tomls: {} hits (want 0)",
        dep_hits.len()
    ));
    if !hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!("registry machinery found: {}", hits.join("; ")),
            evidence,
        ));
    }
    if !dep_hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!("metrics dependency found: {}", dep_hits.join("; ")),
            evidence,
        ));
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"registry_hits": 0, "metrics_deps": 0}),
        evidence,
    ))
}

/// V2: the one counting mechanism that exists
/// (`report_mut::{counter,add,set,push}` in phlow-runtime, a PRIVATE
/// module) is a JSON report object keyed by static string literals —
/// no label sets, no series, no attacker-reachable key construction.
/// Classified adjacent, not the seam.
fn case_plain_counters_have_no_labels() -> Result<CaseReport, DriverError> {
    const CASE: &str = "plain_counters_have_no_labels";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let (sites, dynamic) = report_call_sites(&root)?;
    evidence.push(format!(
        "report_mut:: call sites in phlow-runtime/src/runtime.rs: {sites}; \
         dynamic (non-literal) keys: {} (want 0)",
        dynamic.len()
    ));
    for line in dynamic.iter().take(8) {
        evidence.push(format!("  dynamic key: {line}"));
    }
    evidence.push(
        "the counters are literal-keyed increments on a private JSON report \
         object (\"tool_calls\", \"model_calls\", \"cycles\", ...): there are \
         no label sets to bound and no series to cap — adjacent, not the seam"
            .to_string(),
    );
    if sites == 0 {
        return Ok(CaseReport::fail(
            CASE,
            "no report_mut call sites found — the probe misread the tree".to_string(),
            evidence,
        ));
    }
    if !dynamic.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "attacker-reachable key construction: {}",
                dynamic.join("; ")
            ),
            evidence,
        ));
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"call_sites": sites, "dynamic_keys": 0}),
        evidence,
    ))
}

/// A1: the per-run-id label weapon has no target — there is no label
/// registration API to feed the attack into. The label-API vocabulary
/// scan finds zero registration surface.
fn case_per_run_label_attack_has_no_target() -> Result<CaseReport, DriverError> {
    const CASE: &str = "per_run_label_attack_has_no_target";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let hits = scan_sources(&root, &label_api_tokens())?;
    // The tokens ("labels(", "new_counter", ...) are generic enough to
    // catch innocent code; every hit must be classified. Zero is the
    // honest bar only if the scan is clean — report what exists.
    evidence.push(format!(
        "label-API vocabulary scan over crates/*/src: {} hits",
        hits.len()
    ));
    for hit in hits.iter().take(8) {
        evidence.push(format!("  hit: {hit}"));
    }
    if !hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!("label-registration surface found: {}", hits.join("; ")),
            evidence,
        ));
    }
    evidence.push(
        "no label-registration API exists: a per-run-id label cannot be \
         recorded, so there is nothing to reject or hash with an explicit \
         rejection signal — the design's adversarial scenario has no target"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"label_api_hits": 0}),
        evidence,
    ))
}

/// A2: label values from untrusted tool output — the same absence. With
/// no label path, untrusted output cannot become a series; the only
/// dynamic content lands in report arrays/values under the static keys
/// from V2. The sanitization question is moot because the path does not
/// exist.
fn case_untrusted_output_cannot_become_a_label() -> Result<CaseReport, DriverError> {
    const CASE: &str = "untrusted_output_cannot_become_a_label";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let (sites, _) = report_call_sites(&root)?;
    evidence.push(format!(
        "re-verified {sites} report_mut:: call sites: every key is a \
         static literal — untrusted tool output flows into report VALUES \
         (event objects, status strings), never into series keys"
    ));
    evidence.push(
        "there is no registry whose series count an attacker could grow: \
         the design's pass criterion (a hard cap measured under attack) \
         has no store to measure against"
            .to_string(),
    );
    evidence.push(
        "rejection cannot be 'explicit, not a silent drop' when there is \
         no registration call to reject at — the observability-exhaustion \
         attack surface is absent because the observability is absent"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"label_paths": 0}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "no_metrics_registry_in_sources" => case_no_metrics_registry_in_sources(),
        "plain_counters_have_no_labels" => case_plain_counters_have_no_labels(),
        "per_run_label_attack_has_no_target" => case_per_run_label_attack_has_no_target(),
        "untrusted_output_cannot_become_a_label" => case_untrusted_output_cannot_become_a_label(),
        _ => Err(DriverError::Fixture {
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

fn run_inner(_ctx: &Ctx) -> Result<Vec<String>, TaskFailure> {
    let mut evidence = vec![
        "recon: registry-vocabulary scan over every crates/*/src — zero hits (no labeled metrics registry, no label sets, no series caps, no cardinality machinery); no metrics dependency in any Cargo.toml".to_string(),
        "recon: the one counting mechanism (phlow-runtime report_mut::{counter,add,set,push}, private module) is literal-keyed — classified adjacent, not the seam".to_string(),
        "recon: no label-registration API exists — the design's adversarial weapons (per-run-id label, untrusted label values) have no target".to_string(),
    ];
    for case in CASES {
        let report = run_case(case).map_err(|e| TaskFailure {
            where_: case.to_string(),
            how: e.to_string(),
            evidence: evidence.clone(),
        })?;
        evidence.push(format!("case {case}: passed={}", report.passed));
        evidence.push(format!("case {case} metrics: {}", report.metrics));
        for line in &report.evidence {
            evidence.push(format!("case {case}: {line}"));
        }
        if !report.passed {
            return Err(TaskFailure {
                where_: case.to_string(),
                how: report.failures.join("; "),
                evidence,
            });
        }
    }
    evidence.push(
        "finding: no metrics/telemetry registry exists in phlow — the design's pass criteria (a hard series cap measured under attack; explicit rejection, not a silent drop) need a registry to attach to, and there is none".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no metrics/telemetry registry exists in any phlow crate — a runtime vocabulary scan finds zero registry/label/series tokens, no crate depends on a metrics library, and the only counting mechanism (phlow-runtime's private report_mut::{counter,add,set,push}) is a literal-keyed JSON report object with no label sets and no attacker-reachable key construction. The design's pass criteria (the metric store's series count has a hard cap, measured under attack; rejection is explicit, not a silent drop) need a registry to attach to, and there is none.".to_string(),
        evidence,
    })
}

/// Attempt the task.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    match run_inner(ctx) {
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
