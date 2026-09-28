//! task-13: scheduler overload (rust).
//!
//! Drives phlow's real scheduler — `phlow_experiment::control_plane::Scheduler`
//! — past its admission capacity and checks it degrades gracefully: bounded
//! queue, explicit rejection (never silent drops), no panics, no stuck
//! state, and full recovery once the load subsides.
//!
//! Scope, stated plainly: phlow @ 87c182d ships **no executing scheduler**.
//! `crates/phlow-experiment/src/lib.rs` says the crate enables "no ...
//! scheduler execution", and the `Scheduler` docs say it "spawns nothing,
//! runs nothing, and holds no threads". The scheduling code that exists is
//! the admission-control invariant core: [`Scheduler::admit`] enforces
//! `queue_capacity` and rejects overflow explicitly with
//! `ExperimentError::QueueFull`. This task overloads exactly that boundary —
//! the design doc's stated mechanism for task-13 ("submissions beyond
//! `QUEUE_CAPACITY` → bounded rejection; scheduler state stays consistent").
//! "More work than workers" is admission pressure here: `workers_max` is an
//! admission limit, not a thread pool.
//!
//! Mechanism: the driver compiles a small scenario binary with `rustc`,
//! embedding the crate's own `error.rs` and `control_plane.rs` via `#[path]`
//! (the real sources, read at compile time — no copies, no mocks, no new
//! dependencies), runs it once per case with a bounded wall-clock, and
//! parses its one-line JSON verdict. A live recon guard re-verifies the
//! no-executor premise on every run and fails closed if the sources change
//! shape underneath it.
//!
//! [`Scheduler::admit`]: https://github.com/qompassai/phlow (local path
//! `crates/phlow-experiment/src/control_plane.rs`)

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Task id.
pub const ID: &str = "task-13";
/// Human-readable name.
pub const NAME: &str = "scheduler overload";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Overload cases the scenario binary runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "normal_load",
    "burst_at_capacity",
    "overload_10x",
    "sustained_overload",
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

