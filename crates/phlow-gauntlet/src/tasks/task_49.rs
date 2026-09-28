//! task-49: CPU fairness (rust).
//!
//! Recon probe: the design asks for a task-executor / worker-pool
//! scheduling seam — a mechanism (named: preemption, quanta, priorities)
//! that keeps one runaway task from starving the others, with fairness
//! applying to the runaway's whole child tree. The adversarial scenarios
//! are a task that never yields (the others must still complete within a
//! bounded multiple of solo time) and a runaway that spawns children that
//! also never yield (fairness at the tree level).
//!
//! Honest result: the seam is ABSENT. The evidence is gathered at probe
//! time from the live working tree:
//!
//! 1. Vocabulary scan: a tokenized walk over every `crates/*/src/**/*.rs`
//!    finds zero scheduling-discipline tokens — no `preempt`, no
//!    `timeslice`/`time_slice`, no `fairness`. The one `quantum` hit is
//!    "quantum-proof" (post-quantum cryptography in
//!    phlow-experiment/src/registry.rs), classified and rejected.
//! 2. There is no worker pool to schedule: the pool-vocabulary scan
//!    (`worker_pool`, `thread_pool`, `task_pool`) returns zero, and the
//!    only tokio consumer in the workspace is the msgpack transport's
//!    single current-thread worker (`phlow-runtime/src/transport/msgpack.rs`,
//!    the task-25 channel) — one worker thread, not a scheduled pool; no
//!    fairness question arises there and none is implemented.
//! 3. The experiment `Scheduler` (phlow-experiment) is admission-only
//!    with no consumer (established in tasks 13 and 25): it admits nodes
//!    against limits but executes nothing — there is no task tree to be
//!    fair across. The tree-vocabulary scan (`task_tree`, `child_task`)
//!    returns zero.
//!
//! Four cases, all against the real working tree (no mocks): two
//! validation, two adversarial. The task-level verdict is `fail` at
//! `"seam"` because the design's pass criteria (no task's completion
//! time exceeds Kx its solo time under a runaway; the mechanism named,
//! not emergent) need a scheduler to attach to, and none exists.
//!
//! Banked for Matt (product decision, NOT auto-implemented): whether
//! phlow needs a fair task executor at all — the current execution model
//! is single-threaded workers plus synchronous checks with per-call
//! deadlines (e.g. HOOK_TIMEOUT), so CPU starvation is currently bounded
//! by deadlines, not by scheduling.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use std::fmt;
use std::path::{Path, PathBuf};

/// Task id.
pub const ID: &str = "task-49";
/// Human-readable name.
pub const NAME: &str = "CPU fairness";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "no_scheduling_discipline_tokens",
    "no_worker_pool_to_schedule",
    "runaway_never_yields_has_no_target",
    "runaway_tree_fairness_has_no_target",
];

/// Largest Rust source file the probe will scan, in bytes.
const SOURCE_BYTES_MAX: usize = 1_048_576;
/// Most source files the probe will scan before stopping.
const SOURCE_FILES_MAX: usize = 50_000;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-49 driver itself (not of the code under test).
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
                write!(f, "task-49: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-49: cannot probe {what}: {detail}")
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

/// Walk `crates/` under the workspace root and return `path:line` hits
/// for `.rs` files inside a `src` tree whose alphanumeric-token stream
/// contains `token` (case-insensitive, exact token — not a substring).
/// Skips the probe's own source file by exact path. Bounded: files over
/// [`SOURCE_BYTES_MAX`] are skipped, and the walk stops after
/// [`SOURCE_FILES_MAX`] files.
fn scan_sources(root: &Path, token: &str) -> Result<Vec<String>, DriverError> {
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
                && !path.components().any(|c| c.as_os_str() == "phlow-gauntlet")
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
                for (lineno, line) in text.lines().enumerate() {
                    let found = line
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .any(|tok| tok.eq_ignore_ascii_case(token));
                    if found {
                        hits.push(format!("{}:{}", path.display(), lineno + 1));
                    }
                }
            }
        }
    }
    Ok(hits)
}

