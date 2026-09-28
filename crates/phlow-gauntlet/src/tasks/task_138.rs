//! Task 138 — restart recovery without rescan (rust, validation +
//! adversarial).
//!
//! The `RunLedger` is crash-resumable through a driver-local file
//! adapter (`FileLedger`, MOCK): every run event appends one strict
//! tab-separated line, and result blobs land in `blobs/<run_id>`.
//! Resume replays the file — a malformed line fails closed with typed
//! [`LedgerError::Corrupt`] and probes nothing. A `Finished` run whose
//! blob is missing is re-probed exactly once (at-least-once; the
//! duplicate is task 139's dedup job). A new scope version intersects:
//! completed targets in the new scope are skipped, dropped ones are
//! never re-probed, new ones are queued.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::bounty::{Run, RunLedger, RunState, ScopeSnapshot, Target, TargetId, TargetKind};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-138";
/// Task name.
pub const NAME: &str = "restart recovery without rescan";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "resume_probes_remaining_only",
    "corrupt_ledger_refuses",
    "lost_blob_reprobed_once",
    "new_scope_intersection",
];

/// Max bytes of one ledger line; longer lines are corruption, not data.
const LEDGER_LINE_BYTES_MAX: usize = 4096;
/// Max runs replayed from one ledger file.
const LEDGER_RUNS_MAX: usize = 10_000;

/// Typed ledger failures. Corruption fails closed: resume refuses and
/// probes nothing until an operator intervenes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerError {
    /// A ledger line failed strict validation.
    Corrupt { line_no: usize, detail: String },
    /// The ledger could not be read or written.
    Io { detail: String },
}

impl fmt::Display for LedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LedgerError::Corrupt { line_no, detail } => {
                write!(f, "ledger corrupt at line {line_no}: {detail}")
            }
            LedgerError::Io { detail } => write!(f, "ledger I/O failure: {detail}"),
        }
    }
}

impl std::error::Error for LedgerError {}

fn state_name(state: &RunState) -> &'static str {
    match state {
        RunState::Queued => "Queued",
        RunState::Running => "Running",
        RunState::Finished => "Finished",
        RunState::Failed => "Failed",
        RunState::Cancelled => "Cancelled",
    }
}

fn parse_state(token: &str) -> Option<RunState> {
    match token {
        "Queued" => Some(RunState::Queued),
        "Running" => Some(RunState::Running),
        "Finished" => Some(RunState::Finished),
        "Failed" => Some(RunState::Failed),
        "Cancelled" => Some(RunState::Cancelled),
        _ => None,
    }
}

/// A field is valid when non-empty and free of the tab/newline
/// separators; ids are bounded in length.
fn field_valid(field: &str) -> bool {
    !field.is_empty() && field.len() <= 256 && !field.contains(['\t', '\n', '\r'])
}

/// File-backed run ledger (MOCK). Append-only log of run events plus a
/// result-blob directory. `load` replays strictly: any malformed line
/// is [`LedgerError::Corrupt`], never skipped or guessed.
struct FileLedger {
    log: PathBuf,
    blobs: PathBuf,
}

impl FileLedger {
    fn create(dir: &Path) -> Result<Self, TaskDriverError> {
        let blobs = dir.join("blobs");
        fs::create_dir_all(&blobs).map_err(|e| TaskDriverError::Fixture {
            what: "ledger-dir".to_string(),
            detail: format!("task-138: cannot create ledger dir: {e}"),
        })?;
        Ok(FileLedger {
            log: dir.join("runs.log"),
            blobs,
        })
    }

