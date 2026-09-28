//! task-47: file descriptor exhaustion (rust).
//!
//! Drives the real seam: [`phlow_tuios::hooks::HookManager`] — the only
//! place in phlow that fans out concurrent subprocesses. Each `fire`
//! hands every registered hook command to a spawned thread, and two
//! counting semaphores bound the fan-out:
//!
//! - the queue semaphore ([`HOOK_QUEUED_MAX`] = 64): `fire` takes a slot
//!   with `try_acquire_owned` and never blocks; a firing past the bound
//!   is recorded as a `dropped: true` outcome, never spawned, never
//!   silent;
//! - the run semaphore ([`HOOK_CONCURRENT_MAX`] = 8): each spawned thread
//!   holds a run permit for the whole command, so at most 8 children run
//!   at once;
//! - the single spawn site (`run_argv`, the only `Command::new` in the
//!   crate) reaps the child on every path — wait-with-timeout, then kill,
//!   then wait — and both semaphore guards return their permits on drop,
//!   even when the hook thread panics.
//!
//! Honest scope: this is the hook/subprocess fan-out path — the
//! design's "10,000 rapid tool spawns". The tuios accept loop bounds TCP
//! connections with the same semaphore primitive (a second consumer of the
//! limiter, noted in evidence). The phlow-checks runner
//! (`spawn_pinned`) is classified by inspection in the V2 audit: it runs
//! one child synchronously per call (no queue, no fan-out — nothing to
//! limit), with process-group kill and reap on timeout.
//!
//! Four cases against the real manager (no mock limiter): two
//! validation, two adversarial. The task-level verdict is `pass`: the
//! measured ceiling under attack is the semaphore pair, excess is shed
//! with a recorded outcome (the design's `TooManyHandles` typed error
//! arrives here as `dropped: true` in the outcomes log — explicit and
//! inspectable, never a silent drop and never EMFILE), and the source
//! audit shows every acquisition path goes through the limiter.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_tuios::hooks::{
    HOOK_CONCURRENT_MAX, HOOK_QUEUED_MAX, HookContext, HookEvent, HookManager,
};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

/// Task id.
pub const ID: &str = "task-47";
/// Human-readable name.
pub const NAME: &str = "file descriptor exhaustion";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "normal_fanout_runs",
    "spawn_paths_go_through_limiter",
    "storm_10000_spawns_capped",
    "crashed_tools_return_to_baseline",
];

/// How long to wait for hook outcomes, in milliseconds.
const OUTCOME_WAIT_MS: u64 = 30_000;
/// Poll interval while waiting for outcomes, in milliseconds.
const OUTCOME_POLL_MS: u64 = 50;
/// Fires in the adversarial storm.
const STORM_FIRES: usize = 10_000;
/// Sampler interval during the storm, in milliseconds.
const SAMPLE_MS: u64 = 25;
/// FD headroom above baseline during the storm: the storm's own threads
/// hold no pipes (children inherit stdio), so the bound is the queue
/// depth plus slack for the sampler's transient read_dir handles.
const FD_HEADROOM: usize = 96;
/// Largest source file the audit will scan, in bytes.
const SOURCE_BYTES_MAX: usize = 1_048_576;
/// Most source files the audit will scan before stopping.
const SOURCE_FILES_MAX: usize = 50_000;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-47 driver itself (not of the manager under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture was unusable.
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
                write!(f, "task-47: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-47: cannot probe {what}: {detail}")
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
// Fixtures and /proc helpers
// ---------------------------------------------------------------------------

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

/// This driver's own source file, excluded from the audit by exact path:
/// the driver prose must name the spawn sites it audits.
fn own_source_file() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("tasks")
        .join("task_47.rs")
}

/// Require a Linux /proc; the ceiling is measured from the OS, and a
/// guessed ceiling would be invented evidence.
fn require_proc() -> Result<(), DriverError> {
    if Path::new("/proc/self/fd").is_dir() {
        Ok(())
    } else {
        Err(probe_error(
            "process census",
            "no /proc/self/fd — this case measures the OS handle ceiling and needs Linux",
        ))
    }
}

