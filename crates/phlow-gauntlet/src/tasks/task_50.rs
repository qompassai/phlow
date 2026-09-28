//! task-50: bounded event buffers (rust).
//!
//! Recon probe: the design asks for the internal producer/consumer event
//! bus buffer with a drop policy that is honest about loss — oldest
//! events dropped under overflow *with a monotonic dropped-counter* the
//! consumer can observe, memory bounded by an explicit constant, the
//! drop policy a named documented choice. The adversarial scenarios are
//! sustained overflow (counter stays exact, memory stays flat) and
//! critical event types (never dropped if a priority policy exists —
//! documented either way).
//!
//! Honest result: the seam is ABSENT. The evidence is gathered at probe
//! time from the live working tree and from driving the real
//! [`phlow_tuios::mailbox::Mailbox`]:
//!
//! 1. Evict-style buffers exist and bound memory, but none counts
//!    drops. The mailbox ring (`phlow-tuios/src/mailbox.rs`) evicts
//!    oldest-first under a count bound (`RING_MESSAGES_MAX` = 256) and
//!    a byte bound (`RING_TEXT_BYTES_MAX` = 512 KiB); the struct carries
//!    `messages`, `text_bytes`, `next_id`, `senders` — no dropped
//!    counter, and no method exposes one (checked in V1). The agent
//!    context window and the fusion decision log evict oldest-first the
//!    same way, with no accounting either.
//! 2. The dropped-counter vocabulary scan (`drop_count`,
//!    `dropped_count`, `evicted_count`, `total_dropped`, `num_dropped`,
//!    `dropped_total`) returns zero across the workspace.
//! 3. There is no producer/consumer event bus between orchestration
//!    stages to attach the policy to (re-verified in V2; task-35 found
//!    events are an opaque in-memory `Vec<String>` with no bus and no
//!    delivery ids).
//! 4. The behavioral demonstration (A1) drives the real mailbox past
//!    its count bound: 600 sends, the ring holds 256, ids stay
//!    monotonic — 344 messages vanish silently. The only way to observe
//!    the loss is to infer it from id gaps; the buffer reports nothing.
//! 5. The priority demonstration (A2) shows eviction is oldest-first
//!    regardless of `MessageKind`: an early `Ask` record is evicted by
//!    a later flood of `Notice`s. No priority policy exists — documented
//!    here, as the design requires.
//!
//! Four cases: two validation, two adversarial. The task-level verdict
//! is `fail` at `"seam"` — the design's pass criteria (memory bounded
//! by an explicit constant AND the dropped counter exact AND the drop
//! policy a named documented choice) need a buffer with exact drop
//! accounting, and none exists. The mailbox bounds memory but is silent
//! about loss — the exact failure mode the design's "no silent loss"
//! criterion forbids.
//!
//! Banked for Matt (product decision, NOT auto-implemented): whether to
//! add an explicit dropped counter to the mailbox ring (and/or a real
//! bounded event bus with one), and whether any event kind deserves
//! eviction priority.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_tuios::mailbox::{Mailbox, MessageKind, RING_MESSAGES_MAX};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Task id.
pub const ID: &str = "task-50";
/// Human-readable name.
pub const NAME: &str = "bounded event buffers";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "evict_buffers_drop_silently",
    "no_producer_consumer_event_bus",
    "sustained_overflow_loss_is_silent",
    "critical_kinds_are_dropped_too",
];

/// Largest Rust source file the probe will scan, in bytes.
const SOURCE_BYTES_MAX: usize = 1_048_576;
/// Most source files the probe will scan before stopping.
const SOURCE_FILES_MAX: usize = 50_000;
/// Messages pushed in the overflow demonstration: past the ring bound.
const OVERFLOW_SENDS: usize = 600;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-50 driver itself (not of the code under test).
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
                write!(f, "task-50: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-50: cannot probe {what}: {detail}")
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
// Fixtures: the working tree and the real mailbox are the task source
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

/// This probe's own source file, excluded from scan hits by exact path:
/// the probe prose must name the dropped-counter vocabulary it scans for.
fn own_source_file() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("tasks")
        .join("task_50.rs")
}