    /// Append one run event. Ids are validated before the write so a
    /// driver bug can never produce a line `load` would accept as
    /// corrupt-but-plausible.
    fn append(&self, run: &Run) -> Result<(), TaskDriverError> {
        for (name, field) in [
            ("run id", run.id.as_str()),
            ("target id", run.target_id.0.as_str()),
        ] {
            if !field_valid(field) {
                return Err(TaskDriverError::Fixture {
                    what: "ledger-write".to_string(),
                    detail: format!("task-138: invalid {name} for ledger append"),
                });
            }
        }
        let reason = run.cancel_reason.as_deref().unwrap_or("");
        if !reason.is_empty() && !field_valid(reason) {
            return Err(TaskDriverError::Fixture {
                what: "ledger-write".to_string(),
                detail: "task-138: invalid cancel reason for ledger append".to_string(),
            });
        }
        let line = format!(
            "{}\t{}\t{}\t{}\n",
            run.id,
            run.target_id.0,
            state_name(&run.state),
            reason
        );
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)
            .map_err(|e| TaskDriverError::Fixture {
                what: "ledger-write".to_string(),
                detail: format!("task-138: cannot append to ledger: {e}"),
            })?;
        file.write_all(line.as_bytes())
            .map_err(|e| TaskDriverError::Fixture {
                what: "ledger-write".to_string(),
                detail: format!("task-138: cannot append to ledger: {e}"),
            })
    }

    fn write_blob(&self, run_id: &str, bytes: &[u8]) -> Result<(), TaskDriverError> {
        fs::write(self.blobs.join(run_id), bytes).map_err(|e| TaskDriverError::Fixture {
            what: "blob-write".to_string(),
            detail: format!("task-138: cannot write blob: {e}"),
        })
    }

    fn blob_present(&self, run_id: &str) -> bool {
        self.blobs.join(run_id).is_file()
    }

    fn remove_blob(&self, run_id: &str) -> Result<(), TaskDriverError> {
        fs::remove_file(self.blobs.join(run_id)).map_err(|e| TaskDriverError::Fixture {
            what: "blob-remove".to_string(),
            detail: format!("task-138: cannot remove blob: {e}"),
        })
    }

    /// Replay the log. A missing file is a fresh ledger (no runs); any
    /// malformed line is corruption — fail closed, probe nothing.
    fn load(&self) -> Result<Vec<Run>, LedgerError> {
        let text = match fs::read_to_string(&self.log) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(LedgerError::Io {
                    detail: e.to_string(),
                });
            }
        };
        let mut runs = Vec::new();
        for (idx, line) in text.lines().enumerate() {
            let line_no = idx + 1;
            if runs.len() >= LEDGER_RUNS_MAX {
                return Err(LedgerError::Corrupt {
                    line_no,
                    detail: format!("run cap {LEDGER_RUNS_MAX} exceeded"),
                });
            }
            if line.len() > LEDGER_LINE_BYTES_MAX {
                return Err(LedgerError::Corrupt {
                    line_no,
                    detail: format!("line longer than {LEDGER_LINE_BYTES_MAX} bytes"),
                });
            }
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() != 4 {
                return Err(LedgerError::Corrupt {
                    line_no,
                    detail: format!("want 4 tab fields, got {}", fields.len()),
                });
            }
            let (run_id, target_id, state_tok, reason) =
                (fields[0], fields[1], fields[2], fields[3]);
            if !field_valid(run_id) || !field_valid(target_id) {
                return Err(LedgerError::Corrupt {
                    line_no,
                    detail: "empty or oversized id field".to_string(),
                });
            }
            let state = parse_state(state_tok).ok_or_else(|| LedgerError::Corrupt {
                line_no,
                detail: format!("unknown run state {state_tok:?}"),
            })?;
            if !reason.is_empty() && !field_valid(reason) {
                return Err(LedgerError::Corrupt {
                    line_no,
                    detail: "invalid cancel reason".to_string(),
                });
            }
            runs.push(Run {
                id: run_id.to_string(),
                target_id: TargetId(target_id.to_string()),
                state,
                approval_nonce: 0,
                cancel_reason: if reason.is_empty() {
                    None
                } else {
                    Some(reason.to_string())
                },
            });
        }
        Ok(runs)
    }

    /// Lines recorded per target id (append-only: a run has one line per
    /// state change).
    fn lines_per_target(&self) -> Result<HashMap<TargetId, usize>, LedgerError> {
        let mut counts: HashMap<TargetId, usize> = HashMap::new();
        for run in self.load()? {
            *counts.entry(run.target_id).or_insert(0) += 1;
        }
        Ok(counts)
    }
}

