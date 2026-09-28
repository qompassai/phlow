//! task-24: priority preemption (rust).
//!
//! Recon task: the design asks to "locate phlow's run priority /
//! preemption support (if none, document — the finding is the design
//! gap)". The driver proves the seam is ABSENT:
//!
//! * A case-insensitive source scan of every `crates/*/src` tree finds
//!   zero occurrences of `priorit` / `preempt`.
//! * `SchedulerLimits` (the scheduler's full tuning surface) has no
//!   priority field: `{ workers_max, queue_capacity, children_per_task_max,
//!   depth_max, task_deadline_ms, aggregate_tool_calls_max,
//!   aggregate_output_bytes_max }`.
//! * `NodeParams` (the complete per-node contract, constructed with an
//!   explicit field list in the scenario — compile-time proof of the field
//!   set) has no priority.
//! * The only interruption primitive is `Scheduler::cancel_run`, which is
//!   TERMINAL (cancelled nodes never resume) — preemption's resume half
//!   does not exist.
//!
//! The scenario binary compiles phlow's real `control_plane.rs` +
//! `error.rs` via `#[path]` (no mocks, no copies) and demonstrates the
//! behavioral absence: admission is priority-blind (FIFO arrival order),
//! cancellation is terminal with no preempt/resume path, and a cancelled
//! run id stays poisoned monotonically. The task verdict is `fail` with
//! `where = "seam"`: the design's preemption pass criteria (L yields to H
//! within a bound, L resumes with state intact, no starvation) have no
//! mechanism to evaluate against — an open design gap, not a driver error.
//! The integration tests assert the probe evidence is correct.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use std::fmt;
use std::io::{BufReader, Read as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Task id.
pub const ID: &str = "task-24";
/// Human-readable name.
pub const NAME: &str = "priority preemption";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the scenario binary runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "admission_is_priority_blind",
    "limits_have_no_priority",
    "cancellation_is_terminal",
    "cancelled_run_stays_poisoned",
];

