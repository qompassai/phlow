//! task-93: improvement sandbox validation (rust recon + harness).
//!
//! The design asks for every proposal to pass the full gate suite in an
//! *isolated* sandbox before apply — and for the sandbox's isolation to
//! be verified, not assumed: no network, no writes outside the sandbox
//! dir (both asserted), gate results from the sandbox the only trusted
//! validation signal, and sandbox-escape attempts recorded as policy
//! violations rather than mere test failures.
//!
//! Seam mapping (verified, not invented): the sandbox is a DOCUMENTED
//! SKELETON. `phlow-experiment`'s [`Evaluator::prepare`] — "Prepares the
//! isolated workspace and snapshots" — is implemented as
//! `self.advance(EvalStage::Prepare)`: order enforcement only, with the
//! doc comment "Skeleton: order only". The [`Lifecycle::Isolated`]
//! state exists, but nothing creates a real isolated workspace, no
//! network isolation exists anywhere in the workspace, and no
//! write-containment exists either. Bounded exact-token scans over every
//! product crate's `src/**/*.rs` (the gauntlet crate itself excluded,
//! per the harness-probe principle) confirm zero hits for
//! network-isolation and write-containment vocabulary.
//!
//! Four cases: two validation, two adversarial — each a bounded source
//! recon that fails closed (premise changed) if sandbox vocabulary ever
//! appears. The task-level verdict is `fail` at `"seam"`: the isolation
//! is advertised by names and docs but unimplemented.
//!
//! Product-decision bank: whether phlow-experiment should gain a real
//! pre-apply validation sandbox (network-isolated, write-contained,
//! escape attempts typed as policy violations) is Matt's call — not
//! implemented on gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-93";
/// Human-readable name.
pub const NAME: &str = "improvement sandbox validation";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "prepare_is_skeleton",
    "no_network_isolation",
    "no_write_containment",
    "isolation_advertised_but_unimplemented",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-93 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture was unusable.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
    /// A recon probe failed.
    Probe {
        /// Which probe.
        case: String,
        /// The underlying error.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixture { what, detail } => {
                write!(f, "task-93: cannot build fixture {what}: {detail}")
            }
            Self::Probe { case, detail } => {
                write!(f, "task-93: probe {case} failed: {detail}")
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

fn probe_error(case: &str, detail: impl fmt::Display) -> DriverError {
    DriverError::Probe {
        case: case.to_string(),
        detail: detail.to_string(),
    }
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
            failures: vec![],
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

// ---------------------------------------------------------------------------
// Source probe (task_48 scan pattern)
// ---------------------------------------------------------------------------

/// Maximum source files the probe may read.
const SOURCE_FILES_MAX: usize = 4000;
/// Maximum bytes per source file the probe reads.
const SOURCE_BYTES_MAX: usize = 512 * 1024;

/// Workspace root: two levels above this crate's manifest directory.
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

/// The gauntlet's own crate root, excluded from the product scan: the
/// harness's own probes use the design vocabulary.
fn excluded_crate_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Exact-token (case-insensitive) hits for `token` over every product
/// crate's `src/**/*.rs` — the gauntlet crate itself excluded. Bounded
/// like task_48.
fn scan_workspace(root: &Path, token: &str) -> Result<Vec<String>, DriverError> {
    let excluded = excluded_crate_root();
    let wanted = token.to_lowercase();
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
            if path.is_dir() {
                if path != excluded {
                    stack.push(path);
                }
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
                    return Err(probe_error(
                        "source scan",
                        format!("{} exceeds {SOURCE_BYTES_MAX} bytes", path.display()),
                    ));
                }
                let text = String::from_utf8_lossy(&bytes).to_lowercase();
                // Tokenize on non-alphanumeric boundaries (keep `_`, `/`,
                // `-` inside tokens so `no_network` matches whole).
                let mut token_start: Option<usize> = None;
                let mut found = false;
                for (idx, ch) in text.char_indices() {
                    let is_token = ch.is_alphanumeric() || ch == '_' || ch == '/' || ch == '-';
                    if is_token {
                        if token_start.is_none() {
                            token_start = Some(idx);
                        }
                    } else if let Some(start) = token_start.take()
                        && text[start..idx] == wanted
                    {
                        found = true;
                        break;
                    }
                }
                if !found
                    && let Some(start) = token_start
                    && text[start..] == wanted
                {
                    found = true;
                }
                if found {
                    hits.push(path.display().to_string());
                }
            }
        }
    }
    Ok(hits)
}