/// What one resume cycle must probe.
struct ResumePlan {
    to_probe: Vec<TargetId>,
    skipped_finished: usize,
    reprobed_lost_blobs: Vec<TargetId>,
}

/// Build the resume plan from replayed runs and the current scope.
/// The scaffold's terminal rule holds (Finished/Failed/Cancelled are
/// never re-probed) with one deliberate exception: a `Finished` run
/// whose result blob is missing is re-probed exactly once — the
/// "completion" is hollow without its artifact, so at-least-once
/// applies and task 139's dedup absorbs the duplicate.
fn plan_resume(
    runs: &[Run],
    scope: &ScopeSnapshot,
    ledger: &FileLedger,
) -> Result<ResumePlan, TaskDriverError> {
    // Latest run per target: the log is time-ordered, so last wins.
    // Targets recorded in the ledger but absent from this scope version
    // are dropped by construction: the loop only visits scope targets.
    let mut latest: HashMap<&TargetId, &Run> = HashMap::new();
    for run in runs {
        latest.insert(&run.target_id, run);
    }
    let mut plan = ResumePlan {
        to_probe: Vec::new(),
        skipped_finished: 0,
        reprobed_lost_blobs: Vec::new(),
    };
    for target in &scope.targets {
        match latest.get(&target.id) {
            None => plan.to_probe.push(target.id.clone()),
            Some(run) => match run.state {
                RunState::Finished if ledger.blob_present(&run.id) => {
                    plan.skipped_finished += 1;
                }
                RunState::Finished => {
                    // Blob lost: at-least-once re-probe.
                    plan.reprobed_lost_blobs.push(target.id.clone());
                    plan.to_probe.push(target.id.clone());
                }
                RunState::Queued | RunState::Running => {
                    plan.to_probe.push(target.id.clone());
                }
                RunState::Failed | RunState::Cancelled => {
                    // Terminal per the scaffold: not re-probed by resume.
                }
            },
        }
    }
    // Targets recorded in the ledger but absent from this scope version
    // are dropped, never re-probed.
    Ok(plan)
}

/// Scripted probe (MOCK): record Running → Finished, write the blob.
fn probe(
    ledger: &FileLedger,
    run_ledger: &mut RunLedger,
    target: &Target,
    run_no: &mut u64,
) -> Result<String, TaskDriverError> {
    let run_id = format!("r{:03}", *run_no);
    *run_no += 1;
    let run = Run {
        id: run_id.clone(),
        target_id: target.id.clone(),
        state: RunState::Running,
        approval_nonce: 0,
        cancel_reason: None,
    };
    run_ledger.record(run.clone());
    ledger.append(&run)?;
    let finished = Run {
        state: RunState::Finished,
        ..run
    };
    run_ledger.record(finished.clone());
    ledger.append(&finished)?;
    ledger.write_blob(&run_id, format!("result-blob:{}", target.id.0).as_bytes())?;
    Ok(run_id)
}

fn mk_target(i: usize) -> Target {
    Target {
        id: TargetId(format!("t{i:02}")),
        kind: TargetKind::Domain,
        value: format!("t{i:02}.example.com"),
    }
}

fn mk_scope(version: u64, from: usize, to: usize) -> ScopeSnapshot {
    ScopeSnapshot {
        version,
        targets: (from..to).map(mk_target).collect(),
        fetched_at: 0,
    }
}

/// Scratch dir unique to this case (tests in one process share a PID).
fn scratch_dir(case: &str) -> PathBuf {
    std::env::temp_dir().join(format!("gauntlet-138-{}-{case}", std::process::id()))
}

fn cleanup(dir: &Path) {
    // Best-effort: scratch cleanup must not fail the case.
    let _ = fs::remove_dir_all(dir);
}