/// Failures of the task-13 driver itself (not of the scheduler under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A workspace path was missing or unreadable.
    Path {
        /// Which file was wanted.
        what: String,
        /// Path plus I/O detail.
        detail: String,
    },
    /// The no-executor recon premise no longer holds.
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
            Self::Path { what, detail } => write!(f, "task-13: cannot read {what}: {detail}"),
            Self::ReconChanged { detail } => write!(f, "task-13: recon premise changed: {detail}"),
            Self::Toolchain { detail } => write!(f, "task-13: no Rust toolchain: {detail}"),
            Self::Spawn { what, detail } => write!(f, "task-13: cannot spawn {what}: {detail}"),
            Self::Timeout { what, timeout_ms } => {
                write!(f, "task-13: {what} exceeded {timeout_ms} ms and was killed")
            }
            Self::Output { what, detail } => {
                write!(f, "task-13: cannot read {what} output: {detail}")
            }
            Self::Compile { detail } => write!(f, "task-13: scenario build failed: {detail}"),
            Self::Verdict { case, detail } => {
                write!(f, "task-13: case '{case}' gave no JSON verdict: {detail}")
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
// Recon: re-verify the no-executing-scheduler premise on every run
// ---------------------------------------------------------------------------

/// Verify the task's scope premise against the live sources and return the
/// evidence lines. Fails closed: if the crate ever gains an executor, the
/// old claims must not silently pass.
fn recon() -> Result<Vec<String>, DriverError> {
    let src = experiment_src_dir()?;
    let lib_rs = read_file(&src.join("lib.rs"), "phlow-experiment lib.rs")?;
    let control_plane = read_file(&src.join("control_plane.rs"), "control_plane.rs")?;
    if !lib_rs.contains("no scheduler execution") {
        return Err(DriverError::ReconChanged {
            detail: "phlow-experiment/src/lib.rs no longer states 'no scheduler execution'".into(),
        });
    }
    if !control_plane.contains("spawns nothing, runs nothing") {
        return Err(DriverError::ReconChanged {
            detail: "control_plane.rs Scheduler docs changed; re-verify the no-execution premise"
                .into(),
        });
    }
    // No executor may hide inside the scheduler crate: none of its sources
    // may reference an executor primitive.
    let mut checked: u32 = 0;
    let entries = std::fs::read_dir(&src).map_err(|e| DriverError::Path {
        what: "phlow-experiment src listing".to_string(),
        detail: e.to_string(),
    })?;
    for entry in entries {
        let path = entry
            .map_err(|e| DriverError::Path {
                what: "phlow-experiment src entry".to_string(),
                detail: e.to_string(),
            })?
            .path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let text = read_file(&path, "phlow-experiment source")?;
            if text.contains("tokio::") || text.contains("thread::spawn") {
                return Err(DriverError::ReconChanged {
                    detail: format!(
                        "{} references an executor primitive; premise changed",
                        path.display()
                    ),
                });
            }
            checked += 1;
        }
    }
    Ok(vec![
        "recon: crates/phlow-experiment/src/lib.rs states the crate enables 'no scheduler execution' (verified present)"
            .to_string(),
        "recon: control_plane.rs documents the Scheduler: 'spawns nothing, runs nothing, and holds no threads' (verified present)"
            .to_string(),
        format!(
            "recon: scanned {checked} source files in phlow-experiment/src; no tokio:: or thread::spawn executor primitives"
        ),
        "recon: phlow's only scheduler is control_plane::Scheduler admission control (bounded VecDeque, explicit QueueFull on overflow)"
            .to_string(),
        "scope: no executing scheduler exists to soak with threads; overload is applied at the admission boundary, per the task design ('submissions beyond QUEUE_CAPACITY -> bounded rejection')"
            .to_string(),
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
        .arg("task13_scenario")
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
    /// Measured numbers (queue depths, counts, latencies, RSS).
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

fn run_inner(ctx: &Ctx) -> Result<Vec<String>, TaskFailure> {
    let mut evidence = recon().map_err(|e| TaskFailure {
        where_: "recon".to_string(),
        how: e.to_string(),
        evidence: Vec::new(),
    })?;
    let work_dir = ctx.work_dir.join("task-13");
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
    let mut all_passed = true;
    let mut fail_where = String::new();
    let mut fail_how = String::new();
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
            all_passed = false;
            if fail_where.is_empty() {
                fail_where = case.to_string();
                fail_how = report.failures.join("; ");
                if fail_how.is_empty() {
                    fail_how = "case reported passed=false with no detail".to_string();
                }
            }
        }
    }
    if all_passed {
        Ok(evidence)
    } else {
        Err(TaskFailure {
            where_: fail_where,
            how: fail_how,
            evidence,
        })
    }
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

/// The overload scenario, compiled with `rustc --edition=2024`. `{{SRC_DIR}}`
/// is replaced with the absolute path of `crates/phlow-experiment/src`; the
/// two `#[path]` modules then compile phlow's actual scheduler sources into
/// this binary. Those files import only `std`, so no dependencies are needed.
const SCENARIO_TEMPLATE: &str = r##"//! task-13 overload scenario: drives phlow's real scheduler admission
//! control through normal, burst, and overload cases.
//!
//! Compiled by `crates/phlow-gauntlet/src/tasks/task_13.rs` with `rustc`,
//! embedding the crate's own sources via `#[path]` — the real
//! `phlow_experiment::control_plane::Scheduler`, read at compile time.
//! No mocks, no copies.
//!
//! Scope, stated plainly: phlow ships no executing scheduler. The crate's
//! own docs say it enables "no scheduler execution", and the `Scheduler`
//! docs say it "spawns nothing, runs nothing, and holds no threads". The
//! scheduling code that exists is admission control: `Scheduler::admit`
//! enforces a bounded queue and rejects overflow explicitly with
//! `ExperimentError::QueueFull`. This scenario overloads exactly that
//! boundary: bursts, a 10x submission storm, and sustained overload
//! followed by a drain, checking graceful degradation throughout.

#[path = "{{SRC_DIR}}/error.rs"]
mod error;
#[path = "{{SRC_DIR}}/control_plane.rs"]
mod control_plane;

use control_plane::{
    CapabilitySet, ExperimentId, NodeId, NodeParams, NodeState, RunId, Scheduler, SchedulerLimits,
    SchedulerNode, WorkerRole, QUEUE_CAPACITY_DEFAULT,
};
use error::ExperimentError;
use std::time::Instant;

/// Nodes admitted under normal load (half the default queue capacity).
const NORMAL_LOAD_NODES: usize = 8;
/// Overload factor for the adversarial storm: 10x queue capacity.
const OVERLOAD_FACTOR: usize = 10;
/// Rounds of sustained overload before the drain.
const SUSTAINED_ROUNDS: usize = 5;
/// Submissions per sustained round (2x queue capacity).
const SUSTAINED_WAVE: usize = 32;

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
    seq: usize,
    run: &RunId,
    exp: &ExperimentId,
    caps: &CapabilitySet,
) -> Result<SchedulerNode, String> {
    let node_id = NodeId::new(&format!("node-{seq:05}")).map_err(|e| e.to_string())?;
    SchedulerNode::new(NodeParams {
        run_id: run.clone(),
        experiment_id: exp.clone(),
        baseline_revision: "rev-1".to_string(),
        workspace_snapshot: "snap-1".to_string(),
        node_id,
        parent_node_id: None,
        role: WorkerRole::Implementer,
        capabilities: caps.clone(),
        input_digest: format!("input-{seq}"),
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

fn make_scheduler(queue_capacity: usize) -> Result<Scheduler, String> {
    let limits = SchedulerLimits {
        queue_capacity,
        ..Default::default()
    };
    Scheduler::new(limits).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Load drivers
// ---------------------------------------------------------------------------

/// Outcome of one admission wave.
struct WaveOutcome {
    admitted: Vec<NodeId>,
    queue_full: usize,
    other_errors: Vec<String>,
    admit_ns_total: u128,
    queue_peak: usize,
}

/// Submit `count` nodes; classify every outcome. Nothing is silent: each
/// submission is either admitted or carries an explicit typed error.
fn admit_wave(
    sched: &mut Scheduler,
    run: &RunId,
    exp: &ExperimentId,
    caps: &CapabilitySet,
    start_seq: usize,
    count: usize,
) -> Result<WaveOutcome, String> {
    let mut outcome = WaveOutcome {
        admitted: Vec::new(),
        queue_full: 0,
        other_errors: Vec::new(),
        admit_ns_total: 0,
        queue_peak: sched.queue_len(),
    };
    for i in 0..count {
        let node = make_node(start_seq + i, run, exp, caps)?;
        let id = node.node_id().clone();
        let started = Instant::now();
        match sched.admit(node) {
            Ok(()) => outcome.admitted.push(id),
            Err(ExperimentError::QueueFull { .. }) => outcome.queue_full += 1,
            Err(other) => outcome.other_errors.push(other.to_string()),
        }
        outcome.admit_ns_total += started.elapsed().as_nanos();
        outcome.queue_peak = outcome.queue_peak.max(sched.queue_len());
    }
    Ok(outcome)
}

/// Publish terminal results for every admitted id: the queue drains and the
/// scheduler stays consistent.
fn drain_queue(sched: &mut Scheduler, ids: &[NodeId]) -> Result<usize, String> {
    let mut published = 0;
    for id in ids {
        let generation = sched
            .node(id)
            .ok_or_else(|| format!("admitted node {} vanished before drain", id.as_str()))?
            .generation();
        sched
            .publish_result(
                id,
                generation,
                &format!("digest-{}", id.as_str()),
                NodeState::Succeeded,
            )
            .map_err(|e| e.to_string())?;
        published += 1;
    }
    Ok(published)
}

/// Resident set size in KiB from /proc/self/statm (Linux). `None` when the
/// proc file is unavailable; reported honestly, never invented.
fn rss_kib() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/statm").ok()?;
    let resident_pages: u64 = text.split_whitespace().nth(1)?.parse().ok()?;
    Some(resident_pages * 4) // 4 KiB pages on x86_64 Linux
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

/// V1: normal load — half the queue capacity admits and completes fully.
fn case_normal_load() -> CaseReport {
    let mut failures: Vec<String> = Vec::new();
    let mut evidence: Vec<String> = Vec::new();
    let mut m = JsonObj::new();
    let capacity = QUEUE_CAPACITY_DEFAULT;
    m.num("queue_capacity", capacity as u64);

    let result: Result<(), String> = (|| {
        let (run, exp) = make_ids("normal")?;
        let caps = make_capabilities()?;
        let mut sched = make_scheduler(capacity)?;
        let wave = admit_wave(&mut sched, &run, &exp, &caps, 0, NORMAL_LOAD_NODES)?;
        if wave.admitted.len() != NORMAL_LOAD_NODES {
            return Err(format!(
                "admitted {} of {NORMAL_LOAD_NODES}",
                wave.admitted.len()
            ));
        }
        if wave.queue_full != 0 || !wave.other_errors.is_empty() {
            return Err("unexpected rejections under normal load".to_string());
        }
        let avg_ns = wave.admit_ns_total / NORMAL_LOAD_NODES as u128;
        m.num("admitted", NORMAL_LOAD_NODES as u64)
            .num("admit_ns_avg", avg_ns as u64)
            .num("queue_peak", wave.queue_peak as u64);
        evidence.push(format!(
            "{NORMAL_LOAD_NODES} nodes admitted, avg admit latency {avg_ns} ns, queue peak {}",
            wave.queue_peak
        ));
        let published = drain_queue(&mut sched, &wave.admitted)?;
        if sched.queue_len() != 0 {
            return Err(format!("queue did not drain: {}", sched.queue_len()));
        }
        if sched.published_count() != NORMAL_LOAD_NODES {
            return Err(format!("published {}", sched.published_count()));
        }
        m.num("published", published as u64).num("queue_final", 0);
        evidence.push(format!(
            "all {published} results published exactly once; queue drained to 0"
        ));
        Ok(())
    })();
    if let Err(detail) = result {
        failures.push(detail);
    }
    CaseReport {
        case: "normal_load",
        passed: failures.is_empty(),
        metrics: m.finish(),
        evidence,
        failures,
    }
}

/// V2: burst at exactly capacity — the full queue is absorbed, then drains.
fn case_burst_at_capacity() -> CaseReport {
    let mut failures: Vec<String> = Vec::new();
    let mut evidence: Vec<String> = Vec::new();
    let mut m = JsonObj::new();
    let capacity = QUEUE_CAPACITY_DEFAULT;
    m.num("queue_capacity", capacity as u64);

    let result: Result<(), String> = (|| {
        let (run, exp) = make_ids("burst")?;
        let caps = make_capabilities()?;
        let mut sched = make_scheduler(capacity)?;
        let wave = admit_wave(&mut sched, &run, &exp, &caps, 0, capacity)?;
        if wave.admitted.len() != capacity {
            return Err(format!(
                "burst of {capacity} admitted only {}",
                wave.admitted.len()
            ));
        }
        if wave.queue_full != 0 || !wave.other_errors.is_empty() {
            return Err("burst at exactly capacity saw rejections".to_string());
        }
        if sched.queue_len() != capacity {
            return Err(format!("queue holds {}", sched.queue_len()));
        }
        let avg_ns = wave.admit_ns_total / capacity as u128;
        m.num("admitted", capacity as u64)
            .num("admit_ns_avg", avg_ns as u64)
            .num("queue_peak", wave.queue_peak as u64);
        evidence.push(format!(
            "burst of {capacity} fully absorbed; queue at capacity, avg admit {avg_ns} ns"
        ));
        let published = drain_queue(&mut sched, &wave.admitted)?;
        if sched.queue_len() != 0 || sched.published_count() != capacity {
            return Err("drain after burst left residue".to_string());
        }
        m.num("published", published as u64).num("queue_final", 0);
        evidence.push("burst drained cleanly; all results published once".to_string());
        Ok(())
    })();
    if let Err(detail) = result {
        failures.push(detail);
    }
    CaseReport {
        case: "burst_at_capacity",
        passed: failures.is_empty(),
        metrics: m.finish(),
        evidence,
        failures,
    }
}

/// A1: 10x overload — excess submissions are rejected with explicit
/// `QueueFull` errors (never silently dropped); the queue stays bounded.
fn case_overload_10x() -> CaseReport {
    let mut failures: Vec<String> = Vec::new();
    let mut evidence: Vec<String> = Vec::new();
    let mut m = JsonObj::new();
    let capacity = QUEUE_CAPACITY_DEFAULT;
    let total = capacity * OVERLOAD_FACTOR;
    m.num("queue_capacity", capacity as u64)
        .num("submitted", total as u64)
        .num("overload_factor", OVERLOAD_FACTOR as u64);

    let result: Result<(), String> = (|| {
        let (run, exp) = make_ids("storm")?;
        let caps = make_capabilities()?;
        let mut sched = make_scheduler(capacity)?;
        let rss_before = rss_kib();
        let wave = admit_wave(&mut sched, &run, &exp, &caps, 0, total)?;
        let rss_after = rss_kib();
        if wave.admitted.len() != capacity {
            return Err(format!(
                "storm admitted {} nodes, want exactly {capacity}",
                wave.admitted.len()
            ));
        }
        if wave.queue_full != total - capacity {
            return Err(format!(
                "storm rejected {} with QueueFull, want {}",
                wave.queue_full,
                total - capacity
            ));
        }
        if !wave.other_errors.is_empty() {
            return Err(format!(
                "unexpected error kinds under overload: {:?}",
                wave.other_errors
            ));
        }
        // Conservation: every submission is accounted for — admitted or
        // explicitly rejected. Nothing vanished silently.
        let accounted = wave.admitted.len() + wave.queue_full + wave.other_errors.len();
        if accounted != total {
            return Err(format!("accounted {accounted} of {total} submissions"));
        }
        if wave.queue_peak > capacity {
            return Err(format!("queue peak {} exceeded capacity", wave.queue_peak));
        }
        let avg_ns = wave.admit_ns_total / total as u128;
        m.num("admitted", wave.admitted.len() as u64)
            .num("rejected_queue_full", wave.queue_full as u64)
            .num("rejected_other", wave.other_errors.len() as u64)
            .num("accounted", accounted as u64)
            .num("queue_peak", wave.queue_peak as u64)
            .num("admit_ns_avg", avg_ns as u64);
        if let (Some(before), Some(after)) = (rss_before, rss_after) {
            m.num("rss_kib_before", before).num("rss_kib_after", after);
            evidence.push(format!(
                "RSS {before} KiB -> {after} KiB across the {total}-submission storm (queue bounded at {capacity})"
            ));
        } else {
            m.boolean("rss_unavailable", true);
            evidence.push("/proc/self/statm unavailable; RSS not measured".to_string());
        }
        evidence.push(format!(
            "{total} submissions: {capacity} admitted, {} rejected with explicit QueueFull, 0 other errors, 0 unaccounted",
            wave.queue_full
        ));
        // The admitted set is intact: every admitted id still resolves and
        // is still in the Admitted state — no silent loss, no corruption.
        for id in &wave.admitted {
            let node = sched
                .node(id)
                .ok_or_else(|| format!("admitted node {} lost", id.as_str()))?;
            if node.state() != NodeState::Admitted {
                return Err(format!(
                    "admitted node {} in state {:?}, want Admitted",
                    id.as_str(),
                    node.state()
                ));
            }
        }
        evidence.push(format!(
            "all {} admitted nodes retrievable and still Admitted after the storm",
            wave.admitted.len()
        ));
        let published = drain_queue(&mut sched, &wave.admitted)?;
        if sched.queue_len() != 0 {
            return Err(format!("queue did not drain after storm: {}", sched.queue_len()));
        }
        m.num("published", published as u64).num("queue_final", 0);
        evidence.push("storm drained; scheduler consistent after 10x overload".to_string());
        Ok(())
    })();
    if let Err(detail) = result {
        failures.push(detail);
    }
    CaseReport {
        case: "overload_10x",
        passed: failures.is_empty(),
        metrics: m.finish(),
        evidence,
        failures,
    }
}

/// A2: sustained overload, then the load subsides — the scheduler must
/// drain fully and stay responsive (admit + publish still work).
fn case_sustained_overload() -> CaseReport {
    let mut failures: Vec<String> = Vec::new();
    let mut evidence: Vec<String> = Vec::new();
    let mut m = JsonObj::new();
    let capacity = QUEUE_CAPACITY_DEFAULT;
    m.num("queue_capacity", capacity as u64)
        .num("rounds", SUSTAINED_ROUNDS as u64)
        .num("wave_per_round", SUSTAINED_WAVE as u64);

    let result: Result<(), String> = (|| {
        let (run, exp) = make_ids("sustained")?;
        let caps = make_capabilities()?;
        let mut sched = make_scheduler(capacity)?;
        let mut seq = 0usize;
        let mut total_admitted = 0usize;
        let mut total_rejected = 0usize;
        let mut peak = 0usize;
        let started = Instant::now();
        for round in 0..SUSTAINED_ROUNDS {
            let wave = admit_wave(&mut sched, &run, &exp, &caps, seq, SUSTAINED_WAVE)?;
            seq += SUSTAINED_WAVE;
            total_admitted += wave.admitted.len();
            total_rejected += wave.queue_full;
            peak = peak.max(wave.queue_peak);
            if !wave.other_errors.is_empty() {
                return Err(format!(
                    "round {round}: unexpected errors: {:?}",
                    wave.other_errors
                ));
            }
            // The "executor" drains the queue each round; overload persists
            // across rounds while the queue keeps refilling.
            drain_queue(&mut sched, &wave.admitted)?;
            if sched.queue_len() != 0 {
                return Err(format!(
                    "round {round}: queue stuck at {}",
                    sched.queue_len()
                ));
            }
        }
        let elapsed_ms = started.elapsed().as_millis();
        m.num("total_admitted", total_admitted as u64)
            .num("total_rejected_queue_full", total_rejected as u64)
            .num("queue_peak", peak as u64)
            .num("elapsed_ms", elapsed_ms as u64);
        evidence.push(format!(
            "{SUSTAINED_ROUNDS} rounds of {SUSTAINED_WAVE}-submission waves: {total_admitted} admitted, {total_rejected} explicitly rejected, peak queue {peak}, {elapsed_ms} ms total"
        ));
        // The load subsides: a probe submission must be admitted and
        // published normally — the scheduler is responsive, nothing stuck.
        let probe = make_node(seq, &run, &exp, &caps)?;
        let probe_id = probe.node_id().clone();
        sched.admit(probe).map_err(|e| format!("probe admit failed: {e}"))?;
        sched
            .publish_result(&probe_id, 0, "digest-probe", NodeState::Succeeded)
            .map_err(|e| format!("probe publish failed: {e}"))?;
        if sched.queue_len() != 0 {
            return Err(format!("queue not empty after probe: {}", sched.queue_len()));
        }
        m.num("queue_final", 0).boolean("probe_ok", true);
        evidence.push(
            "after the storm subsided: probe node admitted and published; queue empty; scheduler fully responsive"
                .to_string(),
        );
        Ok(())
    })();
    if let Err(detail) = result {
        failures.push(detail);
    }
    CaseReport {
        case: "sustained_overload",
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
        "normal_load" => case_normal_load(),
        "burst_at_capacity" => case_burst_at_capacity(),
        "overload_10x" => case_overload_10x(),
        "sustained_overload" => case_sustained_overload(),
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