/// Assert an exact-token scan finds zero hits; fail closed (premise
/// changed) when the absence finding is refuted.
fn assert_absent(
    case: &'static str,
    root: &Path,
    tokens: &[&str],
) -> Result<CaseReport, DriverError> {
    let mut evidence = Vec::new();
    let mut total_hits = 0usize;
    for token in tokens {
        let hits = scan_workspace(root, token)?;
        evidence.push(format!(
            "exact-token scan for '{token}' over crates/*/src/**/*.rs: {} hit(s)",
            hits.len()
        ));
        for hit in &hits {
            evidence.push(format!("  unexpected hit: {hit}"));
        }
        total_hits += hits.len();
    }
    if total_hits != 0 {
        return Ok(CaseReport::fail(
            case,
            format!("{total_hits} hit(s) — the absence finding is refuted (premise changed)"),
            evidence,
        ));
    }
    Ok(CaseReport::pass(
        case,
        serde_json::json!({"hits": 0}),
        evidence,
    ))
}

/// Read one product-crate source file, bounded.
fn read_product_source(root: &Path, relative: &str) -> Result<String, DriverError> {
    let path = root.join(relative);
    let bytes = std::fs::read(&path)
        .map_err(|e| fixture_error("source read", format!("{}: {e}", path.display())))?;
    if bytes.len() > SOURCE_BYTES_MAX {
        return Err(probe_error(
            "source read",
            format!("{} exceeds {SOURCE_BYTES_MAX} bytes", path.display()),
        ));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// V1: `Evaluator::prepare` — "Prepares the isolated workspace and
/// snapshots" — is a skeleton: its body only advances the stage order.
/// No workspace is created, no snapshot taken.
fn case_prepare_is_skeleton() -> Result<CaseReport, DriverError> {
    const CASE: &str = "prepare_is_skeleton";
    let root = workspace_root()?;
    let source = read_product_source(&root, "crates/phlow-experiment/src/evaluator.rs")?;
    let mut evidence = Vec::new();
    // The prepare method's doc comment sits immediately above the
    // signature; the body runs from the signature to the next method's
    // doc comment. The body itself must contain no workspace/snapshot
    // logic; the doc comment must declare the skeleton.
    let marker = "pub fn prepare(&mut self) -> Result<(), ExperimentError>";
    let sig_start = source
        .find(marker)
        .ok_or_else(|| probe_error(CASE, "Evaluator::prepare not found in evaluator.rs"))?;
    let doc_marker = "/// Prepares the isolated workspace and snapshots.";
    let doc_start = source[..sig_start].rfind(doc_marker).unwrap_or(sig_start);
    let after_sig = &source[sig_start + marker.len()..];
    let body_end = after_sig
        .find("/// Executes the bounded task")
        .map(|i| sig_start + marker.len() + i)
        .unwrap_or(source.len());
    let doc = &source[doc_start..sig_start];
    let body = &source[sig_start..body_end];
    evidence.push(format!("prepare() doc: {}", doc.trim()));
    evidence.push(format!(
        "prepare() body ({} chars): {}",
        body.len(),
        body.trim()
    ));
    let creates_workspace = body.contains("create_dir")
        || body.contains("workspace")
        || body.contains("snapshot")
        || body.contains("tempdir")
        || body.contains("TempDir");
    if creates_workspace {
        return Ok(CaseReport::fail(
            CASE,
            "prepare() contains workspace/snapshot logic — the skeleton finding is refuted \
             (premise changed)"
                .to_string(),
            evidence,
        ));
    }
    if !doc.contains("Skeleton: order only") {
        return Ok(CaseReport::fail(
            CASE,
            "prepare() is neither a documented skeleton nor a real implementation".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "prepare() is the documented skeleton: `self.advance(EvalStage::Prepare)` — \
         order enforcement only, no isolated workspace, no snapshots"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"workspace_created": false}),
        evidence,
    ))
}

/// V2: no network-isolation vocabulary exists in any product crate —
/// there is no `unshare`, no network namespace, no firewall rule, no
/// offline-sandbox flag for candidate evaluation.
fn case_no_network_isolation() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_network_isolation";
    let root = workspace_root()?;
    let mut report = assert_absent(
        CASE,
        &root,
        &[
            "unshare",
            "network_isolation",
            "no_network",
            "netns",
            "sandbox_network",
        ],
    )?;
    report.evidence.push(
        "no product crate isolates candidate evaluation from the network: \
         a network-dependent proposal's tests would run with full network \
         access, and the design's 'no network' assertion has nothing to \
         assert against"
            .to_string(),
    );
    Ok(report)
}

