//! task-22: DAG diamond dependencies (rust).
//!
//! Recon task: the design asks for phlow's DAG/workflow *executor* and
//! forbids mocking it ("Mocks: none for the executor"). The driver
//! (`ensure_scenario_binary` + cases below) proves the executor seam is
//! ABSENT:
//!
//! * `phlow_experiment::control_plane::SchedulerNode` carries bounded
//!   `dependency_ids` (`DEPENDENCY_IDS_MAX`) — the *metadata* seam exists.
//! * `Scheduler::admit` never reads `dependency_ids`: edges are stored,
//!   never enforced. There is no topological ordering, no ready-set, no
//!   `execute` — and the crate's own docs say it enables "no scheduler
//!   execution" (the Scheduler "spawns nothing, runs nothing").
//!
//! The scenario binary compiles phlow's real `control_plane.rs` +
//! `error.rs` via `#[path]` (no mocks, no copies) and demonstrates the four
//! honest behaviors: diamond admission with edge metadata intact,
//! unenforced dependencies, duplicate-admit rejection, and shared output
//! reads. The task verdict is then `fail` with `where = "seam"`: the
//! design's execution-ordering pass criteria (A executes once, D after B
//! and C, idempotent join) have no executor to evaluate them against — an
//! open design gap, not a driver error. The integration tests assert the
//! probe evidence is correct.
//!
//! [`Scheduler::admit`]: https://github.com/qompassai/phlow (local path
//! `crates/phlow-experiment/src/control_plane.rs`)

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Task id.
pub const ID: &str = "task-22";
/// Human-readable name.
pub const NAME: &str = "DAG diamond dependencies";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the scenario binary runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "diamond_admit",
    "dependencies_not_enforced",
    "duplicate_admit_rejected",
    "shared_output_read",
];

/// Wall-clock bound for compiling the scenario binary.
const COMPILE_TIMEOUT: Duration = Duration::from_secs(90);
/// Wall-clock bound for one scenario case.
const CASE_TIMEOUT: Duration = Duration::from_secs(30);
/// Cap on captured child output (stdout+stderr), in bytes.
const OUTPUT_BYTES_MAX: usize = 1 << 20;
/// Poll interval while waiting on a child process.
const WAIT_POLL: Duration = Duration::from_millis(50);

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-22 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A workspace path was missing or unreadable.
    Path {
        /// Which file was wanted.
        what: String,
        /// Path plus I/O detail.
        detail: String,
    },
    /// The seam recon premise no longer holds.
    ReconChanged {
        /// What changed.
        detail: String,
    },
    /// No Rust toolchain found to compile the scenario.
    Toolchain {
        /// What was tried.
        detail: String,
    },
    /// A child process could not be spawned.
    Spawn {
        /// Which child.
        what: String,
        /// I/O detail.
        detail: String,
    },
    /// A child process exceeded its wall-clock bound and was killed.
    Timeout {
        /// Which child.
        what: String,
        /// The bound in milliseconds.
        timeout_ms: u64,
    },
    /// Child output could not be collected.
    Output {
        /// Which child.
        what: String,
        /// I/O detail.
        detail: String,
    },
    /// rustc rejected the scenario build.
    Compile {
        /// Bounded compiler stderr.
        detail: String,
    },
    /// The scenario printed no parseable JSON verdict.
    Verdict {
        /// Which case.
        case: String,
        /// What was observed.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path { what, detail } => write!(f, "task-22: cannot read {what}: {detail}"),
            Self::ReconChanged { detail } => write!(f, "task-22: recon premise changed: {detail}"),
            Self::Toolchain { detail } => write!(f, "task-22: no Rust toolchain: {detail}"),
            Self::Spawn { what, detail } => write!(f, "task-22: cannot spawn {what}: {detail}"),
            Self::Timeout { what, timeout_ms } => {
                write!(f, "task-22: {what} exceeded {timeout_ms} ms and was killed")
            }
            Self::Output { what, detail } => {
                write!(f, "task-22: cannot read {what} output: {detail}")
            }
            Self::Compile { detail } => write!(f, "task-22: scenario build failed: {detail}"),
            Self::Verdict { case, detail } => {
                write!(f, "task-22: case '{case}' gave no JSON verdict: {detail}")
            }
        }
    }
}