/// Walk `crates/` under the workspace root and return `path:line` hits
/// for `.rs` files inside a `src` tree whose alphanumeric-token stream
/// contains `token` (case-insensitive, exact token — not a substring).
/// Skips the probe's own source file by exact path. Bounded: files over
/// [`SOURCE_BYTES_MAX`] are skipped, and the walk stops after
/// [`SOURCE_FILES_MAX`] files.
fn scan_sources(root: &Path, token: &str) -> Result<Vec<String>, DriverError> {
    let own = own_source_file();
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
                && path != own
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
                        .split(|c: char| !c.is_alphanumeric())
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

/// Send one message from a distinct sender (each sender gets its own
/// rate-limiter budget, so the flood is the ring bound under test, not
/// the rate limiter).
fn send_one(
    mailbox: &mut Mailbox,
    kind: MessageKind,
    from: &str,
    text: &str,
) -> Result<u64, DriverError> {
    mailbox
        .send(
            kind,
            from,
            None,
            "gauntlet",
            text,
            &[],
            false,
            Instant::now(),
        )
        .map_err(|e| probe_error("mailbox send", e))
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

/// V1: the evict buffers exist and bound memory, but none counts drops.
/// The dropped-counter vocabulary scan returns zero workspace-wide, and
/// the `Mailbox` struct/impl carries no drop-counting field or method
/// (checked against the live source: the struct holds `messages`,
/// `text_bytes`, `next_id`, `senders` only).
fn case_evict_buffers_drop_silently() -> Result<CaseReport, DriverError> {
    const CASE: &str = "evict_buffers_drop_silently";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let counter_hits = scan_tokens(
        &root,
        &[
            "drop_count",
            "dropped_count",
            "evicted_count",
            "total_dropped",
            "num_dropped",
            "dropped_total",
        ],
    )?;
    evidence.push(format!(
        "dropped-counter vocabulary hits workspace-wide: {}",
        counter_hits.len()
    ));
    for hit in &counter_hits {
        evidence.push(format!("hit: {hit}"));
    }
    if !counter_hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "dropped-counter vocabulary found: {}",
                counter_hits.join("; ")
            ),
            evidence,
        ));
    }
    // Structural check on the real buffer: the Mailbox struct and its
    // inherent impl must name no drop-counting field or method.
    let mailbox_rs = std::fs::read_to_string(
        root.join("crates")
            .join("phlow-tuios")
            .join("src")
            .join("mailbox.rs"),
    )
    .map_err(|e| fixture_error("mailbox.rs", e))?;
    let struct_fields = ["messages", "text_bytes", "next_id", "senders"];
    for field in struct_fields {
        if !mailbox_rs.contains(field) {
            return Ok(CaseReport::fail(
                CASE,
                format!("Mailbox lost its '{field}' field — re-audit needed"),
                evidence,
            ));
        }
    }
    let impl_block = mailbox_rs
        .split("impl Mailbox")
        .nth(1)
        .unwrap_or("")
        .split("fn validate_send")
        .next()
        .unwrap_or("");
    let drop_methods: Vec<&str> = impl_block
        .lines()
        .filter(|l| l.contains("fn ") && l.to_lowercase().contains("drop"))
        .collect();
    evidence.push(format!(
        "Mailbox struct fields: messages/text_bytes/next_id/senders (no dropped counter); inherent methods mentioning 'drop': {}",
        drop_methods.len()
    ));
    if !drop_methods.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!("Mailbox gained a drop-related method: {drop_methods:?}"),
            evidence,
        ));
    }
    evidence.push(
        "the ring bounds memory (count + byte caps) but eviction is uncounted — loss is silent, not observed"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"counter_tokens": 6, "counter_hits": 0}),
        evidence,
    ))
}

/// V2: there is no producer/consumer event bus between orchestration
/// stages to attach the design's buffer to. Re-verifies task-35's
/// finding against the live tree: events are an opaque in-memory
/// `Vec<String>` with no bus and no delivery ids.
fn case_no_producer_consumer_event_bus() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_producer_consumer_event_bus";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let bus_hits = scan_tokens(&root, &["event_bus", "EventBus", "event_channel"])?;
    evidence.push(format!("event-bus vocabulary hits: {}", bus_hits.len()));
    for hit in &bus_hits {
        evidence.push(format!("hit: {hit}"));
    }
    if !bus_hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!("event-bus vocabulary found: {}", bus_hits.join("; ")),
            evidence,
        ));
    }
    evidence.push(
        "no event bus exists between orchestration stages (task-35: events are an opaque in-memory Vec<String>) — the design's buffer has no bus to live on"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"event_bus_hits": 0}),
        evidence,
    ))
}

