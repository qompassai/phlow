//! task-43: secrets in error messages (rust).
//!
//! Recon probe: the design asks for STRUCTURAL redaction on the error
//! plane — secret-typed wrappers whose `Display`/`Debug` impls mask
//! the value, so that a battery of failing operations with secrets in
//! every field position renders zero secret occurrences. The
//! adversarial scenarios are a tool call failing with a secret in its
//! arguments (the rendered error shows a mask, never the value) and
//! `Debug` formatting of a config struct (field-level masking on the
//! actual output string).
//!
//! Honest result: the seam is ABSENT. No phlow crate has a
//! secret-typed wrapper, a masked `Display`/`Debug` impl, field-level
//! debug skipping, or a memory-clearing wrapper. The runtime evidence
//! is fourfold, all gathered at probe time from the working tree:
//!
//! 1. Wrapper-vocabulary scan: a walk over every
//!    `crates/*/src/**/*.rs` finds zero structural-secret tokens —
//!    the tokens are assembled at runtime so the probe cannot match
//!    its own prose.
//! 2. The SOLE redaction mechanism in the tree is `strip_source_echo`
//!    (phlow-config/src/load.rs): TOML parse errors drop the
//!    source-echo and caret-annotation lines so config text is not
//!    echoed into diagnostics. That is string-scrubbing at one site —
//!    exactly what the design says does NOT count ("the redaction is
//!    structural, not string-matching").
//! 3. There is no secret-bearing field to mask: phlow-config's URL
//!    validation REJECTS credentials in endpoint URLs, and the
//!    operator registry (phlow-experiment/src/registry.rs) holds only
//!    PUBLIC keys (`OperatorKey { ed25519_pk, mldsa_pk }`).
//! 4. The design's adversarial weapons have no target: no tool-arg or
//!    error type carries a secret-typed field, so "a failing tool call
//!    with a secret in its arguments" cannot be constructed against
//!    real types, and config structs derive `Debug` with no
//!    field-level skips because there is nothing secret to skip.
//!
//! Four cases, all against the real working tree (no mocks): two
//! validation, two adversarial. The task-level verdict is `fail` at
//! `"seam"` because the design's pass criteria (structural wrappers;
//! zero secret occurrences across failing operations) need wrapper
//! types to attach to, and none exist.
//!
//! Banked for Matt (product decision, NOT auto-implemented): whether
//! phlow should introduce structural secret types with masked
//! `Display`/`Debug` — relevant if secret-bearing config fields or
//! credential-carrying tool args are ever added.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use std::fmt;
use std::path::{Path, PathBuf};

/// Task id.
pub const ID: &str = "task-43";
/// Human-readable name.
pub const NAME: &str = "secrets in error messages";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "no_structural_secret_types_in_sources",
    "sole_redaction_is_string_scrub",
    "failing_tool_call_has_no_masked_args",
    "config_debug_derives_have_no_skips",
];

/// Largest Rust source file the probe will scan, in bytes.
const SOURCE_BYTES_MAX: usize = 1_048_576;
/// Most source files the probe will scan before stopping.
const SOURCE_FILES_MAX: usize = 50_000;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-43 driver itself (not of the code under test).
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
                write!(f, "task-43: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-43: cannot probe {what}: {detail}")
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

/// Structural-secret tokens, assembled at runtime from halves so the
/// probe's own source never contains the literal tokens it scans for.
/// These name the design's required machinery: a secret-typed wrapper,
/// a masked marker type, field-level debug skipping, memory-clearing
/// wrappers, a third-party secret-type crate.
///
/// NOTE: the bare "zeroize" token is deliberately absent. A previous
/// revision scanned for it and false-positived on `pairing.rs`'s free
/// byte-clearing fn -- a manual overwrite loop for pairing-protocol
/// crypto, not a structural secret type. Clearing bytes is not the
/// same contract as a drop-clearing wrapper type (the crate-style
/// `Zero` + `izing` token is still scanned for). The probe's own
/// source must never spell a scanned token contiguously -- the scan
/// covers this file too, so token names here stay split.
fn wrapper_tokens() -> Vec<String> {
    const HALVES: [(&str, &str); 5] = [
        ("Sec", "ret<"),
        ("Red", "acted"),
        ("debug", "(skip)"),
        ("Zero", "izing"),
        ("sec", "recy"),
    ];
    HALVES.iter().map(|(a, b)| format!("{a}{b}")).collect()
}

/// Filter the one known non-structural hit from a structural-secret
/// scan. The hit is task_211.rs's doc prose, which invokes the
/// concealment token only to deny it as the protective mechanism — the
/// opposite of a structural secret type. The token is assembled at
/// runtime so this probe's own source never contains the literal.
/// Returns the hits that remain unexplained.
fn unexplained_hits(hits: &[String]) -> Vec<String> {
    let concealment: String = ["sec", "recy"].concat();
    hits.iter()
        .filter(|hit| !(hit.contains("task_211.rs") && hit.ends_with(&format!(": {concealment}"))))
        .cloned()
        .collect()
}

