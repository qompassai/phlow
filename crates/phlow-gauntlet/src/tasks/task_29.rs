//! task-29: lease fencing (rust).
//!
//! Recon task. The design asks for lease fencing between two workers
//! contending for a lease and instructs: "locate phlow's distributed lock
//! / leadership lease (if none, document; the worker then tests the
//! *absence* as the finding)."
//!
//! Phlow has no distributed lock, lease, or fencing primitive. The driver
//! proves the absence two ways: a token scan over the
//! coordination-relevant crates (`phlow-experiment`, `phlow-runtime`,
//! `phlow-agent` — fail-closed if lease/fencing tokens ever appear), and
//! behavioral probes against the real experiment `Scheduler` showing the
//! closest existing mechanism is *not* fencing — generation-checked
//! publication (`StaleGeneration` / `DuplicateResult`) answers
//! at-most-once publication, not mutual exclusion: there is no lease to
//! acquire, no holder identity, no expiry, and no shared lease store, so
//! the design's "two workers contend" scenario is unrepresentable.
//!
//! The task verdict is `fail` with `where = "seam"`: the design's fencing
//! pass criteria (at most one writer's effects visible per fencing epoch;
//! a stale holder's write rejected with an explicit fencing error) have no
//! lease to evaluate them against — an open design gap, not a driver
//! error. The integration tests assert the probe evidence is correct.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_experiment::{
    CapabilitySet, ExperimentError, ExperimentId, NodeId, NodeParams, NodeState, RunId, Scheduler,
    SchedulerLimits, SchedulerNode, WorkerRole,
};
use std::fmt;
use std::path::{Path, PathBuf};

/// Task id.
pub const ID: &str = "task-29";
/// Human-readable name.
pub const NAME: &str = "lease fencing";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "no_lease_primitive_in_experiment",
    "no_lease_primitive_in_runtime_or_agent",
    "generation_check_is_not_fencing",
    "two_workers_cannot_contend",
];

/// Max source files scanned per crate directory (fail-closed bound).
const SCAN_FILES_MAX: usize = 500;
/// Max bytes read per source file (fail-closed bound).
const SCAN_BYTES_MAX: u64 = 1 << 20;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-29 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A workspace path was missing or unreadable.
    Path {
        /// Which file was wanted.
        what: String,
        /// Path plus I/O detail.
        detail: String,
    },
    /// The recon premise changed: lease/fencing tokens appeared.
    ReconChanged {
        /// What changed.
        detail: String,
    },
    /// A fixture could not be built.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path { what, detail } => write!(f, "task-29: cannot read {what}: {detail}"),
            Self::ReconChanged { detail } => {
                write!(f, "task-29: recon premise changed: {detail}")
            }
            Self::Fixture { what, detail } => {
                write!(f, "task-29: cannot build fixture {what}: {detail}")
            }
        }
    }
}

impl std::error::Error for DriverError {}

// ---------------------------------------------------------------------------
// Recon: token-scan the coordination crates for lease/fencing machinery
// ---------------------------------------------------------------------------

fn workspace_root() -> Result<PathBuf, DriverError> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .ok_or_else(|| DriverError::Path {
            what: "workspace root".to_string(),
            detail: "CARGO_MANIFEST_DIR has fewer than 2 ancestors".to_string(),
        })
}

/// Collect `.rs` files under `dir`, bounded. Skips `target/` build output.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), DriverError> {
    if out.len() >= SCAN_FILES_MAX {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|e| DriverError::Path {
        what: "crate src dir".to_string(),
        detail: format!("{}: {e}", dir.display()),
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| DriverError::Path {
            what: "dir entry".to_string(),
            detail: e.to_string(),
        })?;
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
            collect_rs_files(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
        if out.len() >= SCAN_FILES_MAX {
            return Ok(());
        }
    }
    Ok(())
}

/// True when the source text contains a standalone lease/fencing token.
/// Tokenized on non-alphanumeric boundaries so `release`/`released` (which
/// contain the substring "lease") never match.
fn has_lease_token(text: &str) -> Option<String> {
    for token in text
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
    {
        match token.as_str() {
            "lease" | "leases" | "fencing" | "fence" | "fences" => return Some(token),
            _ => {}
        }
    }
    None
}