/// A1: sustained overflow against the REAL mailbox. 600 sends past the
/// 256-message bound: the ring holds 256 (memory bounded ✓), ids stay
/// monotonic 1..=600 — and 344 messages are gone with no counter to say
/// so. The driver computes the loss from id gaps, proving the consumer
/// must infer what the buffer will not report.
fn case_sustained_overflow_loss_is_silent() -> Result<CaseReport, DriverError> {
    const CASE: &str = "sustained_overflow_loss_is_silent";
    let mut mailbox = Mailbox::new();
    let mut evidence = Vec::new();
    let mut first_id = 0u64;
    let mut last_id = 0u64;
    for i in 0..OVERFLOW_SENDS {
        let id = send_one(
            &mut mailbox,
            MessageKind::Notice,
            &format!("producer-{i}"),
            "overflow filler",
        )?;
        if i == 0 {
            first_id = id;
        }
        last_id = id;
    }
    let len = mailbox.len();
    evidence.push(format!(
        "sent {OVERFLOW_SENDS} (ids {first_id}..={last_id}), ring holds {len} (bound {RING_MESSAGES_MAX})"
    ));
    if first_id != 1 || last_id != OVERFLOW_SENDS as u64 {
        return Ok(CaseReport::fail(
            CASE,
            format!("ids not monotonic 1..={OVERFLOW_SENDS}: {first_id}..={last_id}"),
            evidence,
        ));
    }
    if len > RING_MESSAGES_MAX {
        return Ok(CaseReport::fail(
            CASE,
            format!("ring exceeded its bound: {len} > {RING_MESSAGES_MAX}"),
            evidence,
        ));
    }
    let inferred_dropped = OVERFLOW_SENDS - len;
    evidence.push(format!(
        "{inferred_dropped} messages evicted with no counter — the loss is computable only from id gaps, never reported by the buffer"
    ));
    // The case passes as a probe: memory IS bounded (honest credit), but
    // the design's "no silent loss" criterion fails — and that is the
    // task-level finding, not a case failure.
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({
            "sent": OVERFLOW_SENDS,
            "ring_len": len,
            "ring_bound": RING_MESSAGES_MAX,
            "inferred_dropped": inferred_dropped,
            "dropped_counter_exposed": false,
        }),
        evidence,
    ))
}

/// A2: no priority policy exists — documented, as the design requires.
/// An early `Ask` record (agent-coordination history, the closest thing
/// to a critical event kind) is evicted by a later flood of `Notice`s:
/// eviction is oldest-first regardless of kind.
fn case_critical_kinds_are_dropped_too() -> Result<CaseReport, DriverError> {
    const CASE: &str = "critical_kinds_are_dropped_too";
    let mut mailbox = Mailbox::new();
    let mut evidence = Vec::new();
    let ask_id = send_one(
        &mut mailbox,
        MessageKind::Ask,
        "agent-0",
        "critical ask record",
    )?;
    for i in 0..(RING_MESSAGES_MAX + 10) {
        send_one(
            &mut mailbox,
            MessageKind::Notice,
            &format!("flooder-{i}"),
            "notice flood",
        )?;
    }
    let remaining: Vec<u64> = {
        // `read` needs `&mut self`; collect ids via a peek read.
        use phlow_tuios::mailbox::ReadFilter;
        let filter = ReadFilter {
            inbox: None,
            unread: false,
            notices: true,
            thread: None,
            limit: Some(RING_MESSAGES_MAX + 10),
            peek: true,
        };
        mailbox.read(&filter).iter().map(|m| m.id()).collect()
    };
    evidence.push(format!(
        "early Ask id={ask_id}; after {} notices the ring holds ids {}..={} (len {})",
        RING_MESSAGES_MAX + 10,
        remaining.first().copied().unwrap_or(0),
        remaining.last().copied().unwrap_or(0),
        remaining.len()
    ));
    if remaining.contains(&ask_id) {
        return Ok(CaseReport::fail(
            CASE,
            "the early Ask survived the flood — a priority policy exists after all".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "documented: no priority policy — eviction is oldest-first regardless of MessageKind, so critical kinds are dropped exactly like the rest, silently"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"ask_evicted": true, "priority_policy": false}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "evict_buffers_drop_silently" => case_evict_buffers_drop_silently(),
        "no_producer_consumer_event_bus" => case_no_producer_consumer_event_bus(),
        "sustained_overflow_loss_is_silent" => case_sustained_overflow_loss_is_silent(),
        "critical_kinds_are_dropped_too" => case_critical_kinds_are_dropped_too(),
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
        "recon: evict-style buffers exist (mailbox ring with count + byte bounds, agent context window, fusion decision log) but none counts drops — the dropped-counter vocabulary scan returns zero workspace-wide, and Mailbox carries no drop-counting field or method".to_string(),
        "recon: no producer/consumer event bus exists between orchestration stages (task-35's finding re-verified) — the design's buffer has no bus to live on".to_string(),
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
        "finding: the closest buffer (the mailbox ring) bounds memory but drops silently — 344 of 600 overflow messages vanished with no counter, inferable only from id gaps; eviction is oldest-first regardless of kind, so critical event kinds are dropped like the rest".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no bounded producer/consumer event buffer with exact drop accounting exists in any phlow crate — the dropped-counter vocabulary scan (drop_count, dropped_count, evicted_count, total_dropped, num_dropped, dropped_total) returns zero workspace-wide, the Mailbox ring carries no drop-counting field or method, and no event bus exists between orchestration stages. Driving the real mailbox past its 256-message bound shows the gap behaviorally: 600 sends, ring holds 256, 344 messages vanish silently — inferable only from id gaps, never reported. The design's pass criteria (memory bounded by an explicit constant AND the dropped counter exact with no silent loss AND a named documented drop policy) need a buffer with exact accounting, and there is none.".to_string(),
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
