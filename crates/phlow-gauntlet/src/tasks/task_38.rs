//! task-38: ReDoS guard (rust).
//!
//! Recon probe: the design asks for a bounded pattern-evaluation seam —
//! some regex/pattern evaluation on untrusted input (tool-arg
//! validation, log scanning, redaction patterns from task-36) — with
//! worst-case evaluation time provably bounded (timeout with a typed
//! error, or a linear-time engine), and untrusted patterns never
//! reaching the evaluator unchecked (allowlist or complexity check at
//! load).
//!
//! Honest result: the seam is ABSENT. No phlow crate evaluates patterns
//! on untrusted input. The runtime evidence is twofold, both gathered
//! at probe time from the working tree (the task source):
//!
//! 1. Dependency graph: a parse of the workspace `Cargo.lock` shows no
//!    workspace member depends on `regex` or `fancy-regex`. The only
//!    regex crates in the graph are `regex`/`fancy-regex` pulled by
//!    `termwiz` (via `ratatui-termwiz` ← `ratatui`, the TUI backend's
//!    terminal-escape parsing) — registry packages, not phlow code,
//!    unreachable from phlow's input paths.
//! 2. Source scan: a walk over every `crates/*/src/**/*.rs` finds zero
//!    pattern-evaluation API usages (the constructor, the match
//!    predicate, the captures accessor, crate imports — the tokens are
//!    assembled at runtime so the probe cannot match its own source).
//!
//! The deliberate posture is even documented in-tree: phlow-config's
//! check-name validation is "Hand-rolled: no regex crate needed"
//! (`crates/phlow-config/src/load.rs`).
//!
//! Four cases, all against the real working tree (no mocks): two
//! validation, two adversarial. Each case documents the real state;
//! the task-level verdict is `fail` at `"seam"` because the design's
//! pass criteria (a bounded evaluator; untrusted patterns checked at
//! load) need an evaluator to attach to, and none exists. The
//! design's adversarial weapon — `(a+)+$` against `aaaa…a!` — has no
//! target: there is nothing to feed it to.
//!
//! Banked for Matt (product decision, NOT auto-implemented): if phlow
//! ever adds pattern-based tool-arg validation or log scanning, the
//! design's bounded-evaluation requirement applies then — a linear-time
//! engine or a timeout with a typed error, plus an allowlist/complexity
//! check for patterns arriving from untrusted config.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use std::fmt;
use std::path::{Path, PathBuf};

/// Task id.
pub const ID: &str = "task-38";
/// Human-readable name.
pub const NAME: &str = "ReDoS guard";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "workspace_members_pull_no_regex",
    "no_regex_api_usage_in_sources",
    "catastrophic_pattern_has_no_evaluator",
    "untrusted_config_patterns_have_no_sink",
];

/// Largest `Cargo.lock` the probe will read, in bytes.
const LOCK_BYTES_MAX: usize = 8_388_608;
/// Largest Rust source file the probe will scan, in bytes.
const SOURCE_BYTES_MAX: usize = 1_048_576;
/// Most source files the probe will scan before stopping.
const SOURCE_FILES_MAX: usize = 50_000;
/// Crate names that would constitute a pattern-evaluation dependency.
const PATTERN_CRATES: [&str; 2] = ["regex", "fancy-regex"];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-38 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture (workspace root, lock file, source tree) was unusable.
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
                write!(f, "task-38: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-38: cannot probe {what}: {detail}")
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

/// One `[[package]]` section of `Cargo.lock`.
struct LockPackage {
    /// Package name.
    name: String,
    /// True when the package is a workspace member (no `source` field).
    is_workspace_member: bool,
    /// Dependency package names (version qualifiers stripped).
    deps: Vec<String>,
}

