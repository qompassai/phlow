//! task-67: audit completeness (rust).
//!
//! Recon probe: the design asks for an audit event EMITTER at every
//! state transition — every transition emits exactly one audit event,
//! no gaps. Scenarios: default (a full lifecycle emits the exact
//! expected event sequence, asserted event-for-event); adversarial: a
//! transition through an unusual path (resume, retry, compensation —
//! still emits); adversarial: the emitter itself errors mid-run (the
//! run fails closed rather than continuing unaudited — or the gap is
//! explicitly tolerated and documented, never silent). Pass criteria:
//! the event log is a *complete* record of the transition log (a
//! mechanical cross-check); zero transitions without events.
//! Distinct from task-42 (tamper-*integrity*: present-but-modified) —
//! this is *completeness*: transitions with no event at all.
//!
//! Honest result: the seam is ABSENT, and the finding is the gap —
//! exactly what the design allows ("the gap is explicitly tolerated
//! and documented, never silent"). The evidence is threefold, all
//! gathered at probe time from the working tree:
//!
//! 1. Emitter-vocabulary scan: a walk over every `crates/*/src/**/*.rs`
//!    (excluding the phlow-gauntlet probe harness itself) finds zero
//!    emitter-mechanism tokens — no event emitter, no emission call at
//!    any transition site, no audit sink type in any public API.
//! 2. The one audit-adjacent writer that DOES exist,
//!    `EvaluationRecord::record_event` (phlow-experiment/src/record.rs),
//!    is a manual opt-in recorder with ZERO production call sites — no
//!    transition calls it, so it cannot be the seam. (The only known
//!    invocations are the crate's own integration test and the gauntlet
//!    harness's fixture drivers: test code, not transition emission.)
//! 3. The real state machines are demonstrated behaviorally: the
//!    `promotion::Lifecycle` walks six transitions (Proposed ->
//!    ... -> Monitored) and the real `Evaluator` walks
//!    Validate -> Prepare -> Execute, and every transition returns
//!    only the next state (`Result<Lifecycle, _>`, `Result<(), _>`).
//!    There is no emitter parameter to pass and no event in any
//!    return type — emission is impossible by construction, not merely
//!    unobserved. The only transition-adjacent counter is
//!    `Evaluator.transitions: u32` — it COUNTS transitions, it records
//!    no events (classified adjacent, not the seam).
//!
//! The design's mechanical cross-check (event log vs transition log)
//! has no event log to run against: the transitions are real and
//! observable, the events are absent. The adversarial "emitter errors
//! mid-run" weapon has no target — there is no emitter to fail.
//!
//! Four cases: two validation, two adversarial. The task-level verdict
//! is `fail` at `"seam"`.
//!
//! Banked for Matt (product decision, NOT auto-implemented): whether
//! phlow should gain an audit event emitter wired into
//! `Lifecycle::transition` and the `Evaluator` stage machine (with
//! fail-closed semantics when emission fails); whether emission is
//! synchronous at the transition site or via a subscribed sink; and
//! whether the manual `EvaluationRecord::record_event` should become
//! that emitter or stay a manual recorder.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_experiment::{BudgetTracker, Evaluator, ExperimentError, Lifecycle, LifecycleEvent};
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-67";
/// Human-readable name.
pub const NAME: &str = "audit completeness";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "no_audit_emitter_in_sources",
    "transitions_produce_no_events",
    "unusual_paths_also_silent",
    "emitter_failure_has_no_target",
];

/// Largest Rust source file the probe will scan, in bytes.
const SOURCE_BYTES_MAX: usize = 1_048_576;
/// Most source files the probe will scan before stopping.
const SOURCE_FILES_MAX: usize = 50_000;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-67 driver itself (not of the code under test).
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
                write!(f, "task-67: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-67: cannot probe {what}: {detail}")
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

/// Audit-emitter mechanism tokens, assembled at runtime from halves so
/// the probe's own source never contains the literal tokens it scans
/// for. These name the design's required machinery: an event emitter
/// that fires at every state transition, an audit sink type, emission
/// wired into the transition sites.
fn emitter_tokens() -> Vec<String> {
    const HALVES: [(&str, &str); 8] = [
        ("audit", "_event"),
        ("emit", "_event"),
        ("event", "_emitter"),
        ("audit", "_emit"),
        ("audit", "_sink"),
        ("transition", "_audit"),
        ("emit", "_audit"),
        ("on", "_transition"),
    ];
    HALVES.iter().map(|(a, b)| format!("{a}{b}")).collect()
}