/// Scan one crate's `src/` for lease/fencing tokens. Returns the evidence
/// lines; fails closed on the first hit.
fn scan_crate(crate_name: &str) -> Result<Vec<String>, DriverError> {
    let dir = workspace_root()?
        .join("crates")
        .join(crate_name)
        .join("src");
    let mut files = Vec::new();
    collect_rs_files(&dir, &mut files)?;
    let mut evidence = Vec::new();
    for file in &files {
        let bytes = std::fs::read(file).map_err(|e| DriverError::Path {
            what: "source file".to_string(),
            detail: format!("{}: {e}", file.display()),
        })?;
        let capped = &bytes[..bytes.len().min(SCAN_BYTES_MAX as usize)];
        let text = String::from_utf8_lossy(capped);
        if let Some(token) = has_lease_token(&text) {
            return Err(DriverError::ReconChanged {
                detail: format!(
                    "{}/src/{} now contains the token '{token}'; lease/fencing machinery may exist",
                    crate_name,
                    file.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                ),
            });
        }
    }
    evidence.push(format!(
        "recon: {crate_name}/src ({} files): no lease/fencing tokens",
        files.len()
    ));
    Ok(evidence)
}

// ---------------------------------------------------------------------------
// Fixtures: the real Scheduler
// ---------------------------------------------------------------------------

fn fixture_error(what: &str, detail: impl fmt::Display) -> DriverError {
    DriverError::Fixture {
        what: what.to_string(),
        detail: detail.to_string(),
    }
}

fn make_scheduler() -> Result<Scheduler, DriverError> {
    let limits = SchedulerLimits {
        queue_capacity: 64,
        ..Default::default()
    };
    Scheduler::new(limits).map_err(|e| fixture_error("scheduler", e))
}

fn make_ids(tag: &str) -> Result<(RunId, ExperimentId), DriverError> {
    let run = RunId::new(&format!("run-{tag}")).map_err(|e| fixture_error("run id", e))?;
    let exp =
        ExperimentId::new(&format!("exp-{tag}")).map_err(|e| fixture_error("experiment id", e))?;
    Ok((run, exp))
}

fn make_capabilities() -> Result<CapabilitySet, DriverError> {
    CapabilitySet::new(
        vec!["read".to_string()],
        vec!["workspace".to_string()],
        64,
        65_536,
    )
    .map_err(|e| fixture_error("capabilities", e))
}

fn make_node(
    key: &str,
    run: &RunId,
    exp: &ExperimentId,
    caps: &CapabilitySet,
) -> Result<SchedulerNode, DriverError> {
    let node_id = NodeId::new(key).map_err(|e| fixture_error("node id", e))?;
    SchedulerNode::new(NodeParams {
        run_id: run.clone(),
        experiment_id: exp.clone(),
        baseline_revision: "rev-1".to_string(),
        workspace_snapshot: "snap-1".to_string(),
        node_id,
        parent_node_id: None,
        role: WorkerRole::Implementer,
        capabilities: caps.clone(),
        input_digest: format!("input-{key}"),
        dependency_ids: Vec::new(),
        generation: 0,
        attempt: 0,
        deadline_ms: 300_000,
        cpu_budget_ms: 1_000,
        memory_budget_bytes: 1_048_576,
        output_bytes_max: 4_096,
        tool_calls_remaining: 10,
    })
    .map_err(|e| fixture_error("node", e))
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

/// V1: no lease/fencing primitive in the experiment crate.
fn case_no_lease_in_experiment() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_lease_primitive_in_experiment";
    let evidence = scan_crate("phlow-experiment")?;
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"crate": "phlow-experiment", "lease_tokens": 0}),
        evidence,
    ))
}

/// V2: no lease/fencing primitive in the runtime or agent crates.
fn case_no_lease_in_runtime_or_agent() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_lease_primitive_in_runtime_or_agent";
    let mut evidence = scan_crate("phlow-runtime")?;
    evidence.extend(scan_crate("phlow-agent")?);
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"crates": ["phlow-runtime", "phlow-agent"], "lease_tokens": 0}),
        evidence,
    ))
}