/// Parse the workspace `Cargo.lock` into package sections. Hand-rolled:
/// the format is line-oriented `[[package]]` stanzas; the probe needs
/// only name, source presence, and the dependency name list.
fn parse_lock(root: &Path) -> Result<Vec<LockPackage>, DriverError> {
    let lock_path = root.join("Cargo.lock");
    let text = std::fs::read_to_string(&lock_path).map_err(|e| fixture_error("Cargo.lock", e))?;
    if text.len() > LOCK_BYTES_MAX {
        return Err(fixture_error(
            "Cargo.lock",
            format!("lock file exceeds {LOCK_BYTES_MAX} bytes"),
        ));
    }
    let mut packages = Vec::new();
    let mut current: Option<LockPackage> = None;
    let mut in_deps = false;
    for line in text.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            if let Some(pkg) = current.take() {
                packages.push(pkg);
            }
            current = Some(LockPackage {
                name: String::new(),
                is_workspace_member: true,
                deps: Vec::new(),
            });
            in_deps = false;
        } else if let Some(pkg) = current.as_mut() {
            if let Some(name) = line.strip_prefix("name = ") {
                pkg.name = name.trim_matches('"').to_string();
                in_deps = false;
            } else if line.starts_with("source = ") {
                pkg.is_workspace_member = false;
                in_deps = false;
            } else if line.starts_with("dependencies = [") {
                in_deps = !line.ends_with(']');
            } else if in_deps {
                if line == "]" {
                    in_deps = false;
                } else if let Some(dep) = line.trim_matches(['"', ',']).split_whitespace().next() {
                    // Entries look like `"serde_json"` or `"sha2 0.10.9"`;
                    // the package name is the first token.
                    pkg.deps.push(dep.trim_matches('"').to_string());
                }
            }
        }
    }
    if let Some(pkg) = current.take() {
        packages.push(pkg);
    }
    if packages.is_empty() {
        return Err(probe_error("Cargo.lock", "no [[package]] sections parsed"));
    }
    Ok(packages)
}

/// Pattern-evaluation API usage tokens, assembled at runtime from halves
/// so the probe's own source never contains the literal tokens it scans
/// for. The scan must be able to cover this crate too.
fn usage_tokens() -> Vec<String> {
    const HALVES: [(&str, &str); 7] = [
        ("Re", "gex::new"),
        (".is", "_match("),
        ("use re", "gex::"),
        ("fancy", "_regex"),
        ("regex", "_automata"),
        ("regex", "_syntax"),
        (".cap", "tures("),
    ];
    HALVES.iter().map(|(a, b)| format!("{a}{b}")).collect()
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

/// V1: no workspace member depends on a pattern-evaluation crate. The
/// lock parse names the actual regex consumers: `ratatui-termwiz` ←
/// `ratatui` (the TUI backend) → `termwiz` → `fancy-regex` → `regex` —
/// registry packages doing terminal-escape parsing, unreachable from
/// phlow's input paths.
fn case_workspace_members_pull_no_regex() -> Result<CaseReport, DriverError> {
    const CASE: &str = "workspace_members_pull_no_regex";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let packages = parse_lock(&root)?;
    let members: Vec<&LockPackage> = packages.iter().filter(|p| p.is_workspace_member).collect();
    evidence.push(format!(
        "Cargo.lock parses: {} packages, {} workspace members",
        packages.len(),
        members.len()
    ));
    let mut offenders = Vec::new();
    for member in &members {
        for dep in &member.deps {
            if PATTERN_CRATES.contains(&dep.as_str()) {
                offenders.push(format!("{} -> {dep}", member.name));
            }
        }
    }
    if !offenders.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "workspace members pull pattern crates: {}",
                offenders.join(", ")
            ),
            evidence,
        ));
    }
    // Name the real consumers, so the negative is sourced, not asserted.
    let mut consumers = Vec::new();
    for pkg in &packages {
        if !pkg.is_workspace_member && PATTERN_CRATES.contains(&pkg.name.as_str()) {
            let mut pulled_by: Vec<&str> = packages
                .iter()
                .filter(|other| other.deps.iter().any(|d| d == &pkg.name))
                .map(|other| other.name.as_str())
                .collect();
            pulled_by.sort_unstable();
            consumers.push(format!("{} (pulled by {})", pkg.name, pulled_by.join(", ")));
        }
    }
    consumers.sort();
    evidence.push(format!(
        "registry-side pattern-crate consumers: {}",
        consumers.join("; ")
    ));
    evidence.push(
        "the only regex in the graph is termwiz's internal terminal-escape parsing (TUI backend); no phlow input path reaches it"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({
            "workspace_members": members.len(),
            "members_pulling_pattern_crates": 0,
            "registry_pattern_consumers": consumers,
        }),
        evidence,
    ))
}

/// V2: no pattern-evaluation API usage in any crate's sources. The
/// scan covers every `crates/*/src/**/*.rs` including this crate —
/// the tokens are assembled at runtime so the probe cannot match its
/// own documentation strings.
fn case_no_regex_api_usage_in_sources() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_regex_api_usage_in_sources";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let tokens = usage_tokens();
    let hits = scan_sources(&root, &tokens)?;
    evidence.push(format!(
        "scanned {} usage tokens over crates/*/src; hits: {}",
        tokens.len(),
        hits.len()
    ));
    if !hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!("pattern-evaluation API usage found: {}", hits.join("; ")),
            evidence,
        ));
    }
    evidence.push(
        "zero hits: no phlow source constructs, imports, or calls a pattern evaluator".to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"usage_tokens": tokens.len(), "usage_hits": 0}),
        evidence,
    ))
}