/// Walk `crates/` under the workspace root and return every
/// `path: token` hit for `.rs` files inside a `src` tree, skipping the
/// whole phlow-gauntlet probe harness (it is the scanner, not the
/// product). Bounded: files over [`SOURCE_BYTES_MAX`]
/// are skipped, and the walk stops after [`SOURCE_FILES_MAX`] files.
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
            if path.components().any(|c| c.as_os_str() == "phlow-gauntlet") {
                continue;
            }
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

/// Count PRODUCTION call sites of the manual recorder: lines containing
/// the call form, excluding the definition itself. Skips the whole
/// phlow-gauntlet probe harness (its fixture drivers call the recorder
/// as test subjects) and every `tests/` tree (test-only callers are not
/// transition emission). Assembled from halves so the probe source
/// carries no literal the V1 scan would trip on.
fn record_event_callers(root: &Path) -> Result<usize, DriverError> {
    let call = format!("{}{}", "record", "_event(");
    let defn = format!("{}{}", "fn record", "_event");
    let mut count = 0usize;
    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| fixture_error("caller walk", format!("{}: {e}", dir.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| fixture_error("caller walk", e))?;
            let path = entry.path();
            if path.components().any(|c| {
                let s = c.as_os_str();
                s == "phlow-gauntlet" || s == "tests"
            }) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.components().any(|c| c.as_os_str() == "src")
            {
                let bytes = std::fs::read(&path).map_err(|e| {
                    fixture_error("caller read", format!("{}: {e}", path.display()))
                })?;
                if bytes.len() > SOURCE_BYTES_MAX {
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes);
                for line in text.lines() {
                    let trimmed = line.trim_start();
                    if trimmed.starts_with("//") || trimmed.starts_with("///") {
                        continue;
                    }
                    if line.contains(&call) && !line.contains(&defn) {
                        count += 1;
                    }
                }
            }
        }
    }
    Ok(count)
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
            metrics: serde_json::json!({}),
            evidence,
            failures: vec![failure],
        }
    }
}

/// V1: no audit-emitter machinery exists in any phlow source; the one
/// audit-adjacent writer (`EvaluationRecord::record_event`) has zero
/// call sites — it is a manual opt-in recorder, not a transition
/// emitter.
fn case_no_audit_emitter_in_sources() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_audit_emitter_in_sources";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let tokens = emitter_tokens();
    let hits = scan_sources(&root, &tokens)?;
    let callers = record_event_callers(&root)?;
    evidence.push(format!(
        "emitter-vocabulary scan over crates/*/src: {} hits (want 0)",
        hits.len()
    ));
    for hit in hits.iter().take(8) {
        evidence.push(format!("  hit: {hit}"));
    }
    evidence.push(format!(
        "manual recorder PRODUCTION call sites outside its definition: {callers} (want 0)"
    ));
    evidence.push(
        "phlow-experiment/src/record.rs defines a manual recorder on EvaluationRecord; \
         it is opt-in (the caller must invoke it) and no production transition invokes \
         it — a recorder with no production callers cannot be the transition emitter"
            .to_string(),
    );
    evidence.push(
        "known NON-production invocations (classified, not the seam): \
         crates/phlow-experiment/tests/integration.rs calls it in an integration test, \
         and the gauntlet harness's own fixture drivers (tasks 32, 34) call it as test \
         subjects — both are excluded as test code"
            .to_string(),
    );
    if !hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!("emitter machinery found: {}", hits.join("; ")),
            evidence,
        ));
    }
    if callers != 0 {
        return Ok(CaseReport::fail(
            CASE,
            format!("manual recorder has {callers} callers"),
            evidence,
        ));
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"emitter_hits": 0, "recorder_callers": 0}),
        evidence,
    ))
}