/// Scan for several tokens at once, concatenating the hits.
fn scan_tokens(root: &Path, tokens: &[&str]) -> Result<Vec<String>, DriverError> {
    let mut hits = Vec::new();
    for token in tokens {
        hits.extend(scan_sources(root, token)?);
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

/// V1: no scheduling-discipline vocabulary anywhere in the workspace
/// sources. `preempt`, `timeslice`/`time_slice`, and `fairness` return
/// zero hits; the `quantum` hits are classified — every one is the
/// "quantum-proof" cryptography term in
/// `phlow-experiment/src/registry.rs`, not a scheduling quantum.
fn case_no_scheduling_discipline_tokens() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_scheduling_discipline_tokens";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let mut unexplained: Vec<String> = Vec::new();
    for token in ["preempt", "timeslice", "time_slice", "fairness"] {
        let hits = scan_sources(&root, token)?;
        evidence.push(format!("token '{token}': {} hits", hits.len()));
        unexplained.extend(hits);
    }
    let quantum_hits = scan_sources(&root, "quantum")?;
    evidence.push(format!("token 'quantum': {} hits", quantum_hits.len()));
    for hit in &quantum_hits {
        // Classify: read the hit line and require "quantum-proof".
        let (path, lineno) = hit
            .rsplit_once(':')
            .ok_or_else(|| probe_error("hit parse", format!("bad hit '{hit}'")))?;
        let lineno: usize = lineno
            .parse()
            .map_err(|_| probe_error("hit parse", format!("bad lineno in '{hit}'")))?;
        let text = std::fs::read_to_string(path)
            .map_err(|e| fixture_error("hit read", format!("{path}: {e}")))?;
        let line = text.lines().nth(lineno - 1).unwrap_or("");
        let lowered = line.to_lowercase();
        // "quantum" here is the post-quantum-cryptography term
        // (ML-DSA-65 / ML-KEM / PQ KEX), not a scheduling quantum.
        if lowered.contains("quantum-proof")
            || lowered.contains("post-quantum")
            || lowered.contains("ml-dsa")
            || lowered.contains("ml-kem")
        {
            evidence.push(format!("classified (cryptography, not scheduling): {hit}"));
        } else {
            unexplained.push(hit.clone());
        }
    }
    if !unexplained.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "scheduling-discipline vocabulary found: {}",
                unexplained.join("; ")
            ),
            evidence,
        ));
    }
    evidence.push(
        "zero scheduling-discipline tokens: no preemption, no quanta, no fairness machinery is named anywhere"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"discipline_tokens": 5, "unexplained_hits": 0}),
        evidence,
    ))
}

/// V2: there is no worker pool to schedule. The pool-vocabulary scan
/// returns zero, and the only async-task machinery in the workspace is
/// the msgpack transport's single current-thread worker — one thread,
/// not a scheduled pool, so no fairness question arises there and none
/// is implemented.
fn case_no_worker_pool_to_schedule() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_worker_pool_to_schedule";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let pool_hits = scan_tokens(&root, &["worker_pool", "thread_pool", "task_pool"])?;
    evidence.push(format!("pool vocabulary hits: {}", pool_hits.len()));
    for hit in &pool_hits {
        evidence.push(format!("hit: {hit}"));
    }
    if !pool_hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!("worker-pool vocabulary found: {}", pool_hits.join("; ")),
            evidence,
        ));
    }
    let tokio_hits = scan_sources(&root, "tokio")?;
    evidence.push(format!("'tokio' token hits: {}", tokio_hits.len()));
    let mut non_transport = Vec::new();
    for hit in &tokio_hits {
        // A word in prose is not machinery. Only code USE of tokio
        // counts: `use tokio` or a `tokio::` path on the hit line.
        let (path, lineno) = hit
            .rsplit_once(':')
            .ok_or_else(|| probe_error("hit parse", format!("bad hit '{hit}'")))?;
        let lineno: usize = lineno
            .parse()
            .map_err(|_| probe_error("hit parse", format!("bad lineno in '{hit}'")))?;
        let text = std::fs::read_to_string(path)
            .map_err(|e| fixture_error("hit read", format!("{path}: {e}")))?;
        let line = text.lines().nth(lineno - 1).unwrap_or("");
        let is_code_use = line.contains("use tokio") || line.contains("tokio::");
        if !is_code_use {
            evidence.push(format!("classified (prose mention, not machinery): {hit}"));
        } else if hit.contains("transport/msgpack.rs") {
            evidence.push(format!("classified (single transport worker): {hit}"));
        } else {
            non_transport.push(hit.clone());
        }
    }
    if !non_transport.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "async task machinery outside the single transport worker: {}",
                non_transport.join("; ")
            ),
            evidence,
        ));
    }
    evidence.push(
        "the only task machinery is the msgpack transport's single current-thread worker (the task-25 channel) — no pool, no scheduling discipline".to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"pool_hits": 0, "tokio_hits_outside_transport": 0}),
        evidence,
    ))
}