/// A1: the closest existing mechanism — generation-checked publication —
/// is not fencing. A stale generation is rejected (`StaleGeneration`) and
/// a second publish for the same generation is refused
/// (`DuplicateResult`), but there is no lease to acquire, no holder
/// identity, and no expiry: anyone presenting the right generation can
/// publish. That is at-most-once publication, not mutual exclusion.
fn case_generation_is_not_fencing() -> Result<CaseReport, DriverError> {
    const CASE: &str = "generation_check_is_not_fencing";
    let mut evidence = Vec::new();
    let (run, exp) = make_ids("fence1")?;
    let caps = make_capabilities()?;
    let mut sched = make_scheduler()?;
    let node = make_node("node-w", &run, &exp, &caps)?;
    let node_id = node.node_id().clone();
    sched.admit(node).map_err(|e| fixture_error("admit", e))?;
    let generation = sched
        .node(&node_id)
        .ok_or_else(|| fixture_error("lookup", "node vanished"))?
        .generation();
    // A "stale holder" presents the wrong generation: rejected — but with
    // StaleGeneration, not a fencing error, and there was never a lease.
    match sched.publish_result(
        &node_id,
        generation + 1,
        "digest-stale",
        NodeState::Succeeded,
    ) {
        Err(ExperimentError::StaleGeneration {
            node,
            expected,
            got,
        }) => {
            evidence.push(format!(
                "wrong-generation publish rejected: StaleGeneration {{ node: {node}, expected: {expected}, got: {got} }}"
            ));
        }
        Err(other) => {
            return Ok(CaseReport::fail(
                CASE,
                format!("wrong-generation publish gave '{other}', want StaleGeneration"),
                evidence,
            ));
        }
        Ok(()) => {
            return Ok(CaseReport::fail(
                CASE,
                "wrong-generation publish succeeded".to_string(),
                evidence,
            ));
        }
    }
    // Correct generation publishes; a second "writer" with the same
    // generation is refused — at-most-once, not fencing.
    sched
        .publish_result(&node_id, generation, "digest-first", NodeState::Succeeded)
        .map_err(|e| fixture_error("first publish", e))?;
    match sched.publish_result(&node_id, generation, "digest-second", NodeState::Succeeded) {
        Err(ExperimentError::DuplicateResult { .. }) => {
            evidence.push(
                "second publish for the same generation refused with DuplicateResult: at-most-once publication"
                    .to_string(),
            );
        }
        Err(other) => {
            return Ok(CaseReport::fail(
                CASE,
                format!("second publish gave '{other}', want DuplicateResult"),
                evidence,
            ));
        }
        Ok(()) => {
            return Ok(CaseReport::fail(
                CASE,
                "second publish succeeded: two writers' effects visible".to_string(),
                evidence,
            ));
        }
    }
    evidence.push(
        "this is not fencing: no lease was acquired, no holder identity exists, nothing expires — anyone presenting the generation may publish"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"stale_rejected": true, "double_publish_refused": true}),
        evidence,
    ))
}

/// A2: two "workers" (independent schedulers) both admit the same key —
/// each succeeds, because there is no shared lease store to contend on.
/// The design's contention scenario is unrepresentable: the absence is
/// behavioral, not just textual.
fn case_two_workers_cannot_contend() -> Result<CaseReport, DriverError> {
    const CASE: &str = "two_workers_cannot_contend";
    let mut evidence = Vec::new();
    let (run, exp) = make_ids("fence2")?;
    let caps = make_capabilities()?;
    let mut worker_a = make_scheduler()?;
    let mut worker_b = make_scheduler()?;
    worker_a
        .admit(make_node("shared-key", &run, &exp, &caps)?)
        .map_err(|e| fixture_error("worker A admit", e))?;
    // Worker B admits the same key into its own ledger: no contention,
    // because there is no shared lease store — the "loser" is never told.
    worker_b
        .admit(make_node("shared-key", &run, &exp, &caps)?)
        .map_err(|e| fixture_error("worker B admit", e))?;
    evidence.push(
        "two independent workers admitted the same key 'shared-key': both succeeded".to_string(),
    );
    evidence.push(
        "no shared lease store exists, so the design's 'two workers contend for a lease' cannot be set up — the loser is never fenced because there is nothing to hold"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"workers": 2, "admissions_ok": 2, "contention_possible": false}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "no_lease_primitive_in_experiment" => case_no_lease_in_experiment(),
        "no_lease_primitive_in_runtime_or_agent" => case_no_lease_in_runtime_or_agent(),
        "generation_check_is_not_fencing" => case_generation_is_not_fencing(),
        "two_workers_cannot_contend" => case_two_workers_cannot_contend(),
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

/// The honest task verdict: the probes all pass (the evidence is correct),
/// but the designed lease/fencing seam is absent, so the design's fencing
/// pass criteria have nothing to evaluate against. Recorded as an open
/// design gap.
fn seam_finding(evidence: Vec<String>) -> TaskFailure {
    TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: phlow has no distributed lock, lease, or fencing primitive — \
              token scans of phlow-experiment, phlow-runtime and phlow-agent find no lease/fencing \
              machinery, and the closest mechanism (generation-checked publication) is at-most-once \
              publication, not mutual exclusion. Two workers cannot contend because there is no \
              shared lease store. The fencing pass criteria cannot be evaluated; open design gap."
            .to_string(),
        evidence,
    }
}

fn run_inner(_ctx: &Ctx) -> Result<Vec<String>, TaskFailure> {
    let mut evidence = Vec::new();
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
    Err(seam_finding(evidence))
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