/// A1: no write-containment vocabulary exists in any product crate —
/// no chroot, no read-only bind mount, no write-allowlist for the
/// candidate workspace. A proposal writing outside its dir during
/// validation would not be denied by any sandbox layer.
fn case_no_write_containment() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_write_containment";
    let root = workspace_root()?;
    let mut report = assert_absent(
        CASE,
        &root,
        &[
            "chroot",
            "write_containment",
            "readonly_mount",
            "pivot_root",
            "sandbox_dir",
        ],
    )?;
    report.evidence.push(
        "no product crate contains candidate-evaluation writes: an \
         escaping proposal's writes are not denied by any sandbox, so \
         the design's 'policy violation, not test failure' distinction \
         has no mechanism behind it"
            .to_string(),
    );
    Ok(report)
}

/// A2: the isolation is advertised but unimplemented. `Lifecycle::Isolated`
/// ("Candidate workspace created in isolation") exists as a state, and
/// `EvalStage::Prepare` is named "Prepare the isolated workspace and
/// immutable snapshots" — but the transition into `Isolated` performs no
/// isolation, and `prepare()` is the documented skeleton. Names and docs
/// promise a sandbox; the code does not build one.
fn case_isolation_advertised_but_unimplemented() -> Result<CaseReport, DriverError> {
    const CASE: &str = "isolation_advertised_but_unimplemented";
    let root = workspace_root()?;
    let promotion = read_product_source(&root, "crates/phlow-experiment/src/promotion.rs")?;
    let mut evidence = Vec::new();
    if !promotion.contains("Candidate workspace created in isolation") {
        return Ok(CaseReport::fail(
            CASE,
            "Lifecycle::Isolated doc changed — the advertised-isolation finding is refuted \
             (premise changed)"
                .to_string(),
            evidence,
        ));
    }
    evidence.push(
        "Lifecycle::Isolated is documented as 'Candidate workspace created \
         in isolation' (promotion.rs)"
            .to_string(),
    );
    // The only transition into Isolated is a state change: no isolation
    // primitive is invoked. Confirm no sandbox module backs it.
    let hits = scan_workspace(&root, "isolated_workspace")?;
    evidence.push(format!(
        "exact-token scan for 'isolated_workspace': {} hit(s)",
        hits.len()
    ));
    if !hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            "an isolated_workspace implementation appeared — premise changed".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "the state name and stage docs advertise isolation; no code \
         implements it — the sandbox is a documented skeleton, honestly \
         labeled as such in the source ('Skeleton: order only')"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"sandbox_implemented": false}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, DriverError> {
    match case {
        "prepare_is_skeleton" => case_prepare_is_skeleton(),
        "no_network_isolation" => case_no_network_isolation(),
        "no_write_containment" => case_no_write_containment(),
        "isolation_advertised_but_unimplemented" => case_isolation_advertised_but_unimplemented(),
        _ => Err(fixture_error("case", format!("unknown case '{case}'"))),
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
        "seam: DOCUMENTED SKELETON — phlow-experiment's Evaluator::prepare \
         ('Prepares the isolated workspace and snapshots') is order-only; \
         Lifecycle::Isolated names isolation no code implements"
            .to_string(),
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
        "finding: the pre-apply validation sandbox is a documented \
         skeleton — no network isolation, no write containment, no escape \
         policy; gate results cannot come 'from the sandbox' because \
         there is no sandbox"
            .to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: the improvement sandbox is a documented skeleton, not an isolation \
         mechanism. Evaluator::prepare() ('Prepares the isolated workspace and snapshots') is \
         `self.advance(EvalStage::Prepare)` — order enforcement only, honestly labeled 'Skeleton: \
         order only'. Bounded exact-token scans over every product crate's src/**/*.rs (gauntlet \
         crate excluded per the harness-probe principle) find zero hits for network-isolation \
         vocabulary (unshare, network_isolation, no_network, netns, sandbox_network) and zero \
         hits for write-containment vocabulary (chroot, write_containment, readonly_mount, \
         pivot_root, sandbox_dir). Lifecycle::Isolated advertises 'Candidate workspace created \
         in isolation', but the transition performs no isolation and no isolated_workspace \
         implementation exists. A network-dependent proposal's tests would run with full \
         network; an escaping proposal's writes would not be denied — the design's \
         verified-isolation assertions have nothing to assert against. Product decision banked \
         for Matt: whether phlow-experiment should gain a real pre-apply validation sandbox \
         (network-isolated, write-contained, escape attempts typed as policy violations) is \
         not implemented on gauntlet authority."
            .to_string(),
        evidence,
    })
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