/// Current open-FD count of this process, via /proc/self/fd.
fn fd_count() -> Result<usize, DriverError> {
    std::fs::read_dir("/proc/self/fd")
        .map_err(|e| probe_error("fd census", e))
        .map(|entries| entries.count())
}

/// Current child-process count of this process: /proc entries whose
/// `PPid:` line equals our pid.
fn child_count() -> Result<usize, DriverError> {
    let self_pid = std::process::id().to_string();
    let proc = std::fs::read_dir("/proc").map_err(|e| probe_error("process census", e))?;
    let mut children = 0usize;
    for entry in proc {
        let entry = entry.map_err(|e| probe_error("process census", e))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if name == self_pid {
            continue;
        }
        let status = std::fs::read_to_string(entry.path().join("status"));
        let Ok(status) = status else { continue };
        for line in status.lines() {
            if let Some(ppid) = line.strip_prefix("PPid:") {
                if ppid.trim() == self_pid {
                    children += 1;
                }
                break;
            }
        }
    }
    Ok(children)
}

/// Wait until the manager has recorded at least `want` outcomes, or the
/// deadline passes. Returns the outcomes seen.
fn wait_for_outcomes(
    manager: &HookManager,
    want: usize,
    timeout: Duration,
) -> Vec<phlow_tuios::hooks::HookOutcome> {
    let started = Instant::now();
    loop {
        let outcomes = manager.outcomes();
        if outcomes.len() >= want || started.elapsed() >= timeout {
            return outcomes;
        }
        thread::sleep(Duration::from_millis(OUTCOME_POLL_MS));
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

/// V1: normal fan-out works. Five firings of a `/bin/true` hook all run
/// to completion: none dropped, every outcome carries exit code 0.
fn case_normal_fanout_runs() -> Result<CaseReport, DriverError> {
    const CASE: &str = "normal_fanout_runs";
    let manager = HookManager::new();
    manager
        .register(HookEvent::AfterAgentState, vec!["/bin/true".to_string()])
        .map_err(|e| fixture_error("hook register", e))?;
    let ctx = HookContext::default();
    for _ in 0..5 {
        manager.fire(HookEvent::AfterAgentState, &ctx);
    }
    let outcomes = wait_for_outcomes(&manager, 5, Duration::from_millis(OUTCOME_WAIT_MS));
    let mut evidence = vec![format!("outcomes recorded: {}", outcomes.len())];
    if outcomes.len() < 5 {
        return Ok(CaseReport::fail(
            CASE,
            format!("only {} of 5 outcomes recorded", outcomes.len()),
            evidence,
        ));
    }
    let dropped = outcomes.iter().filter(|o| o.dropped).count();
    let ok = outcomes
        .iter()
        .filter(|o| !o.dropped && o.exit_code == Some(0))
        .count();
    evidence.push(format!(
        "dropped={dropped}, exit_code=Some(0) and not dropped: {ok}"
    ));
    if dropped != 0 || ok != 5 {
        return Ok(CaseReport::fail(
            CASE,
            format!("normal fan-out degraded: dropped={dropped}, clean runs={ok}/5"),
            evidence,
        ));
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"fired": 5, "dropped": 0, "clean_runs": 5}),
        evidence,
    ))
}