impl std::error::Error for DriverError {}

// ---------------------------------------------------------------------------
// Workspace locations (derived, never guessed)
// ---------------------------------------------------------------------------

/// Repository root, derived from this crate's manifest directory:
/// `<root>/crates/phlow-gauntlet` → ancestors → `<root>`.
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

/// `crates/phlow-experiment/src`, verified to hold the scheduler sources.
fn experiment_src_dir() -> Result<PathBuf, DriverError> {
    let dir = workspace_root()?
        .join("crates")
        .join("phlow-experiment")
        .join("src");
    for file in ["error.rs", "control_plane.rs", "lib.rs"] {
        if !dir.join(file).is_file() {
            return Err(DriverError::Path {
                what: "phlow-experiment source".to_string(),
                detail: format!("missing {}", dir.join(file).display()),
            });
        }
    }
    Ok(dir)
}

/// Locate `rustc`: first on `PATH`, then the rustup shim under `$HOME`.
/// Never assumes the ambient `PATH`; the sandbox shell does not carry one.
fn find_rustc() -> Result<PathBuf, DriverError> {
    let on_path = Command::new("rustc")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if on_path {
        return Ok(PathBuf::from("rustc"));
    }
    if let Ok(home) = std::env::var("HOME") {
        let shim = PathBuf::from(home).join(".cargo").join("bin").join("rustc");
        if shim.is_file() {
            return Ok(shim);
        }
    }
    Err(DriverError::Toolchain {
        detail: "rustc not on PATH and no ~/.cargo/bin/rustc shim".to_string(),
    })
}

fn read_file(path: &Path, what: &str) -> Result<String, DriverError> {
    std::fs::read_to_string(path).map_err(|e| DriverError::Path {
        what: what.to_string(),
        detail: format!("{}: {e}", path.display()),
    })
}

// ---------------------------------------------------------------------------
// Recon: re-verify the seam premises against the live sources on every run
// ---------------------------------------------------------------------------

/// Verify the task's scope premises against the live sources and return the
/// evidence lines. Fails closed: if the crate ever gains a DAG executor,
/// the old absence claims must not silently pass.
fn recon() -> Result<Vec<String>, DriverError> {
    let src = experiment_src_dir()?;
    let lib_rs = read_file(&src.join("lib.rs"), "phlow-experiment lib.rs")?;
    let control_plane = read_file(&src.join("control_plane.rs"), "control_plane.rs")?;
    if !lib_rs.contains("no scheduler execution") {
        return Err(DriverError::ReconChanged {
            detail: "phlow-experiment/src/lib.rs no longer states 'no scheduler execution'".into(),
        });
    }
    // Premise 1: the dependency-ids metadata seam exists.
    if !control_plane.contains("dependency_ids") {
        return Err(DriverError::ReconChanged {
            detail: "control_plane.rs no longer mentions dependency_ids; metadata seam changed"
                .into(),
        });
    }
    // Premise 2: Scheduler::admit never consults dependency_ids — the edges
    // are stored, never enforced. Extract admit's body and check.
    let admit_start =
        control_plane
            .find("pub fn admit(")
            .ok_or_else(|| DriverError::ReconChanged {
                detail: "Scheduler::admit not found in control_plane.rs".into(),
            })?;
    let after_admit = &control_plane[admit_start..];
    let admit_end = after_admit
        .find("\n    pub fn ")
        .map(|i| admit_start + i)
        .unwrap_or(control_plane.len());
    let admit_body = &control_plane[admit_start..admit_end];
    if admit_body.contains("dependency") {
        return Err(DriverError::ReconChanged {
            detail: "Scheduler::admit now references dependency_ids; enforcement may exist".into(),
        });
    }
    // Premise 3: no DAG executor machinery anywhere in the scheduler file.
    let executor_needles = [
        "topolog",
        "fn execute",
        "execution_order",
        "ready_nodes",
        "fn run_node",
        "fn schedule_next",
    ];
    for needle in executor_needles {
        if control_plane.contains(needle) {
            return Err(DriverError::ReconChanged {
                detail: format!(
                    "control_plane.rs now contains '{needle}'; executor machinery may exist"
                ),
            });
        }
    }
    Ok(vec![
        "recon: crates/phlow-experiment/src/lib.rs states the crate enables 'no scheduler execution' (verified present)".to_string(),
        "recon: SchedulerNode carries dependency_ids (bounded DEPENDENCY_IDS_MAX) — the metadata seam exists".to_string(),
        "recon: Scheduler::admit body contains no reference to dependency_ids — edges are stored, never enforced".to_string(),
        "recon: no topological/execute/ready-set machinery in control_plane.rs (6 needles absent)".to_string(),
        "finding: phlow has a DAG-shaped admission ledger, not a DAG executor — the task-22 executor seam is absent".to_string(),
    ])
}