/// Walk the real `promotion::Lifecycle` through six transitions and the
/// real `Evaluator` through three stage transitions. Every transition
/// returns only the next state — there is no emitter parameter to pass
/// and no event in any return type, so emission is impossible by
/// construction, not merely unobserved.
fn case_transitions_produce_no_events() -> Result<CaseReport, DriverError> {
    const CASE: &str = "transitions_produce_no_events";
    let mut evidence = Vec::new();

    let hops = [
        (Lifecycle::Proposed, LifecycleEvent::WorkspaceCreated),
        (Lifecycle::Isolated, LifecycleEvent::ChecksComplete),
        (Lifecycle::Tested, LifecycleEvent::EvaluationComplete),
        (Lifecycle::Reviewed, LifecycleEvent::GatesSatisfied),
        (Lifecycle::AwaitingHuman, LifecycleEvent::HumanApproved),
        (Lifecycle::Promoted, LifecycleEvent::DeployedToCanary),
    ];
    let mut state = Lifecycle::Proposed;
    let mut walked = 0u32;
    for (from, event) in hops {
        if state != from {
            return Ok(CaseReport::fail(
                CASE,
                format!(
                    "lifecycle walk desynced: at {} expected {}",
                    state.name(),
                    from.name()
                ),
                evidence,
            ));
        }
        state = state.transition(event).map_err(|e| {
            probe_error(
                "lifecycle walk",
                format!("{} + {}: {e}", from.name(), event.name()),
            )
        })?;
        walked += 1;
    }
    evidence.push(format!(
        "lifecycle walked {walked} transitions to {}; each transition() \
         returns only Result<Lifecycle, _> — no emitter argument, no event \
         in the return type",
        state.name()
    ));

    let budget =
        BudgetTracker::new(10, 1_000, 3_600_000).map_err(|e| probe_error("budget fixture", e))?;
    let mut evaluator = Evaluator::new(budget);
    evaluator
        .validate()
        .map_err(|e| probe_error("evaluator validate", e))?;
    evaluator
        .prepare()
        .map_err(|e| probe_error("evaluator prepare", e))?;
    evaluator
        .execute(1, 100)
        .map_err(|e| probe_error("evaluator execute", e))?;
    evidence.push(format!(
        "evaluator walked Validate -> Prepare -> Execute (now at {}); \
         validate/prepare/execute return Result<(), _> — the stage machine \
         has no event channel",
        evaluator.stage().name()
    ));
    evidence.push(
        "adjacent, not the seam: Evaluator.transitions is a u32 that COUNTS \
         stage transitions (STAGE_TRANSITIONS_MAX bound) — it records no \
         per-transition events"
            .to_string(),
    );
    evidence.push(
        "the design's mechanical cross-check (event log vs transition log) \
         has no event log to run against: transitions are real and \
         observable, events are absent"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"lifecycle_transitions": walked, "evaluator_stages": 3}),
        evidence,
    ))
}

/// A1: unusual paths — rejection, terminal-state rejection, expiry,
/// rollback (the compensation path), budget exhaustion. The state
/// machines behave per contract (fail closed where designed) and emit
/// nothing on any of these paths either.
fn case_unusual_paths_also_silent() -> Result<CaseReport, DriverError> {
    const CASE: &str = "unusual_paths_also_silent";
    let mut evidence = Vec::new();

    let rejected = Lifecycle::Proposed
        .transition(LifecycleEvent::ContractInvalid)
        .map_err(|e| probe_error("rejection path", e))?;
    if rejected != Lifecycle::Rejected {
        return Ok(CaseReport::fail(
            CASE,
            format!("expected Rejected, got {}", rejected.name()),
            evidence,
        ));
    }
    evidence
        .push("unusual path 1: Proposed + ContractInvalid -> Rejected (no event emitted)".into());

    let terminal = Lifecycle::Rejected.transition(LifecycleEvent::WorkspaceCreated);
    match terminal {
        Err(ExperimentError::LifecycleTerminal { .. }) => {
            evidence.push(
                "unusual path 2: event on terminal Rejected fails closed with \
                 LifecycleTerminal (no event emitted)"
                    .to_string(),
            );
        }
        other => {
            return Ok(CaseReport::fail(
                CASE,
                format!("terminal state did not fail closed: {other:?}"),
                evidence,
            ));
        }
    }

    let expired = Lifecycle::AwaitingHuman
        .transition(LifecycleEvent::ApprovalExpired)
        .map_err(|e| probe_error("expiry path", e))?;
    if expired != Lifecycle::Rejected {
        return Ok(CaseReport::fail(
            CASE,
            format!("expected Rejected on expiry, got {}", expired.name()),
            evidence,
        ));
    }
    evidence.push(
        "unusual path 3: AwaitingHuman + ApprovalExpired -> Rejected (no event emitted)".into(),
    );

    let rolled_back = Lifecycle::Promoted
        .transition(LifecycleEvent::DeployedToCanary)
        .and_then(|s| s.transition(LifecycleEvent::RegressionDetected))
        .map_err(|e| probe_error("rollback path", e))?;
    if rolled_back != Lifecycle::RolledBack {
        return Ok(CaseReport::fail(
            CASE,
            format!("expected RolledBack, got {}", rolled_back.name()),
            evidence,
        ));
    }
    evidence.push(
        "unusual path 4 (compensation): Promoted -> Monitored -> RolledBack \
         on RegressionDetected (no event emitted)"
            .to_string(),
    );

    let illegal = Lifecycle::Proposed.transition(LifecycleEvent::HumanApproved);
    match illegal {
        Err(ExperimentError::BadTransition { .. }) => {
            evidence.push(
                "unusual path 5: Proposed + HumanApproved fails closed with \
                 BadTransition (no event emitted)"
                    .to_string(),
            );
        }
        other => {
            return Ok(CaseReport::fail(
                CASE,
                format!("illegal transition did not fail closed: {other:?}"),
                evidence,
            ));
        }
    }

    let budget =
        BudgetTracker::new(2, 1_000, 3_600_000).map_err(|e| probe_error("budget fixture", e))?;
    let mut evaluator = Evaluator::new(budget);
    evaluator
        .validate()
        .map_err(|e| probe_error("evaluator validate", e))?;
    evaluator
        .prepare()
        .map_err(|e| probe_error("evaluator prepare", e))?;
    match evaluator.execute(100, 100) {
        Err(ExperimentError::BudgetExhausted { .. }) => {
            evidence.push(
                "unusual path 6: execute() against a 2-call budget fails closed \
                 with BudgetExhausted — a failed transition emits no event either"
                    .to_string(),
            );
        }
        other => {
            return Ok(CaseReport::fail(
                CASE,
                format!("budget exhaustion did not fail closed: {other:?}"),
                evidence,
            ));
        }
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"unusual_paths": 6}),
        evidence,
    ))
}