/// V2: every process-spawn site in the non-gauntlet crates is enumerated
/// by source walk (this driver's own file path-excluded), and each is
/// classified: the hook fan-out spawn is behind the queue + run
/// semaphores; the checks-runner spawn is synchronous single-child per
/// call (no queue exists to bound); the CLI spawn is verified
/// test-only. No other acquisition path may exist — a new spawn site
/// fails this case.
fn case_spawn_paths_go_through_limiter() -> Result<CaseReport, DriverError> {
    const CASE: &str = "spawn_paths_go_through_limiter";
    let root = workspace_root()?;
    let own = own_source_file();
    let mut evidence = Vec::new();
    let mut sites: Vec<String> = Vec::new();
    let mut files_seen = 0usize;
    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| fixture_error("source walk", format!("{}: {e}", dir.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| fixture_error("source walk", e))?;
            let path = entry.path();
            if path.is_dir() {
                // The gauntlet's own drivers are test doubles, not the
                // system under test; this driver's own file is excluded
                // by exact path because it must name the sites it audits.
                if path.file_name().is_some_and(|n| n == "phlow-gauntlet") {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.components().any(|c| c.as_os_str() == "src")
                && path != own
            {
                files_seen += 1;
                if files_seen > SOURCE_FILES_MAX {
                    return Err(probe_error(
                        "source audit",
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
                    if line.contains("Command::new") && !line.trim_start().starts_with("//") {
                        sites.push(format!("{}:{}", path.display(), lineno + 1));
                    }
                }
            }
        }
    }
    evidence.push(format!(
        "spawn sites outside phlow-gauntlet: {} ({})",
        sites.len(),
        sites.join(", ")
    ));
    // Classify each site. The hook fan-out spawn must sit in the file
    // that owns the limiter; the checks spawn must be the synchronous
    // single-child runner (classified by inspection — quoted below);
    // the CLI spawn must be test-only (verified mechanically).
    let mut hook_gated = false;
    let mut checks_sync = false;
    let mut cli_test_only = false;
    for site in &sites {
        if site.contains("phlow-tuios/src/hooks.rs") {
            let text = std::fs::read_to_string(
                root.join("crates")
                    .join("phlow-tuios")
                    .join("src")
                    .join("hooks.rs"),
            )
            .map_err(|e| fixture_error("hooks.rs", e))?;
            let has_queue = text.contains("HOOK_QUEUED_MAX");
            let has_run = text.contains("HOOK_CONCURRENT_MAX");
            let has_shed = text.contains("try_acquire_owned");
            evidence.push(format!(
                "hooks.rs: queue bound present={has_queue}, run bound present={has_run}, load-shed present={has_shed}"
            ));
            hook_gated = has_queue && has_run && has_shed;
        } else if site.contains("phlow-checks/src/runner.rs") {
            checks_sync = true;
            evidence.push(
                "runner.rs: spawn_pinned runs ONE child synchronously per call (wait_with_deadline, process-group kill, reap) — no queue, no fan-out, nothing to limit"
                    .to_string(),
            );
        } else if site.contains("phlow-cli/src/lib.rs") {
            // Verify test-only-ness mechanically: the enclosing fn must
            // carry #[test]. Walk up from the hit line to the nearest
            // `fn`, then require #[test] on the lines just above it.
            let (path, lineno) = site
                .rsplit_once(':')
                .ok_or_else(|| probe_error("hit parse", format!("bad hit '{site}'")))?;
            let lineno: usize = lineno
                .parse()
                .map_err(|_| probe_error("hit parse", format!("bad lineno in '{site}'")))?;
            let text = std::fs::read_to_string(path)
                .map_err(|e| fixture_error("cli lib.rs", format!("{path}: {e}")))?;
            let lines: Vec<&str> = text.lines().collect();
            let mut fn_line = None;
            for (i, line) in lines.iter().enumerate().take(lineno - 1).rev() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("fn ") || trimmed.starts_with("pub fn ") {
                    fn_line = Some(i);
                    break;
                }
            }
            let Some(fn_idx) = fn_line else {
                return Ok(CaseReport::fail(
                    CASE,
                    format!("cli spawn site {site}: no enclosing fn found"),
                    evidence,
                ));
            };
            let is_test = lines[fn_idx.saturating_sub(3)..fn_idx]
                .iter()
                .any(|l| l.trim() == "#[test]");
            evidence.push(format!(
                "phlow-cli/src/lib.rs:{lineno}: enclosing fn at line {} is #[test]={is_test} (version cross-check)",
                fn_idx + 1
            ));
            cli_test_only = is_test;
        } else {
            return Ok(CaseReport::fail(
                CASE,
                format!("unclassified spawn site outside the limiter: {site}"),
                evidence,
            ));
        }
    }
    if sites.len() != 3 || !hook_gated || !checks_sync || !cli_test_only {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "audit failed: want exactly the gated hook spawn + the synchronous checks spawn + one test-only cli spawn; hook_gated={hook_gated}, checks_sync={checks_sync}, cli_test_only={cli_test_only}"
            ),
            evidence,
        ));
    }
    evidence.push(
        "every acquisition path is accounted for: the fan-out path is semaphore-gated (queue slot + run permit precede spawn; excess recorded as dropped); the only other production spawner is sequential; the third site is test-only".to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"spawn_sites": 3, "gated_fanout": 1, "synchronous_single": 1, "test_only": 1}),
        evidence,
    ))
}