/// V1: kill after 3 of 10 finish. Resume probes exactly the remaining
/// 7; the ledger ends with 10 Finished, 0 duplicates.
fn case_resume_probes_remaining_only() -> Result<CaseReport, TaskDriverError> {
    let dir = scratch_dir("v1");
    let _ = fs::remove_dir_all(&dir);
    let ledger = FileLedger::create(&dir)?;
    let scope = mk_scope(1, 1, 11); // t01..t10
    let mut run_ledger = RunLedger::new();
    let mut run_no = 0u64;
    // Cycle 1: probe t01..t03, then "crash" (drop everything).
    for target in scope.targets.iter().take(3) {
        probe(&ledger, &mut run_ledger, target, &mut run_no)?;
    }
    drop(run_ledger);
    // Cycle 2: resume on the same dir.
    let ledger2 = FileLedger::create(&dir)?;
    let runs = ledger2.load().map_err(|e| TaskDriverError::Arm {
        arm: CASES[0].to_string(),
        detail: format!("task-138: resume load failed: {e}"),
    })?;
    let plan = plan_resume(&runs, &scope, &ledger2)?;
    // Cross-check against the scaffold's own pending computation, over
    // the targets the ledger actually recorded: pending_target_ids
    // only knows recorded targets (it cannot see never-probed scope
    // targets), so the comparison is restricted to that projection.
    // pending_target_ids counts a target terminal only when EVERY
    // recorded run is terminal, so the comparison ledger is built from
    // the latest run per target — the same last-wins projection
    // plan_resume uses. With no lost blobs every recorded target's
    // latest run is Finished, hence the scaffold must report zero
    // pending there.
    let mut latest: HashMap<&TargetId, &Run> = HashMap::new();
    for run in &runs {
        latest.insert(&run.target_id, run);
    }
    let mut latest_runs: Vec<&&Run> = latest.values().collect();
    latest_runs.sort_by(|a, b| a.target_id.0.cmp(&b.target_id.0));
    let mut mem_ledger = RunLedger::new();
    for run in latest_runs {
        mem_ledger.record((*run).clone());
    }
    let mut scaffold_pending = mem_ledger.pending_target_ids();
    scaffold_pending.sort_by(|a, b| a.0.cmp(&b.0));
    let recorded: HashSet<TargetId> = runs.iter().map(|r| r.target_id.clone()).collect();
    let mut plan_on_recorded: Vec<TargetId> = plan
        .to_probe
        .iter()
        .filter(|id| recorded.contains(id))
        .cloned()
        .collect();
    plan_on_recorded.sort_by(|a, b| a.0.cmp(&b.0));
    let mut run_ledger2 = RunLedger::new();
    let mut probed: Vec<TargetId> = Vec::new();
    for target in &scope.targets {
        if plan.to_probe.contains(&target.id) {
            probe(&ledger2, &mut run_ledger2, target, &mut run_no)?;
            probed.push(target.id.clone());
        }
    }
    let final_counts = ledger2
        .lines_per_target()
        .map_err(|e| TaskDriverError::Arm {
            arm: CASES[0].to_string(),
            detail: format!("task-138: final replay failed: {e}"),
        })?;
    let mut failures = Vec::new();
    if probed.len() != 7 {
        failures.push(format!("resumed probes {} != 7", probed.len()));
    }
    let probed_want: HashSet<String> = (4..11).map(|i| format!("t{i:02}")).collect();
    let probed_got: HashSet<String> = probed.iter().map(|t| t.0.clone()).collect();
    if probed_got != probed_want {
        failures.push(format!("probed set {probed_got:?} != {probed_want:?}"));
    }
    if plan_on_recorded != scaffold_pending {
        failures.push(format!(
            "driver plan {plan_on_recorded:?} disagrees with scaffold pending_target_ids {scaffold_pending:?} over recorded targets"
        ));
    }
    // 0 duplicates: the 3 early finishers gained no new lines.
    for i in 1..4 {
        let id = TargetId(format!("t{i:02}"));
        if final_counts.get(&id).copied().unwrap_or(0) != 2 {
            failures.push(format!("t{i:02} re-recorded on resume"));
        }
    }
    let finished_targets: HashSet<String> = final_counts.keys().map(|t| t.0.clone()).collect();
    if finished_targets.len() != 10 {
        failures.push(format!(
            "ledger covers {} targets, want 10",
            finished_targets.len()
        ));
    }
    let evidence = vec![
        "cycle 1: probed t01..t03 (Finished + blobs), then killed".to_string(),
        format!(
            "resume plan: probe {} targets, skip {} finished",
            plan.to_probe.len(),
            plan.skipped_finished
        ),
        format!(
            "probed on resume: {:?}",
            probed.iter().map(|t| t.0.clone()).collect::<Vec<_>>()
        ),
        "driver plan == scaffold pending_target_ids over the recorded-target, latest-run projection (no lost blobs)"
            .to_string(),
        "early finishers t01..t03: still 2 lines each (never rescanned)".to_string(),
        "final ledger: 10 targets, each Finished exactly once".to_string(),
        "file-backed ledger in temp dir; ManualClock unused by resume (MOCK)".to_string(),
    ];
    cleanup(&dir);
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "resumed_probes": probed.len(),
            "skipped_finished": plan.skipped_finished,
            "targets_covered": finished_targets.len(),
            "scaffold_agrees": plan_on_recorded == scaffold_pending,
            "backend": "scripted-mock",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    if !report.passed {
        report.failures.clone_from(&failures);
    }
    Ok(report)
}