/// A1: the design's adversarial weapon — `(a+)+$` against a long run of
/// `a` ending in `!` — has no target. With no evaluator in the graph
/// (V1) and no evaluator call in any source (V2), there is nothing to
/// feed the catastrophic input to. The case passes as a probe: it
/// documents the weapon and the missing target, rather than claiming a
/// timing bound that was never measured against a real engine.
fn case_catastrophic_pattern_has_no_evaluator() -> Result<CaseReport, DriverError> {
    const CASE: &str = "catastrophic_pattern_has_no_evaluator";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let tokens = usage_tokens();
    let hits = scan_sources(&root, &tokens)?;
    if !hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "an evaluator exists ({}), so the catastrophic input has a target after all: {}",
                hits.len(),
                hits.join("; ")
            ),
            evidence,
        ));
    }
    evidence.push(
        "adversarial input (a+)+$ vs 'a'*n + '!' would need an evaluator; the V1/V2 scans show none exists in phlow code"
            .to_string(),
    );
    evidence.push(
        "no worst-case timing was measured because there is no engine to time — a claimed bound would be invented, not sourced"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"evaluators_found": 0}),
        evidence,
    ))
}

/// A2: untrusted patterns from config have no sink. phlow-config — the
/// crate that loads operator configuration — contains no
/// pattern-evaluation API usage, and its own source documents the
/// deliberate posture: check-name validation is "Hand-rolled: no regex
/// crate needed" (`crates/phlow-config/src/load.rs`). A pattern
/// arriving from config could not reach an evaluator because there is
/// no evaluator and no pattern-accepting config field feeds one.
fn case_untrusted_config_patterns_have_no_sink() -> Result<CaseReport, DriverError> {
    const CASE: &str = "untrusted_config_patterns_have_no_sink";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let tokens = usage_tokens();
    // The walk is rooted at the workspace; keep only phlow-config hits.
    let hits: Vec<String> = scan_sources(&root, &tokens)?
        .into_iter()
        .filter(|h| h.contains("phlow-config"))
        .collect();
    if !hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "pattern-evaluation API usage in phlow-config: {}",
                hits.join("; ")
            ),
            evidence,
        ));
    }
    evidence.push("phlow-config sources: zero pattern-evaluation API usages".to_string());
    let load_rs = std::fs::read_to_string(
        root.join("crates")
            .join("phlow-config")
            .join("src")
            .join("load.rs"),
    )
    .map_err(|e| fixture_error("phlow-config load.rs", e))?;
    if load_rs.contains("no regex crate needed") {
        evidence.push(
            "phlow-config/src/load.rs documents the deliberate posture: check-name validation is hand-rolled, 'no regex crate needed'"
                .to_string(),
        );
    } else {
        evidence.push(
            "note: the 'no regex crate needed' comment was not found in load.rs — the zero-usage scan above is the operative evidence"
                .to_string(),
        );
    }
    evidence.push(
        "an untrusted pattern from config has no evaluator to reach: no sink exists, so no allowlist/complexity check at load is needed — and none is claimed"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"phlow_config_usage_hits": 0}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "workspace_members_pull_no_regex" => case_workspace_members_pull_no_regex(),
        "no_regex_api_usage_in_sources" => case_no_regex_api_usage_in_sources(),
        "catastrophic_pattern_has_no_evaluator" => case_catastrophic_pattern_has_no_evaluator(),
        "untrusted_config_patterns_have_no_sink" => case_untrusted_config_patterns_have_no_sink(),
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
        "recon: workspace Cargo.lock — no member depends on regex/fancy-regex; the only pattern crates in the graph are registry-side (termwiz via ratatui-termwiz <- ratatui), doing terminal-escape parsing unreachable from phlow input paths".to_string(),
        "recon: source scan over every crates/*/src/**/*.rs — zero pattern-evaluation API usages (tokens assembled at runtime so the probe cannot self-match)".to_string(),
        "recon: phlow-config documents the deliberate posture — 'Hand-rolled: no regex crate needed' (crates/phlow-config/src/load.rs)".to_string(),
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
        "finding: no pattern evaluation on untrusted input exists in phlow — the ReDoS dimension has no seam to guard".to_string(),
    );
    evidence.push(
        "finding: the design's adversarial weapon ((a+)+$ vs a...a!) has no target; no worst-case timing was measured because there is no engine to time".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no regex/pattern evaluation on untrusted input exists in any phlow crate — the dependency graph (Cargo.lock: no member pulls regex/fancy-regex; only termwiz's internal terminal-escape parsing, unreachable from input paths) and a runtime source scan (zero pattern-evaluation API usages across every crates/*/src) agree. The design's pass criteria (bounded worst-case evaluation; untrusted patterns checked at load) need an evaluator to attach to, and there is none.".to_string(),
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