/// A2: the design's "emitter errors mid-run" adversarial has no target.
/// There is no emitter type in phlow-experiment's public API, so there
/// is nothing whose failure could be made to fail the run closed. The
/// task-level verdict folds in here: fail at "seam".
fn case_emitter_failure_has_no_target() -> Result<CaseReport, DriverError> {
    const CASE: &str = "emitter_failure_has_no_target";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let lib_rs = root
        .join("crates")
        .join("phlow-experiment")
        .join("src")
        .join("lib.rs");
    let text = std::fs::read_to_string(&lib_rs)
        .map_err(|e| fixture_error("lib.rs read", format!("{}: {e}", lib_rs.display())))?;
    let needle = |frag: &str| text.to_lowercase().contains(frag);
    let emitter_types = ["emitter", "sink", "audit"]
        .iter()
        .filter(|frag| needle(frag))
        .count();
    evidence.push(format!(
        "phlow-experiment public API mentions emitter/sink/audit types: {emitter_types} (want 0)"
    ));
    evidence.push(
        "the one audit-adjacent doc line ('read-only view for audit' on \
         Evaluator::budget) is a read accessor, not an emission site"
            .to_string(),
    );
    if emitter_types != 0 {
        return Ok(CaseReport::fail(
            CASE,
            "emitter-adjacent type in public API".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "the design's adversarial 'the emitter itself errors mid-run' has \
         no target: with no emitter, no run can continue unaudited OR fail \
         closed on emitter error — the choice the design demands does not exist"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"emitter_api_types": 0}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "no_audit_emitter_in_sources" => case_no_audit_emitter_in_sources(),
        "transitions_produce_no_events" => case_transitions_produce_no_events(),
        "unusual_paths_also_silent" => case_unusual_paths_also_silent(),
        "emitter_failure_has_no_target" => case_emitter_failure_has_no_target(),
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
        "recon: emitter-vocabulary scan over every crates/*/src — zero hits (no event emitter, no emission at any transition site, no audit sink type)".to_string(),
        "recon: the one audit-adjacent writer (EvaluationRecord::record_event) is a manual opt-in recorder with zero production call sites — classified adjacent, not the seam".to_string(),
        "recon: the real promotion::Lifecycle and the real Evaluator stage machine transition with no event channel — transition() returns only the next state; validate/prepare/execute return Result<(), _>".to_string(),
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
        "finding: no audit event emitter exists in phlow — the design's pass criteria (every transition emits exactly one audit event; the event log is a complete record of the transition log) need an emitter to attach to, and there is none".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no audit event emitter exists in any phlow crate — a runtime vocabulary scan finds zero emitter-mechanism tokens, the one audit-adjacent writer (EvaluationRecord::record_event) is manual with zero production call sites, and the real promotion::Lifecycle plus the real Evaluator stage machine transition with no event channel (transition() returns only the next state; validate/prepare/execute return Result<(), _>). The design's pass criteria (every transition emits exactly one audit event; the event log is a complete record of the transition log, asserted event-for-event; unusual paths still emit; emitter failure fails the run closed) need an emitter to attach to, and there is none.".to_string(),
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