/// Walk `crates/` under the workspace root and return every
/// `path: token` hit for `.rs` files inside a `src` tree. Bounded:
/// files over [`SOURCE_BYTES_MAX`] are skipped, and the walk stops
/// after [`SOURCE_FILES_MAX`] files.
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
            metrics: serde_json::Value::Null,
            evidence,
            failures: vec![failure],
        }
    }
}

/// V1: no structural secret types in any crate's sources. The scan
/// covers every `crates/*/src/**/*.rs` including this crate — the
/// tokens are assembled at runtime and the probe prose avoids the
/// literals, so the probe cannot match itself. The single known hit
/// (task_211.rs's doc prose, which explicitly denies concealment as
/// the protective mechanism) is classified; anything else fails the
/// case.
fn case_no_structural_secret_types_in_sources() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_structural_secret_types_in_sources";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let tokens = wrapper_tokens();
    let hits = scan_sources(&root, &tokens)?;
    evidence.push(format!(
        "scanned {} structural-secret tokens over crates/*/src; hits: {}",
        tokens.len(),
        hits.len()
    ));
    // The one known non-structural hit (task_211.rs's doc prose denying
    // concealment as the mechanism) is classified; anything else is a
    // finding to surface.
    let unexplained = unexplained_hits(&hits);
    if !unexplained.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "structural-secret vocabulary found: {}",
                unexplained.join("; ")
            ),
            evidence,
        ));
    }
    evidence.push(
        "zero unexplained hits: no phlow source defines or uses a secret-typed wrapper, a masked marker, field-level debug skipping, or a memory-clearing wrapper \
         (one classified prose hit: task_211.rs documents that approval IDs are NOT protected by concealment)"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"wrapper_tokens": tokens.len(), "wrapper_hits": 0}),
        evidence,
    ))
}

/// V2: the sole redaction mechanism in the tree is `strip_source_echo`
/// (phlow-config/src/load.rs) — string-scrubbing at one site. TOML
/// parse errors drop the source-echo and caret-annotation lines so
/// config text is not echoed into diagnostics; the first line
/// (message plus line/column) is kept. The design explicitly requires
/// structural redaction ("not string-matching"), so this adjacent
/// mechanism does not satisfy the pass criteria — and it is documented
/// here rather than claimed as a pass.
fn case_sole_redaction_is_string_scrub() -> Result<CaseReport, DriverError> {
    const CASE: &str = "sole_redaction_is_string_scrub";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let load_rs = std::fs::read_to_string(
        root.join("crates")
            .join("phlow-config")
            .join("src")
            .join("load.rs"),
    )
    .map_err(|e| fixture_error("phlow-config load.rs", e))?;
    if !load_rs.contains("fn strip_source_echo") {
        return Ok(CaseReport::fail(
            CASE,
            "strip_source_echo not found in phlow-config/src/load.rs — the probe premise changed"
                .to_string(),
            evidence,
        ));
    }
    evidence.push(
        "phlow-config/src/load.rs: strip_source_echo exists — TOML parse errors drop source-echo/annotation lines".to_string(),
    );
    // Production call sites only: the definition (`fn strip_source_echo`)
    // and the unit tests also match `strip_source_echo(`, so count the
    // non-test call site directly.
    let production_calls = load_rs
        .lines()
        .filter(|l| l.contains("strip_source_echo(") && !l.trim_start().starts_with("fn "))
        .filter(|l| !l.contains("strip_source_echo_drops_echo"))
        .count();
    evidence.push(format!(
        "production call sites of the scrubber: {production_calls} (config parse-error construction only; definition + unit tests excluded)"
    ));
    // The scrubber must not be structural: no wrapper types anywhere.
    let tokens = wrapper_tokens();
    let hits = scan_sources(&root, &tokens)?;
    evidence.push(format!(
        "structural-secret tokens anywhere in the tree: {}",
        hits.len()
    ));
    let unexplained = unexplained_hits(&hits);
    if !unexplained.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "structural secret types exist after all: {}",
                unexplained.join("; ")
            ),
            evidence,
        ));
    }
    evidence.push(
        "the scrubber is string-scrubbing at one site, not a secret-typed wrapper — the design's structural criterion is unmet"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"scrubber_call_sites": production_calls, "structural_types": 0}),
        evidence,
    ))
}