// ---------------------------------------------------------------------------
// Bounded child processes
// ---------------------------------------------------------------------------

/// Run a child with a wall-clock bound: poll, kill on expiry, then collect
/// output. Never blocks without a deadline.
fn run_bounded(
    mut cmd: Command,
    timeout: Duration,
    what: &str,
) -> Result<std::process::Output, DriverError> {
    let mut child = cmd.spawn().map_err(|e| DriverError::Spawn {
        what: what.to_string(),
        detail: e.to_string(),
    })?;
    let started = Instant::now();
    loop {
        match child.try_wait().map_err(|e| DriverError::Output {
            what: what.to_string(),
            detail: format!("try_wait failed: {e}"),
        })? {
            Some(_) => break,
            None => {
                if started.elapsed() > timeout {
                    let _ = child.kill();
                    return Err(DriverError::Timeout {
                        what: what.to_string(),
                        timeout_ms: timeout.as_millis() as u64,
                    });
                }
                std::thread::sleep(WAIT_POLL);
            }
        }
    }
    let mut output = child.wait_with_output().map_err(|e| DriverError::Output {
        what: what.to_string(),
        detail: e.to_string(),
    })?;
    output.stdout.truncate(OUTPUT_BYTES_MAX);
    output.stderr.truncate(OUTPUT_BYTES_MAX);
    Ok(output)
}

// ---------------------------------------------------------------------------
// Scenario binary: compile phlow's real scheduler sources with rustc
// ---------------------------------------------------------------------------

/// Build the scenario binary in `work_dir`.
///
/// The scenario embeds the crate's own `error.rs` and `control_plane.rs`
/// via `#[path]` — the real scheduler sources, read at compile time. Those
/// two files import only `std` (verified), so the scenario needs no
/// dependencies beyond what `rustc` ships.
pub fn ensure_scenario_binary(work_dir: &Path) -> Result<PathBuf, DriverError> {
    std::fs::create_dir_all(work_dir).map_err(|e| DriverError::Path {
        what: "task work dir".to_string(),
        detail: format!("{}: {e}", work_dir.display()),
    })?;
    let src_dir = experiment_src_dir()?;
    let scenario_rs = work_dir.join("scenario.rs");
    let binary = work_dir.join("scenario");
    let source = SCENARIO_TEMPLATE.replace("{{SRC_DIR}}", &src_dir.to_string_lossy());
    std::fs::write(&scenario_rs, source).map_err(|e| DriverError::Path {
        what: "scenario source".to_string(),
        detail: format!("{}: {e}", scenario_rs.display()),
    })?;
    let rustc = find_rustc()?;
    let mut cmd = Command::new(&rustc);
    cmd.arg("--edition=2024")
        .arg("-O")
        .arg("--crate-name")
        .arg("task22_scenario")
        .arg(&scenario_rs)
        .arg("-o")
        .arg(&binary)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = run_bounded(cmd, COMPILE_TIMEOUT, "rustc scenario build")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DriverError::Compile {
            detail: stderr.chars().take(2000).collect(),
        });
    }
    Ok(binary)
}

// ---------------------------------------------------------------------------
// Case verdicts
// ---------------------------------------------------------------------------