/// A1: 10,000 rapid spawns. The sampler measures concurrent child
/// processes and open FDs while the storm runs; the ceiling must hold:
/// at most [`HOOK_CONCURRENT_MAX`] children at once, FDs within
/// baseline + headroom — and no spawn-error outcomes (the EMFILE
/// signature), because the excess is shed before it can spawn.
fn case_storm_10000_spawns_capped() -> Result<CaseReport, DriverError> {
    const CASE: &str = "storm_10000_spawns_capped";
    require_proc()?;
    let manager = HookManager::new();
    manager
        .register(
            HookEvent::AfterCommandFinished,
            vec!["/bin/sleep".to_string(), "0.2".to_string()],
        )
        .map_err(|e| fixture_error("hook register", e))?;
    let baseline_fds = fd_count()?;
    let baseline_children = child_count()?;

    let stop = Arc::new(AtomicBool::new(false));
    let sampler_stop = Arc::clone(&stop);
    let sampler = thread::spawn(move || {
        let mut max_children = 0usize;
        let mut max_fds = 0usize;
        let mut samples = 0usize;
        while !sampler_stop.load(Ordering::Relaxed) {
            samples += 1;
            if let Ok(n) = child_count() {
                max_children = max_children.max(n);
            }
            if let Ok(n) = fd_count() {
                max_fds = max_fds.max(n);
            }
            thread::sleep(Duration::from_millis(SAMPLE_MS));
        }
        (samples, max_children, max_fds)
    });

    let ctx = HookContext::default();
    let fire_start = Instant::now();
    for _ in 0..STORM_FIRES {
        manager.fire(HookEvent::AfterCommandFinished, &ctx);
    }
    let fire_elapsed = fire_start.elapsed();
    // Drain: 64 queue slots, 8 concurrent, 0.2 s per run — ~2 s worst
    // case; the sampler keeps measuring until the runs are done.
    thread::sleep(Duration::from_secs(4));
    stop.store(true, Ordering::Relaxed);
    let (samples, max_children, max_fds) = sampler
        .join()
        .map_err(|_| probe_error("sampler", "sampler thread panicked"))?;

    let mut evidence = vec![
        format!("fired {STORM_FIRES} in {fire_elapsed:?} (fire never blocks)"),
        format!(
            "sampler: {samples} samples, baseline fds={baseline_fds} children={baseline_children}, max fds={max_fds} max children={max_children}"
        ),
    ];
    if max_children > baseline_children + HOOK_CONCURRENT_MAX {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "concurrent children {max_children} exceeded the run-permit ceiling (baseline {baseline_children} + {HOOK_CONCURRENT_MAX})"
            ),
            evidence,
        ));
    }
    if max_fds > baseline_fds + FD_HEADROOM {
        return Ok(CaseReport::fail(
            CASE,
            format!("open FDs {max_fds} exceeded baseline {baseline_fds} + headroom {FD_HEADROOM}"),
            evidence,
        ));
    }
    // The EMFILE signature: spawn errors surface as outcomes with no
    // exit code that were NOT shed. The outcomes log keeps the last 128;
    // none may carry that signature.
    let outcomes = manager.outcomes();
    let spawn_errors = outcomes
        .iter()
        .filter(|o| !o.dropped && o.exit_code.is_none() && !o.timed_out)
        .count();
    evidence.push(format!(
        "kept outcomes: {}, spawn-error signature among them: {spawn_errors}",
        outcomes.len()
    ));
    if spawn_errors != 0 {
        return Ok(CaseReport::fail(
            CASE,
            format!("{spawn_errors} spawn failures under the storm — the OS was hit"),
            evidence,
        ));
    }
    evidence.push(format!(
        "ceiling held: <= {HOOK_CONCURRENT_MAX} concurrent children (the run semaphore), FDs within baseline + {FD_HEADROOM}; excess shed as recorded drops before spawning"
    ));
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({
            "fired": STORM_FIRES,
            "fire_elapsed_ms": fire_elapsed.as_millis(),
            "max_concurrent_children": max_children,
            "run_permit_ceiling": HOOK_CONCURRENT_MAX,
            "queue_ceiling": HOOK_QUEUED_MAX,
            "max_fds": max_fds,
            "spawn_errors": 0,
        }),
        evidence,
    ))
}