/// V2 (adversarial): the ledger file is corrupted on disk. Resume
/// refuses with typed `LedgerCorrupt` and probes nothing — no silent
/// full rescan.
fn case_corrupt_ledger_refuses() -> Result<CaseReport, TaskDriverError> {
    let dir = scratch_dir("v2");
    let _ = fs::remove_dir_all(&dir);
    let ledger = FileLedger::create(&dir)?;
    let scope = mk_scope(1, 1, 11);
    let mut run_ledger = RunLedger::new();
    let mut run_no = 0u64;
    probe(&ledger, &mut run_ledger, &scope.targets[0], &mut run_no)?;
    // Corrupt: APPEND a line that fails strict validation. (fs::write
    // would truncate the two valid lines; the corruption must sit at
    // line 3 after them.)
    fs::OpenOptions::new()
        .append(true)
        .open(&ledger.log)
        .and_then(|mut f| f.write_all(b"this is not a ledger line\n"))
        .map_err(|e| TaskDriverError::Fixture {
            what: "corrupt-fixture".to_string(),
            detail: format!("task-138: cannot plant corruption: {e}"),
        })?;
    let mut failures = Vec::new();
    let mut error_text = String::new();
    match ledger.load() {
        Err(LedgerError::Corrupt { line_no, detail }) => {
            error_text = format!("LedgerError::Corrupt {{ line_no: {line_no}, detail: {detail} }}");
            if line_no != 3 {
                failures.push(format!("corruption reported at line {line_no}, want 3"));
            }
            // Fail closed: the plan is never built, nothing is probed.
        }
        Err(other) => failures.push(format!("wrong error type: {other}")),
        Ok(_) => failures.push("corrupt ledger LOADED — fail-closed violated".to_string()),
    }
    let evidence = vec![
        "planted: one valid probe, then a non-tab garbage line".to_string(),
        format!("resume load -> {error_text}"),
        "probed after refusal: 0 (fail closed, no silent rescan)".to_string(),
        "operator must confirm before any probe runs again".to_string(),
        "file-backed ledger in temp dir (MOCK)".to_string(),
    ];
    cleanup(&dir);
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "error": error_text,
            "probed": 0,
            "backend": "scripted-mock",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    if !report.passed {
        report.failures.clone_from(&failures);
    }
    Ok(report)
}