/// The parsed verdict of one scenario case.
#[derive(Debug, Clone)]
pub struct CaseReport {
    /// Which case ran.
    pub case: String,
    /// Whether the case's own assertions held.
    pub passed: bool,
    /// Measured numbers and observed values.
    pub metrics: serde_json::Value,
    /// Diagnostic lines from the scenario.
    pub evidence: Vec<String>,
    /// Failing assertion details, empty when `passed`.
    pub failures: Vec<String>,
}

fn str_vec(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Run one scenario case with a wall-clock bound and parse its JSON verdict.
pub fn run_case(binary: &Path, case: &str) -> Result<CaseReport, DriverError> {
    let mut cmd = Command::new(binary);
    cmd.arg(case).stdout(Stdio::piped()).stderr(Stdio::piped());
    let output = run_bounded(cmd, CASE_TIMEOUT, case)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find(|l| l.trim_start().starts_with('{'))
        .ok_or_else(|| {
            let stderr = String::from_utf8_lossy(&output.stderr);
            DriverError::Verdict {
                case: case.to_string(),
                detail: format!(
                    "no JSON verdict on stdout (exit={:?}); stderr: {}",
                    output.status.code(),
                    stderr.chars().take(500).collect::<String>()
                ),
            }
        })?;
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|e| DriverError::Verdict {
            case: case.to_string(),
            detail: format!("verdict is not JSON: {e}"),
        })?;
    let verdict_case = value
        .get("case")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if verdict_case != case {
        return Err(DriverError::Verdict {
            case: case.to_string(),
            detail: format!("verdict case '{verdict_case}' does not match"),
        });
    }
    Ok(CaseReport {
        case: verdict_case,
        passed: value
            .get("passed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        metrics: value
            .get("metrics")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        evidence: str_vec(value.get("evidence").unwrap_or(&serde_json::Value::Null)),
        failures: str_vec(value.get("failures").unwrap_or(&serde_json::Value::Null)),
    })
}

fn render_metrics(metrics: &serde_json::Value) -> String {
    match metrics.as_object() {
        Some(map) => map
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(" "),
        None => metrics.to_string(),
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
/// but the designed executor seam is absent, so the design's
/// execution-ordering pass criteria have nothing to evaluate against.
/// Recorded as an open design gap.
fn seam_finding(evidence: Vec<String>) -> TaskFailure {
    TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: phlow has no DAG executor — SchedulerNode.dependency_ids is inert \
              admission metadata (Scheduler::admit never reads it; no topological ordering, no \
              ready-set, no execute; crate docs: 'no scheduler execution'). The diamond's \
              execution-ordering pass criteria cannot be evaluated; open design gap."
            .to_string(),
        evidence,
    }
}

fn run_inner(ctx: &Ctx) -> Result<Vec<String>, TaskFailure> {
    let mut evidence = recon().map_err(|e| TaskFailure {
        where_: "recon".to_string(),
        how: e.to_string(),
        evidence: Vec::new(),
    })?;
    let work_dir = ctx.work_dir.join("task-22");
    let binary = ensure_scenario_binary(&work_dir).map_err(|e| TaskFailure {
        where_: "harness".to_string(),
        how: e.to_string(),
        evidence: evidence.clone(),
    })?;
    evidence.push(format!("harness: scenario binary at {}", binary.display()));
    evidence.push(
        "harness: scenario compiles phlow's own error.rs + control_plane.rs via #[path]; no mocks, no copies"
            .to_string(),
    );
    for case in CASES {
        let report = run_case(&binary, case).map_err(|e| TaskFailure {
            where_: case.to_string(),
            how: e.to_string(),
            evidence: evidence.clone(),
        })?;
        evidence.push(format!("case {case}: passed={}", report.passed));
        evidence.push(format!(
            "case {case} metrics: {}",
            render_metrics(&report.metrics)
        ));
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

// ---------------------------------------------------------------------------
// Scenario program: compiled by rustc, drives the REAL scheduler sources.
// ---------------------------------------------------------------------------

/// The diamond scenario, compiled with `rustc --edition=2024`. `{{SRC_DIR}}`
/// is replaced with the absolute path of `crates/phlow-experiment/src`; the
/// two `#[path]` modules then compile phlow's actual scheduler sources into
/// this binary. Those files import only `std`, so no dependencies are needed.
///
/// Scope, stated plainly: phlow ships no DAG executor. The Scheduler is an
/// admission ledger: `admit` stores nodes (with their `dependency_ids`
/// metadata) and `publish_result` records terminal digests. Nothing orders
/// execution by edges. These cases prove exactly that — the metadata is
/// faithful, the edges are unenforced, and the ledger's own exactly-once
/// properties (duplicate admit/result rejected) hold.
const SCENARIO_TEMPLATE: &str = r##"//! task-22 diamond scenario: drives phlow's real scheduler admission
//! ledger through a diamond dependency shape A -> {B, C} -> D.
//!
//! Compiled by `crates/phlow-gauntlet/src/tasks/task_22.rs` with `rustc`,
//! embedding the crate's own sources via `#[path]` — the real
//! `phlow_experiment::control_plane::Scheduler`, read at compile time.
//! No mocks, no copies.

#[path = "{{SRC_DIR}}/error.rs"]
mod error;
#[path = "{{SRC_DIR}}/control_plane.rs"]
mod control_plane;

use control_plane::{
    CapabilitySet, ExperimentId, NodeId, NodeParams, NodeState, RunId, Scheduler, SchedulerLimits,
    SchedulerNode, WorkerRole,
};
use error::ExperimentError;

// ---------------------------------------------------------------------------
// Minimal JSON writer (the scenario binary carries no dependencies)
// ---------------------------------------------------------------------------

fn json_escape_into(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", ch as u32));
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}

/// Tiny JSON object builder: string keys with u64/str/bool values.
struct JsonObj {
    buf: String,
    first: bool,
}

impl JsonObj {
    fn new() -> Self {
        Self {
            buf: String::from("{"),
            first: true,
        }
    }

    fn sep(&mut self) {
        if !self.first {
            self.buf.push(',');
        }
        self.first = false;
    }

    fn num(&mut self, key: &str, value: u64) -> &mut Self {
        self.sep();
        json_escape_into(&mut self.buf, key);
        self.buf.push(':');
        self.buf.push_str(&value.to_string());
        self
    }

    fn text(&mut self, key: &str, value: &str) -> &mut Self {
        self.sep();
        json_escape_into(&mut self.buf, key);
        self.buf.push(':');
        json_escape_into(&mut self.buf, value);
        self
    }

    fn boolean(&mut self, key: &str, value: bool) -> &mut Self {
        self.sep();
        json_escape_into(&mut self.buf, key);
        self.buf.push(':');
        self.buf.push_str(if value { "true" } else { "false" });
        self
    }

    fn finish(mut self) -> String {
        self.buf.push('}');
        self.buf
    }
}

// ---------------------------------------------------------------------------
// Fixture builders
// ---------------------------------------------------------------------------

fn make_ids(tag: &str) -> Result<(RunId, ExperimentId), String> {
    let run = RunId::new(&format!("run-{tag}")).map_err(|e| e.to_string())?;
    let exp = ExperimentId::new(&format!("exp-{tag}")).map_err(|e| e.to_string())?;
    Ok((run, exp))
}

fn make_capabilities() -> Result<CapabilitySet, String> {
    CapabilitySet::new(
        vec!["read".to_string()],
        vec!["workspace".to_string()],
        64,
        65_536,
    )
    .map_err(|e| e.to_string())
}

fn make_node(
    name: &str,
    run: &RunId,
    exp: &ExperimentId,
    caps: &CapabilitySet,
    dependency_ids: Vec<NodeId>,
) -> Result<SchedulerNode, String> {
    let node_id = NodeId::new(name).map_err(|e| e.to_string())?;
    SchedulerNode::new(NodeParams {
        run_id: run.clone(),
        experiment_id: exp.clone(),
        baseline_revision: "rev-1".to_string(),
        workspace_snapshot: "snap-1".to_string(),
        node_id,
        parent_node_id: None,
        role: WorkerRole::Implementer,
        capabilities: caps.clone(),
        input_digest: format!("input-{name}"),
        dependency_ids,
        generation: 0,
        attempt: 0,
        deadline_ms: 300_000,
        cpu_budget_ms: 1_000,
        memory_budget_bytes: 1_048_576,
        output_bytes_max: 4_096,
        tool_calls_remaining: 10,
    })
    .map_err(|e| e.to_string())
}

fn make_scheduler() -> Result<Scheduler, String> {
    let limits = SchedulerLimits {
        queue_capacity: 64,
        ..Default::default()
    };
    Scheduler::new(limits).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

struct CaseReport {
    case: &'static str,
    passed: bool,
    metrics: String,
    evidence: Vec<String>,
    failures: Vec<String>,
}

/// V1: the diamond admits with its edge metadata intact; A is admitted
/// exactly once (second admit is DuplicateNode, not a second A).
fn case_diamond_admit() -> CaseReport {
    let mut failures: Vec<String> = Vec::new();
    let mut evidence: Vec<String> = Vec::new();
    let mut m = JsonObj::new();

    let result: Result<(), String> = (|| {
        let (run, exp) = make_ids("diamond")?;
        let caps = make_capabilities()?;
        let mut sched = make_scheduler()?;
        let a = make_node("node-a", &run, &exp, &caps, vec![])?;
        let a_id = a.node_id().clone();
        sched.admit(a).map_err(|e| format!("admit A: {e}"))?;
        // A executes "once": the ledger refuses a second admission of A.
        let a2 = make_node("node-a", &run, &exp, &caps, vec![])?;
        match sched.admit(a2) {
            Err(ExperimentError::DuplicateNode { .. }) => {}
            Err(other) => return Err(format!("re-admit A gave {other}, want DuplicateNode")),
            Ok(()) => return Err("re-admit A succeeded: A admitted twice".to_string()),
        }
        let b = make_node("node-b", &run, &exp, &caps, vec![a_id.clone()])?;
        let c = make_node("node-c", &run, &exp, &caps, vec![a_id.clone()])?;
        let b_id = b.node_id().clone();
        let c_id = c.node_id().clone();
        sched.admit(b).map_err(|e| format!("admit B: {e}"))?;
        sched.admit(c).map_err(|e| format!("admit C: {e}"))?;
        let d = make_node(
            "node-d",
            &run,
            &exp,
            &caps,
            vec![b_id.clone(), c_id.clone()],
        )?;
        let d_id = d.node_id().clone();
        sched.admit(d).map_err(|e| format!("admit D: {e}"))?;
        // Edge metadata is stored faithfully on every node.
        let edges = |id: &NodeId| {
            sched
                .node(id)
                .map(|n| {
                    n.dependency_ids()
                        .iter()
                        .map(|d| d.as_str().to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let ea: Vec<String> = edges(&a_id);
        let eb = edges(&b_id);
        let ec = edges(&c_id);
        let ed = edges(&d_id);
        if !ea.is_empty() || eb != ["node-a"] || ec != ["node-a"] {
            return Err(format!("edge metadata wrong: A={ea:?} B={eb:?} C={ec:?}"));
        }
        let mut ed_sorted = ed.clone();
        ed_sorted.sort();
        if ed_sorted != ["node-b", "node-c"] {
            return Err(format!("D edges wrong: {ed:?}"));
        }
        m.num("nodes_admitted", 4)
            .text("a_edges", &ea.join(","))
            .text("b_edges", &eb.join(","))
            .text("c_edges", &ec.join(","))
            .text("d_edges", &ed_sorted.join(","))
            .boolean("a_readmit_duplicate_node", true);
        evidence.push(
            "diamond A->{B,C}->D admitted: 4 nodes, edge metadata intact (B:[A] C:[A] D:[B,C])"
                .to_string(),
        );
        evidence.push("A re-admitted -> DuplicateNode: the ledger holds A exactly once".to_string());
        Ok(())
    })();
    if let Err(detail) = result {
        failures.push(detail);
    }
    CaseReport {
        case: "diamond_admit",
        passed: failures.is_empty(),
        metrics: m.finish(),
        evidence,
        failures,
    }
}

/// V2: dependency edges are NOT enforced — a node whose dependencies were
/// never admitted (or never existed) is still admitted. The honest proof
/// that no executor/gate reads dependency_ids.
fn case_dependencies_not_enforced() -> CaseReport {
    let mut failures: Vec<String> = Vec::new();
    let mut evidence: Vec<String> = Vec::new();
    let mut m = JsonObj::new();

    let result: Result<(), String> = (|| {
        let (run, exp) = make_ids("unenforced")?;
        let caps = make_capabilities()?;
        let mut sched = make_scheduler()?;
        let ghost_b = NodeId::new("ghost-b").map_err(|e| e.to_string())?;
        let ghost_c = NodeId::new("ghost-c").map_err(|e| e.to_string())?;
        let d = make_node(
            "node-d-early",
            &run,
            &exp,
            &caps,
            vec![ghost_b.clone(), ghost_c.clone()],
        )?;
        let d_id = d.node_id().clone();
        // D "depends" on B and C, which were never admitted and never will
        // be. A real DAG gate would refuse; the ledger admits.
        sched
            .admit(d)
            .map_err(|e| format!("admit of D with unadmitted deps failed: {e}"))?;
        let node = sched
            .node(&d_id)
            .ok_or_else(|| "admitted D vanished".to_string())?;
        if node.state() != NodeState::Admitted {
            return Err(format!("D in state {:?}, want Admitted", node.state()));
        }
        m.num("deps_declared", 2)
            .num("deps_admitted", 0)
            .boolean("d_admitted", true);
        evidence.push(
            "D admitted with dependency_ids=[ghost-b, ghost-c], neither ever admitted: \
             edges are not enforced by admit"
                .to_string(),
        );
        evidence.push(
            "this is the behavioral proof of the recon claim: Scheduler::admit never reads dependency_ids"
                .to_string(),
        );
        Ok(())
    })();
    if let Err(detail) = result {
        failures.push(detail);
    }
    CaseReport {
        case: "dependencies_not_enforced",
        passed: failures.is_empty(),
        metrics: m.finish(),
        evidence,
        failures,
    }
}

/// A1: D's "trigger fires twice" — the second publication for D is refused
/// with DuplicateResult (the admission layer's idempotent join: at most
/// one terminal result per node).
fn case_duplicate_admit_rejected() -> CaseReport {
    let mut failures: Vec<String> = Vec::new();
    let mut evidence: Vec<String> = Vec::new();
    let mut m = JsonObj::new();

    let result: Result<(), String> = (|| {
        let (run, exp) = make_ids("dupjoin")?;
        let caps = make_capabilities()?;
        let mut sched = make_scheduler()?;
        let d = make_node("node-d", &run, &exp, &caps, vec![])?;
        let d_id = d.node_id().clone();
        sched.admit(d).map_err(|e| format!("admit D: {e}"))?;
        let generation = sched
            .node(&d_id)
            .ok_or_else(|| "D vanished after admit".to_string())?
            .generation();
        // First "trigger": D publishes its terminal result.
        sched
            .publish_result(&d_id, generation, "digest-d-1", NodeState::Succeeded)
            .map_err(|e| format!("first publish: {e}"))?;
        // Second "trigger": same node, same generation — refused.
        match sched.publish_result(&d_id, generation, "digest-d-2", NodeState::Succeeded) {
            Err(ExperimentError::DuplicateResult { .. }) => {}
            Err(other) => {
                return Err(format!("second publish gave {other}, want DuplicateResult"));
            }
            Ok(()) => return Err("second publish succeeded: D has two terminal results".to_string()),
        }
        let digest = sched
            .node(&d_id)
            .and_then(|n| n.result_digest().map(str::to_string))
            .ok_or_else(|| "D has no result digest".to_string())?;
        if digest != "digest-d-1" {
            return Err(format!("committed digest changed to {digest}"));
        }
        m.boolean("first_publish_ok", true)
            .boolean("second_publish_duplicate_result", true)
            .text("committed_digest", &digest);
        evidence.push(
            "D's trigger fired twice: first publish committed digest-d-1, second refused with DuplicateResult"
                .to_string(),
        );
        evidence.push(
            "admission-layer idempotent join: at most one terminal result per node; the committed digest is immutable"
                .to_string(),
        );
        Ok(())
    })();
    if let Err(detail) = result {
        failures.push(detail);
    }
    CaseReport {
        case: "duplicate_admit_rejected",
        passed: failures.is_empty(),
        metrics: m.finish(),
        evidence,
        failures,
    }
}

/// A2: B and C "race" to read A's output — both see the same committed
/// digest. The ledger's published results are immutable and uniformly
/// visible; there is no torn read.
fn case_shared_output_read() -> CaseReport {
    let mut failures: Vec<String> = Vec::new();
    let mut evidence: Vec<String> = Vec::new();
    let mut m = JsonObj::new();

    let result: Result<(), String> = (|| {
        let (run, exp) = make_ids("shared")?;
        let caps = make_capabilities()?;
        let mut sched = make_scheduler()?;
        let a = make_node("node-a", &run, &exp, &caps, vec![])?;
        let a_id = a.node_id().clone();
        sched.admit(a).map_err(|e| format!("admit A: {e}"))?;
        let b = make_node("node-b", &run, &exp, &caps, vec![a_id.clone()])?;
        let c = make_node("node-c", &run, &exp, &caps, vec![a_id.clone()])?;
        sched.admit(b).map_err(|e| format!("admit B: {e}"))?;
        sched.admit(c).map_err(|e| format!("admit C: {e}"))?;
        // A "runs" (externally — the Scheduler executes nothing) and its
        // result is published to the ledger.
        let gen_a = sched
            .node(&a_id)
            .ok_or_else(|| "A vanished".to_string())?
            .generation();
        sched
            .publish_result(&a_id, gen_a, "digest-a-final", NodeState::Succeeded)
            .map_err(|e| format!("publish A: {e}"))?;
        // B and C both read A's committed output.
        let seen_b = sched
            .node(&a_id)
            .and_then(|n| n.result_digest().map(str::to_string))
            .ok_or_else(|| "B could not read A's digest".to_string())?;
        let seen_c = sched
            .node(&a_id)
            .and_then(|n| n.result_digest().map(str::to_string))
            .ok_or_else(|| "C could not read A's digest".to_string())?;
        if seen_b != "digest-a-final" || seen_c != "digest-a-final" {
            return Err(format!("torn read: B saw {seen_b}, C saw {seen_c}"));
        }
        m.text("b_saw", &seen_b).text("c_saw", &seen_c).boolean("identical", true);
        evidence.push(
            "B and C both read A's committed digest: identical (digest-a-final), no torn read".to_string(),
        );
        evidence.push(
            "published results are immutable and uniformly visible through the ledger".to_string(),
        );
        Ok(())
    })();
    if let Err(detail) = result {
        failures.push(detail);
    }
    CaseReport {
        case: "shared_output_read",
        passed: failures.is_empty(),
        metrics: m.finish(),
        evidence,
        failures,
    }
}

fn print_report(report: &CaseReport) {
    let mut out = String::from("{\"case\":");
    json_escape_into(&mut out, report.case);
    out.push_str(",\"passed\":");
    out.push_str(if report.passed { "true" } else { "false" });
    out.push_str(",\"metrics\":");
    out.push_str(&report.metrics);
    out.push_str(",\"evidence\":[");
    for (i, line) in report.evidence.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        json_escape_into(&mut out, line);
    }
    out.push_str("],\"failures\":[");
    for (i, line) in report.failures.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        json_escape_into(&mut out, line);
    }
    out.push_str("]}");
    println!("{out}");
}

fn main() {
    let case = std::env::args().nth(1).unwrap_or_default();
    let report = match case.as_str() {
        "diamond_admit" => case_diamond_admit(),
        "dependencies_not_enforced" => case_dependencies_not_enforced(),
        "duplicate_admit_rejected" => case_duplicate_admit_rejected(),
        "shared_output_read" => case_shared_output_read(),
        _ => CaseReport {
            case: "unknown",
            passed: false,
            metrics: JsonObj::new().finish(),
            evidence: Vec::new(),
            failures: vec![format!("unknown case '{case}'")],
        },
    };
    print_report(&report);
}
"##;