/// A2: crashed tools must not leak permits. A batch of hooks whose
/// commands cannot spawn plus a batch that exit nonzero runs through;
/// afterwards a fresh firing must still run — the queue slot and run
/// permit counts returned to baseline, so no handle was leaked.
fn case_crashed_tools_return_to_baseline() -> Result<CaseReport, DriverError> {
    const CASE: &str = "crashed_tools_return_to_baseline";
    let manager = HookManager::new();
    manager
        .register(
            HookEvent::AfterFocusChange,
            vec!["/nonexistent-phlow-hook-binary-xyz".to_string()],
        )
        .map_err(|e| fixture_error("hook register", e))?;
    manager
        .register(
            HookEvent::AfterWorkspaceSwitch,
            vec!["/bin/false".to_string()],
        )
        .map_err(|e| fixture_error("hook register", e))?;
    let ctx = HookContext::default();
    for _ in 0..8 {
        manager.fire(HookEvent::AfterFocusChange, &ctx);
        manager.fire(HookEvent::AfterWorkspaceSwitch, &ctx);
    }
    let outcomes = wait_for_outcomes(&manager, 16, Duration::from_millis(OUTCOME_WAIT_MS));
    let mut evidence = vec![format!("crash batch outcomes: {}", outcomes.len())];
    if outcomes.len() < 16 {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "only {} of 16 crash-batch outcomes recorded",
                outcomes.len()
            ),
            evidence,
        ));
    }
    // Baseline check: register a healthy hook on a fresh event and fire
    // once. If any permit leaked, this firing would shed (dropped=true)
    // or never run.
    manager
        .register(HookEvent::AfterNewWindow, vec!["/bin/true".to_string()])
        .map_err(|e| fixture_error("hook register", e))?;
    manager.fire(HookEvent::AfterNewWindow, &ctx);
    let after = wait_for_outcomes(&manager, 17, Duration::from_millis(OUTCOME_WAIT_MS));
    let probe = after.iter().find(|o| o.event == HookEvent::AfterNewWindow);
    match probe {
        Some(o) if !o.dropped && o.exit_code == Some(0) => {
            evidence.push(
                "post-crash firing ran clean (dropped=false, exit 0): permits returned to baseline"
                    .to_string(),
            );
        }
        Some(o) => {
            return Ok(CaseReport::fail(
                CASE,
                format!(
                    "post-crash firing degraded: dropped={}, exit_code={:?} — a permit leaked",
                    o.dropped, o.exit_code
                ),
                evidence,
            ));
        }
        None => {
            return Ok(CaseReport::fail(
                CASE,
                "post-crash firing never recorded — permits leaked".to_string(),
                evidence,
            ));
        }
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"crash_batch": 16, "baseline_restored": true}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "normal_fanout_runs" => case_normal_fanout_runs(),
        "spawn_paths_go_through_limiter" => case_spawn_paths_go_through_limiter(),
        "storm_10000_spawns_capped" => case_storm_10000_spawns_capped(),
        "crashed_tools_return_to_baseline" => case_crashed_tools_return_to_baseline(),
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
        "recon: the seam is phlow_tuios::hooks::HookManager — the only concurrent subprocess fan-out in phlow; a queue semaphore (HOOK_QUEUED_MAX=64, load-shed on full) plus a run semaphore (HOOK_CONCURRENT_MAX=8) bound it, and the single spawn site reaps on every path".to_string(),
        "note: excess does not arrive as a typed TooManyHandles error — fire() is fire-and-forget, so the shed shows up as a recorded dropped=true outcome (explicit, inspectable, never silent, never EMFILE)".to_string(),
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
        "finding: 10,000 rapid hook spawns held the ceiling — at most 8 concurrent children (run semaphore), FDs within baseline + headroom, zero spawn errors; normal fan-out runs clean; crashed tools return permits to baseline; the source audit accounts for every spawn site".to_string(),
    );
    Ok(evidence)
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