/// A1: the runaway-never-yields weapon has no target. With no
/// preemption, no quanta, and no pool (V1/V2), there is no scheduler to
/// starve and no mechanism whose fairness could be measured. The case
/// passes as a probe: it documents the missing target rather than
/// inventing a timing measurement.
fn case_runaway_never_yields_has_no_target() -> Result<CaseReport, DriverError> {
    const CASE: &str = "runaway_never_yields_has_no_target";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let hits = scan_tokens(
        &root,
        &["preempt", "fairness", "worker_pool", "thread_pool"],
    )?;
    if !hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "scheduling machinery exists after all — the runaway has a target: {}",
                hits.join("; ")
            ),
            evidence,
        ));
    }
    evidence.push(
        "a never-yielding task would need a preemptive scheduler to starve; the V1/V2 scans show no scheduler exists — a Kx-solo-time measurement would be invented, not sourced"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"schedulers_found": 0}),
        evidence,
    ))
}

/// A2: the runaway's child tree has no fairness either — there is no
/// task tree to be fair across. The tree-vocabulary scan returns zero,
/// and the experiment `Scheduler` is admission-only with no consumer
/// (established in tasks 13 and 25): it admits nodes against limits but
/// executes nothing.
fn case_runaway_tree_fairness_has_no_target() -> Result<CaseReport, DriverError> {
    const CASE: &str = "runaway_tree_fairness_has_no_target";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let tree_hits = scan_tokens(&root, &["task_tree", "child_task"])?;
    evidence.push(format!("task-tree vocabulary hits: {}", tree_hits.len()));
    for hit in &tree_hits {
        evidence.push(format!("hit: {hit}"));
    }
    if !tree_hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!("task-tree vocabulary found: {}", tree_hits.join("; ")),
            evidence,
        ));
    }
    evidence.push(
        "no task tree exists, and the experiment Scheduler admits without executing — tree-level fairness has nothing to attach to"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"task_tree_primitives": 0}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "no_scheduling_discipline_tokens" => case_no_scheduling_discipline_tokens(),
        "no_worker_pool_to_schedule" => case_no_worker_pool_to_schedule(),
        "runaway_never_yields_has_no_target" => case_runaway_never_yields_has_no_target(),
        "runaway_tree_fairness_has_no_target" => case_runaway_tree_fairness_has_no_target(),
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
        "recon: tokenized vocabulary scan over every crates/*/src — zero scheduling-discipline tokens (no preempt, no timeslice/time_slice, no fairness; the only 'quantum' hits are the quantum-proof cryptography term, classified and rejected)".to_string(),
        "recon: no worker pool exists — pool vocabulary zero; the only async task machinery is the msgpack transport's single current-thread worker (the task-25 channel); the experiment Scheduler is admission-only with no consumer".to_string(),
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
        "finding: no CPU-fairness seam exists — the design's runaway scenarios (never-yielding task, runaway child tree) have no scheduler to starve and no mechanism to measure".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no task executor or worker-pool scheduling discipline exists in any phlow crate — a tokenized workspace scan finds zero scheduling-discipline tokens (no preempt, no timeslice/time_slice, no fairness; the sole 'quantum' hits are the quantum-proof cryptography term), zero worker-pool vocabulary, and the only async task machinery is the msgpack transport's single current-thread worker; the experiment Scheduler is admission-only with no consumer. The design's pass criteria (no task's completion time exceeds Kx its solo time under a runaway; the mechanism named, not emergent) need a scheduler to attach to, and there is none.".to_string(),
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