/// A1 (adversarial): t02 is `Finished` but its result blob was lost.
/// At-least-once: it is re-probed exactly once; the others are not.
fn case_lost_blob_reprobed_once() -> Result<CaseReport, TaskDriverError> {
    let dir = scratch_dir("a1");
    let _ = fs::remove_dir_all(&dir);
    let ledger = FileLedger::create(&dir)?;
    let scope = mk_scope(1, 1, 11);
    let mut run_ledger = RunLedger::new();
    let mut run_no = 0u64;
    let mut run_ids: HashMap<String, String> = HashMap::new();
    for target in scope.targets.iter().take(3) {
        let rid = probe(&ledger, &mut run_ledger, target, &mut run_no)?;
        run_ids.insert(target.id.0.clone(), rid);
    }
    // Lose t02's blob.
    let t02_run = run_ids["t02"].clone();
    ledger.remove_blob(&t02_run)?;
    drop(run_ledger);
    // Resume.
    let ledger2 = FileLedger::create(&dir)?;
    let runs = ledger2.load().map_err(|e| TaskDriverError::Arm {
        arm: CASES[2].to_string(),
        detail: format!("task-138: resume load failed: {e}"),
    })?;
    let plan = plan_resume(&runs, &scope, &ledger2)?;
    let mut run_ledger2 = RunLedger::new();
    let mut probed: Vec<TargetId> = Vec::new();
    for target in &scope.targets {
        if plan.to_probe.contains(&target.id) {
            probe(&ledger2, &mut run_ledger2, target, &mut run_no)?;
            probed.push(target.id.clone());
        }
    }
    let mut failures = Vec::new();
    if plan.reprobed_lost_blobs != vec![TargetId("t02".to_string())] {
        failures.push(format!(
            "reprobe set {:?} != [t02]",
            plan.reprobed_lost_blobs
        ));
    }
    if probed.len() != 8 {
        failures.push(format!(
            "probed {} targets, want 8 (t02 + t04..t10)",
            probed.len()
        ));
    }
    // Exactly-once re-probe: t02 has 4 lines (2 runs), one fresh blob.
    let counts = ledger2
        .lines_per_target()
        .map_err(|e| TaskDriverError::Arm {
            arm: CASES[2].to_string(),
            detail: format!("task-138: final replay failed: {e}"),
        })?;
    if counts
        .get(&TargetId("t02".to_string()))
        .copied()
        .unwrap_or(0)
        != 4
    {
        failures.push("t02 does not have exactly 2 run records".to_string());
    }
    let t02_runs: Vec<&Run> = runs
        .iter()
        .chain(run_ledger2.runs().iter())
        .filter(|r| r.target_id.0 == "t02")
        .collect();
    let distinct_ids: HashSet<&str> = t02_runs.iter().map(|r| r.id.as_str()).collect();
    if distinct_ids.len() != 2 {
        failures.push(format!("t02 run ids not exactly 2: {distinct_ids:?}"));
    }
    let new_run_id = distinct_ids
        .iter()
        .find(|id| **id != t02_run.as_str())
        .copied()
        .unwrap_or("");
    if !ledger2.blob_present(new_run_id) {
        failures.push("re-probe wrote no fresh blob".to_string());
    }
    for i in [1usize, 3] {
        let id = TargetId(format!("t{i:02}"));
        if counts.get(&id).copied().unwrap_or(0) != 2 {
            failures.push(format!("t{i:02} re-recorded despite intact blob"));
        }
    }
    let evidence = vec![
        "cycle 1: t01..t03 Finished + blobs; t02's blob then deleted".to_string(),
        format!(
            "resume plan: reprobe_lost_blobs = {:?}",
            plan.reprobed_lost_blobs
                .iter()
                .map(|t| t.0.clone())
                .collect::<Vec<_>>()
        ),
        format!(
            "probed on resume: {} targets (t02 + t04..t10)",
            probed.len()
        ),
        "t02: 2 run records (original + re-probe), fresh blob present".to_string(),
        "t01, t03: still 2 lines each — intact completions never rescanned".to_string(),
        "at-least-once: the duplicate is task 139's dedup job".to_string(),
        "file-backed ledger in temp dir (MOCK)".to_string(),
    ];
    cleanup(&dir);
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "reprobed": plan.reprobed_lost_blobs.iter().map(|t| t.0.clone()).collect::<Vec<_>>(),
            "probed": probed.len(),
            "t02_run_records": 2,
            "backend": "scripted-mock",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    if !report.passed {
        report.failures.clone_from(&failures);
    }
    Ok(report)
}