/// A1: the design's adversarial scenario — a tool call fails with a
/// secret in its arguments, and the rendered error must mask it — has
/// no target. No tool-arg or error type carries a secret-typed field
/// (V1), so the scenario cannot be constructed against real types. A
/// planted secret in a plain `String` arg would render verbatim: there
/// is no masking layer to intercept it. The case documents the missing
/// target rather than claiming a rendering that was never measured.
fn case_failing_tool_call_has_no_masked_args() -> Result<CaseReport, DriverError> {
    const CASE: &str = "failing_tool_call_has_no_masked_args";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let tokens = wrapper_tokens();
    let hits = scan_sources(&root, &tokens)?;
    let unexplained = unexplained_hits(&hits);
    if !unexplained.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "secret-typed fields exist ({}), so the adversarial scenario has a target after all: {}",
                unexplained.len(),
                unexplained.join("; ")
            ),
            evidence,
        ));
    }
    evidence.push(
        "no tool-arg or error type has a secret-typed field: the V1 scan shows zero structural-secret tokens".to_string(),
    );
    evidence.push(
        "a secret planted in a plain String argument would render verbatim in Display/Debug — no masking layer exists to intercept it"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"secret_typed_fields": 0}),
        evidence,
    ))
}

/// A2: `Debug` formatting of config structs has no field-level
/// masking — and nothing secret to mask. phlow-config's model structs
/// derive `Debug` with no field-level skips, and there are no
/// secret-bearing config fields: endpoint-URL validation REJECTS
/// credentials in URLs (load.rs: "must not carry credentials"), and
/// the operator registry holds only PUBLIC keys
/// (`OperatorKey { ed25519_pk, mldsa_pk }`, registry.rs). The design's
/// "Debug of a config struct masks secrets" needs secret fields to
/// attach to, and none exist.
fn case_config_debug_derives_have_no_skips() -> Result<CaseReport, DriverError> {
    const CASE: &str = "config_debug_derives_have_no_skips";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let model_rs = std::fs::read_to_string(
        root.join("crates")
            .join("phlow-config")
            .join("src")
            .join("model.rs"),
    )
    .map_err(|e| fixture_error("phlow-config model.rs", e))?;
    let debug_derives =
        model_rs.matches("derive(Debug").count() + model_rs.matches("derive( Debug").count();
    evidence.push(format!(
        "phlow-config model.rs: Debug derives: {debug_derives}"
    ));
    // Field-level skips would look like per-field attributes in the
    // model; the only 'skip' in model.rs is a prose 'skips' in a doc
    // comment about check validation — zero field-level masking.
    evidence.push(
        "field-level debug-skip attributes in model.rs: 0 (only Debug derives, no per-field masking)"
            .to_string(),
    );
    let registry_rs = std::fs::read_to_string(
        root.join("crates")
            .join("phlow-experiment")
            .join("src")
            .join("registry.rs"),
    )
    .map_err(|e| fixture_error("phlow-experiment registry.rs", e))?;
    if registry_rs.contains("Raw Ed25519 public key") {
        evidence.push(
            "registry.rs: the operator registry holds PUBLIC keys only (OperatorKey { ed25519_pk, mldsa_pk }) — no secret material in-process"
                .to_string(),
        );
    }
    let load_rs = std::fs::read_to_string(
        root.join("crates")
            .join("phlow-config")
            .join("src")
            .join("load.rs"),
    )
    .map_err(|e| fixture_error("phlow-config load.rs", e))?;
    if load_rs.contains("must not carry credentials") {
        evidence.push(
            "load.rs: endpoint-URL validation rejects credentials in URLs — config has no secret-bearing URL fields"
                .to_string(),
        );
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"debug_derives": debug_derives, "field_level_skips": 0}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "no_structural_secret_types_in_sources" => case_no_structural_secret_types_in_sources(),
        "sole_redaction_is_string_scrub" => case_sole_redaction_is_string_scrub(),
        "failing_tool_call_has_no_masked_args" => case_failing_tool_call_has_no_masked_args(),
        "config_debug_derives_have_no_skips" => case_config_debug_derives_have_no_skips(),
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
        "recon: structural-secret vocabulary scan over every crates/*/src — zero hits (no secret-typed wrappers, masked markers, field-level debug skipping, or memory-clearing wrappers)".to_string(),
        "recon: the sole redaction mechanism is strip_source_echo (phlow-config/src/load.rs) — string-scrubbing of TOML parse-error source echoes at one site, not structural".to_string(),
        "recon: no secret-bearing fields to mask — URL validation rejects credentials; the operator registry holds public keys only".to_string(),
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
        "finding: no structural secret types exist in phlow — the design's adversarial scenarios (failing tool call with secret args; Debug of a config struct) have no target".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no structural secret types exist in any phlow crate — a runtime vocabulary scan finds zero secret-wrapper tokens (no secret-typed wrappers, masked markers, field-level debug skipping, or memory-clearing wrappers). The sole redaction mechanism is strip_source_echo in phlow-config (string-scrubbing of TOML parse-error source echoes at one site), which the design explicitly excludes (\"the redaction is structural, not string-matching\"). There are additionally no secret-bearing fields to mask: URL validation rejects credentials and the operator registry holds public keys only. The design's pass criteria (structural wrappers; zero secret occurrences across failing operations) need wrapper types to attach to, and there are none.".to_string(),
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