/// Wall-clock bound for compiling the scenario binary.
const COMPILE_TIMEOUT: Duration = Duration::from_secs(90);
/// Wall-clock bound for one scenario case.
const CASE_TIMEOUT: Duration = Duration::from_secs(30);
/// Cap on captured child output (stdout+stderr), in bytes.
const OUTPUT_BYTES_MAX: usize = 1 << 20;
/// Poll interval while waiting on a child process.
const WAIT_POLL: Duration = Duration::from_millis(50);
/// Maximum source files the recon scan will read (bounded work).
const RECON_FILES_MAX: usize = 5000;
/// Maximum bytes read per source file during recon (bounded work).
const RECON_FILE_BYTES_MAX: u64 = 1 << 20;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-24 driver itself (not of the code under test).
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
            Self::Path { what, detail } => write!(f, "task-24: cannot read {what}: {detail}"),
            Self::ReconChanged { detail } => write!(f, "task-24: recon premise changed: {detail}"),
            Self::Toolchain { detail } => write!(f, "task-24: no Rust toolchain: {detail}"),
            Self::Spawn { what, detail } => write!(f, "task-24: cannot spawn {what}: {detail}"),
            Self::Timeout { what, timeout_ms } => {
                write!(f, "task-24: {what} exceeded {timeout_ms} ms and was killed")
            }
            Self::Output { what, detail } => {
                write!(f, "task-24: cannot read {what} output: {detail}")
            }
            Self::Compile { detail } => write!(f, "task-24: scenario build failed: {detail}"),
            Self::Verdict { case, detail } => {
                write!(f, "task-24: case '{case}' gave no JSON verdict: {detail}")
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

/// Collect `.rs` files under `dir`, bounded.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), DriverError> {
    if out.len() >= RECON_FILES_MAX {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|e| DriverError::Path {
        what: "crate src listing".to_string(),
        detail: format!("{}: {e}", dir.display()),
    })?;
    // Sort for determinism.
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|e| DriverError::Path {
                what: "crate src entry".to_string(),
                detail: e.to_string(),
            })?
            .path();
        paths.push(path);
    }
    paths.sort();
    for path in paths {
        if out.len() >= RECON_FILES_MAX {
            break;
        }
        if path.is_dir() {
            collect_rs_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Verify the task's scope premises against the live sources and return the
/// evidence lines. Fails closed: if any crate ever gains priority or
/// preemption machinery, the old absence claims must not silently pass.
fn recon() -> Result<Vec<String>, DriverError> {
    let root = workspace_root()?;
    let crates_dir = root.join("crates");
    // Structural premise: SchedulerLimits has no priority field and
    // NodeParams has no priority field (checked textually; the scenario
    // binary proves the same at compile time with explicit construction).
    let control_plane = read_file(
        &experiment_src_dir()?.join("control_plane.rs"),
        "control_plane.rs",
    )?;
    for needle in ["priority", "Priority", "preempt", "Preempt"] {
        if control_plane.contains(needle) {
            return Err(DriverError::ReconChanged {
                detail: format!(
                    "control_plane.rs now contains '{needle}'; priority/preemption may exist"
                ),
            });
        }
    }
    // Global premise: no priority/preemption machinery in ANY crate.
    let mut files: Vec<PathBuf> = Vec::new();
    let mut crate_dirs: Vec<PathBuf> = Vec::new();
    let entries = std::fs::read_dir(&crates_dir).map_err(|e| DriverError::Path {
        what: "crates listing".to_string(),
        detail: e.to_string(),
    })?;
    for entry in entries {
        let path = entry
            .map_err(|e| DriverError::Path {
                what: "crates entry".to_string(),
                detail: e.to_string(),
            })?
            .path();
        if path.join("src").is_dir() {
            // phlow-gauntlet is the test harness, not production: its own
            // task-24 driver mentions priority/preemption constantly.
            // Scanning it would self-falsify the absence claim.
            if path
                .file_name()
                .is_some_and(|name| name == "phlow-gauntlet")
            {
                continue;
            }
            crate_dirs.push(path);
        }
    }
    crate_dirs.sort();
    for dir in &crate_dirs {
        collect_rs_files(&dir.join("src"), &mut files)?;
    }
    let mut hits: Vec<String> = Vec::new();
    for path in &files {
        let capped = std::fs::File::open(path)
            .and_then(|f| {
                BufReader::new(f.take(RECON_FILE_BYTES_MAX))
                    .bytes()
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(|e| DriverError::Path {
                what: "recon source read".to_string(),
                detail: format!("{}: {e}", path.display()),
            })?;
        let text = String::from_utf8_lossy(&capped).to_lowercase();
        if text.contains("priorit") || text.contains("preempt") {
            hits.push(path.display().to_string());
        }
    }
    if !hits.is_empty() {
        return Err(DriverError::ReconChanged {
            detail: format!(
                "priority/preemption machinery may exist in: {}",
                hits.join(", ")
            ),
        });
    }
    Ok(vec![
        format!(
            "recon: scanned {} .rs files across {} non-gauntlet crates; zero hits for 'priorit'/'preempt' (case-insensitive)",
            files.len(),
            crate_dirs.len()
        ),
        "recon: control_plane.rs contains no 'priority'/'Priority'/'preempt'/'Preempt'".to_string(),
        "recon: SchedulerLimits fields are {workers_max, queue_capacity, children_per_task_max, depth_max, task_deadline_ms, aggregate_tool_calls_max, aggregate_output_bytes_max} — no priority".to_string(),
        "finding: phlow has no run-priority or preemption support — the task-24 seam is absent (design gap)".to_string(),
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
        .arg("task24_scenario")
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
/// but the designed priority/preemption seam is absent, so the design's
/// pass criteria have no mechanism to evaluate against. Recorded as an
/// open design gap.
fn seam_finding(evidence: Vec<String>) -> TaskFailure {
    TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: phlow has no run-priority or preemption support — zero \
              'priorit'/'preempt' hits across all non-gauntlet crate sources; SchedulerLimits and NodeParams \
              carry no priority field; the only interruption primitive (cancel_run) is terminal \
              with no resume path. The design's preemption pass criteria cannot be evaluated; \
              open design gap."
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
    let work_dir = ctx.work_dir.join("task-24");
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

/// The priority-preemption scenario, compiled with `rustc --edition=2024`.
/// `{{SRC_DIR}}` is replaced with the absolute path of
/// `crates/phlow-experiment/src`; the two `#[path]` modules then compile
/// phlow's actual scheduler sources into this binary. Those files import
/// only `std`, so no dependencies are needed.
///
/// Scope, stated plainly: phlow has no priority or preemption support.
/// These cases demonstrate the behavioral absence against the real
/// Scheduler: admission is priority-blind, the limits surface carries no
/// priority knob, and cancellation is terminal (preemption's resume half
/// does not exist).
const SCENARIO_TEMPLATE: &str = r##"//! task-24 priority scenario: probes phlow's real scheduler for
//! priority/preemption support and documents its absence.
//!
//! Compiled by `crates/phlow-gauntlet/src/tasks/task_24.rs` with `rustc`,
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

/// Build a node with the COMPLETE explicit field list of NodeParams.
/// This compiles only because the list is exhaustive: it is compile-time
/// proof that NodeParams carries no priority field — there is nowhere to
/// express "this node is high priority".
fn make_node(
    name: &str,
    run: &RunId,
    exp: &ExperimentId,
    caps: &CapabilitySet,
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
        dependency_ids: Vec::new(),
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
    Scheduler::new(SchedulerLimits::default()).map_err(|e| e.to_string())
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

/// V1: admission is priority-blind — a "low" node admitted before a "high"
/// node keeps its earlier queue position; nothing reorders by importance.
/// (Priority exists only in the node NAMES here; the Scheduler has no
/// priority input to honor.)
fn case_admission_is_priority_blind() -> CaseReport {
    let mut failures: Vec<String> = Vec::new();
    let mut evidence: Vec<String> = Vec::new();
    let mut m = JsonObj::new();

    let result: Result<(), String> = (|| {
        let (run, exp) = make_ids("blind")?;
        let caps = make_capabilities()?;
        let mut sched = make_scheduler()?;
        let low = make_node("node-low-priority", &run, &exp, &caps)?;
        let high = make_node("node-high-priority", &run, &exp, &caps)?;
        let low_id = low.node_id().clone();
        let high_id = high.node_id().clone();
        sched.admit(low).map_err(|e| format!("admit low: {e}"))?;
        sched.admit(high).map_err(|e| format!("admit high: {e}"))?;
        // Both admitted; arrival order preserved; no reordering primitive.
        if sched.queue_len() != 2 {
            return Err(format!("queue holds {}", sched.queue_len()));
        }
        let low_state = sched
            .node(&low_id)
            .ok_or_else(|| "low node vanished".to_string())?
            .state();
        let high_state = sched
            .node(&high_id)
            .ok_or_else(|| "high node vanished".to_string())?
            .state();
        if low_state != NodeState::Admitted || high_state != NodeState::Admitted {
            return Err(format!("states: low={low_state:?} high={high_state:?}"));
        }
        m.num("queue_len", 2)
            .text("low_state", low_state.name())
            .text("high_state", high_state.name())
            .boolean("reordered", false);
        evidence.push(
            "low-priority node admitted first, high-priority second: both Admitted, FIFO order kept"
                .to_string(),
        );
        evidence.push(
            "no API exists to express or honor priority — the names are inert labels".to_string(),
        );
        Ok(())
    })();
    if let Err(detail) = result {
        failures.push(detail);
    }
    CaseReport {
        case: "admission_is_priority_blind",
        passed: failures.is_empty(),
        metrics: m.finish(),
        evidence,
        failures,
    }
}

/// V2: the scheduler's full tuning surface carries no priority knob.
/// SchedulerLimits is Debug; its formatted field set is runtime evidence.
fn case_limits_have_no_priority() -> CaseReport {
    let mut failures: Vec<String> = Vec::new();
    let mut evidence: Vec<String> = Vec::new();
    let mut m = JsonObj::new();

    let result: Result<(), String> = (|| {
        let limits = SchedulerLimits::default();
        let debug = format!("{limits:?}");
        for needle in ["priorit", "preempt"] {
            if debug.to_lowercase().contains(needle) {
                return Err(format!("SchedulerLimits Debug mentions '{needle}': {debug}"));
            }
        }
        // The complete field set, asserted by name.
        for field in [
            "workers_max",
            "queue_capacity",
            "children_per_task_max",
            "depth_max",
            "task_deadline_ms",
            "aggregate_tool_calls_max",
            "aggregate_output_bytes_max",
        ] {
            if !debug.contains(field) {
                return Err(format!("SchedulerLimits missing expected field '{field}'"));
            }
        }
        m.text("limits_debug", &debug).boolean("priority_knob", false);
        evidence.push(format!("SchedulerLimits field set: {debug}"));
        evidence.push(
            "no priority, preemption, quantum, or aging knob exists on the scheduler's tuning surface"
                .to_string(),
        );
        Ok(())
    })();
    if let Err(detail) = result {
        failures.push(detail);
    }
    CaseReport {
        case: "limits_have_no_priority",
        passed: failures.is_empty(),
        metrics: m.finish(),
        evidence,
        failures,
    }
}

/// A1: the "high-priority arrives" scenario — the only interruption
/// primitive, cancel_run, is TERMINAL. There is no suspend/preempt: a
/// cancelled node never resumes, so L cannot "yield and resume".
fn case_cancellation_is_terminal() -> CaseReport {
    let mut failures: Vec<String> = Vec::new();
    let mut evidence: Vec<String> = Vec::new();
    let mut m = JsonObj::new();

    let result: Result<(), String> = (|| {
        let (run, exp) = make_ids("terminal")?;
        let caps = make_capabilities()?;
        let mut sched = make_scheduler()?;
        let l = make_node("node-L-low", &run, &exp, &caps)?;
        let l_id = l.node_id().clone();
        sched.admit(l).map_err(|e| format!("admit L: {e}"))?;
        // "H arrives and preempts L" — the closest real operation is
        // cancelling the whole run.
        let cancelled = sched.cancel_run(&run);
        if cancelled != 1 {
            return Err(format!("cancel_run moved {cancelled} nodes, want 1"));
        }
        let state = sched
            .node(&l_id)
            .ok_or_else(|| "L vanished".to_string())?
            .state();
        if state != NodeState::Cancelled {
            return Err(format!("L in state {state:?} after cancel_run, want Cancelled"));
        }
        if !state.is_terminal() {
            return Err("Cancelled is not terminal: resume might exist".to_string());
        }
        // No resume path: publishing for the cancelled node is refused, and
        // there is no API to un-cancel.
        let generation = sched
            .node(&l_id)
            .ok_or_else(|| "L vanished".to_string())?
            .generation();
        match sched.publish_result(&l_id, generation, "digest-l", NodeState::Succeeded) {
            Err(ExperimentError::RunCancelled { .. }) => {}
            Err(other) => return Err(format!("publish after cancel gave {other}")),
            Ok(()) => return Err("publish after cancel_run succeeded".to_string()),
        }
        m.num("cancelled_nodes", cancelled)
            .text("l_state", state.name())
            .boolean("l_terminal", true)
            .boolean("resume_path", false);
        evidence.push(
            "cancel_run moved L to Cancelled — a terminal state with no resume path".to_string(),
        );
        evidence.push(
            "publish for the cancelled node -> RunCancelled: preemption's 'L resumes with state intact' half does not exist"
                .to_string(),
        );
        Ok(())
    })();
    if let Err(detail) = result {
        failures.push(detail);
    }
    CaseReport {
        case: "cancellation_is_terminal",
        passed: failures.is_empty(),
        metrics: m.finish(),
        evidence,
        failures,
    }
}

/// A2: a cancelled run id stays poisoned monotonically — every later
/// operation under it is refused. No "un-preempt", no priority boost can
/// revive it; starvation/aging questions are moot without the mechanism.
fn case_cancelled_run_stays_poisoned() -> CaseReport {
    let mut failures: Vec<String> = Vec::new();
    let mut evidence: Vec<String> = Vec::new();
    let mut m = JsonObj::new();

    let result: Result<(), String> = (|| {
        let (run, exp) = make_ids("poisoned")?;
        let caps = make_capabilities()?;
        let mut sched = make_scheduler()?;
        let m_node = make_node("node-M-mid", &run, &exp, &caps)?;
        sched.admit(m_node).map_err(|e| format!("admit M: {e}"))?;
        sched.cancel_run(&run);
        if !sched.is_run_cancelled(&run) {
            return Err("run not marked cancelled".to_string());
        }
        // Every recovery-shaped operation is refused with RunCancelled.
        let late = make_node("node-late", &run, &exp, &caps)?;
        match sched.admit(late) {
            Err(ExperimentError::RunCancelled { .. }) => {}
            Err(other) => return Err(format!("late admit gave {other}, want RunCancelled")),
            Ok(()) => return Err("admit under a cancelled run succeeded".to_string()),
        }
        // A second cancel_run is a no-op count, not a state change.
        let again = sched.cancel_run(&run);
        if again != 0 {
            return Err(format!("second cancel_run moved {again} nodes"));
        }
        m.boolean("run_cancelled", true)
            .boolean("late_admit_refused", true)
            .num("second_cancel_moved", again);
        evidence.push(
            "after cancel_run: late admit -> RunCancelled; second cancel_run moved 0 nodes".to_string(),
        );
        evidence.push(
            "the run id is poisoned monotonically — no preempt/un-preempt cycle exists to starve or age within"
                .to_string(),
        );
        Ok(())
    })();
    if let Err(detail) = result {
        failures.push(detail);
    }
    CaseReport {
        case: "cancelled_run_stays_poisoned",
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
        "admission_is_priority_blind" => case_admission_is_priority_blind(),
        "limits_have_no_priority" => case_limits_have_no_priority(),
        "cancellation_is_terminal" => case_cancellation_is_terminal(),
        "cancelled_run_stays_poisoned" => case_cancelled_run_stays_poisoned(),
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