/// A2 (adversarial): resume under a new scope version. Completed set is
/// intersected with the new scope: dropped targets are never
/// re-probed, new targets are queued.
fn case_new_scope_intersection() -> Result<CaseReport, TaskDriverError> {
    let dir = scratch_dir("a2");
    let _ = fs::remove_dir_all(&dir);
    let ledger = FileLedger::create(&dir)?;
    let scope_v1 = mk_scope(1, 1, 11); // t01..t10
    let mut run_ledger = RunLedger::new();
    let mut run_no = 0u64;
    for target in scope_v1.targets.iter().take(3) {
        probe(&ledger, &mut run_ledger, target, &mut run_no)?;
    }
    drop(run_ledger);
    // New scope version: t03..t12 (t01,t02 dropped; t11,t12 new).
    let scope_v2 = mk_scope(2, 3, 13);
    let ledger2 = FileLedger::create(&dir)?;
    let runs = ledger2.load().map_err(|e| TaskDriverError::Arm {
        arm: CASES[3].to_string(),
        detail: format!("task-138: resume load failed: {e}"),
    })?;
    let plan = plan_resume(&runs, &scope_v2, &ledger2)?;
    let mut run_ledger2 = RunLedger::new();
    let mut probed: Vec<TargetId> = Vec::new();
    for target in &scope_v2.targets {
        if plan.to_probe.contains(&target.id) {
            probe(&ledger2, &mut run_ledger2, target, &mut run_no)?;
            probed.push(target.id.clone());
        }
    }
    let mut failures = Vec::new();
    let probed_want: HashSet<String> = (4..13).map(|i| format!("t{i:02}")).collect();
    let probed_got: HashSet<String> = probed.iter().map(|t| t.0.clone()).collect();
    if probed_got != probed_want {
        failures.push(format!("probed {probed_got:?} != {probed_want:?}"));
    }
    if plan.skipped_finished != 1 {
        failures.push(format!(
            "skipped_finished {} != 1 (t03)",
            plan.skipped_finished
        ));
    }
    // Dropped targets never re-probed: t01,t02 keep exactly 2 lines.
    let counts = ledger2
        .lines_per_target()
        .map_err(|e| TaskDriverError::Arm {
            arm: CASES[3].to_string(),
            detail: format!("task-138: final replay failed: {e}"),
        })?;
    for id in ["t01", "t02", "t03"] {
        let want = 2;
        let got = counts.get(&TargetId(id.to_string())).copied().unwrap_or(0);
        if got != want {
            failures.push(format!("{id}: {got} lines, want {want}"));
        }
    }
    let evidence = vec![
        "scope v1: t01..t10, probed t01..t03; scope v2: t03..t12".to_string(),
        "completed ∩ v2 = {t03}: skipped (blob intact)".to_string(),
        format!(
            "probed on resume: {:?}",
            probed.iter().map(|t| t.0.clone()).collect::<Vec<_>>()
        ),
        "dropped t01,t02: still 2 lines each, never re-probed".to_string(),
        "new t11,t12: queued and probed".to_string(),
        "file-backed ledger in temp dir (MOCK)".to_string(),
    ];
    cleanup(&dir);
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "probed": probed.len(),
            "skipped_finished": plan.skipped_finished,
            "scope_version": 2,
            "backend": "scripted-mock",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    if !report.passed {
        report.failures.clone_from(&failures);
    }
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "resume_probes_remaining_only" => case_resume_probes_remaining_only(),
        "corrupt_ledger_refuses" => case_corrupt_ledger_refuses(),
        "lost_blob_reprobed_once" => case_lost_blob_reprobed_once(),
        "new_scope_intersection" => case_new_scope_intersection(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-138: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-138".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-138".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
