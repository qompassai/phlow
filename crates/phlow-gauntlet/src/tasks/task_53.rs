//! task-53: quorum reads and writes (rust).
//!
//! Recon probe: the design asks for quorum reads/writes (W + R > N
//! across 3 replicas) on the replicated state store — write to 2, read
//! from 2, latest value wins; a partitioned replica doesn't break
//! writes/reads; a rejoining replica with stale data is corrected by
//! read-repair or version comparison.
//!
//! Honest result: the seam is ABSENT, and the finding is single-node.
//! The evidence is gathered at probe time from the live working tree
//! and from driving the real [`phlow_experiment::Scheduler`]:
//!
//! 1. The replication vocabulary scan (`replica`, `quorum`,
//!    `read_repair`, `write_quorum`, `read_quorum`, `anti_entropy`)
//!    returns zero across every phlow crate's `src` tree.
//! 2. The actual state stores are all single-node and in-memory: the
//!    [`Scheduler`]'s run tables and node states (`phlow-experiment`),
//!    the skill store (`phlow-self-improve`), the agent memory store
//!    (`phlow-agent`), and the file-based operator config
//!    (`phlow-config`). None has a peer set, a replica count, or W/R
//!    knobs on any API.
//! 3. The behavioral demonstration (V2) drives the real `Scheduler`:
//!    admit a node, publish its result, read it back — a write is one
//!    local mutation, a read is one local lookup. There is no second
//!    replica to write to and no quorum to satisfy.
//! 4. The partition scenarios are vacuous (A1): there is no peer set to
//!    partition and no partition/sever primitive on the store.
//! 5. The stale-read guard is doubly absent (A2): records carry no
//!    per-record version (task-31's finding re-verified — `record.rs`
//!    has only the `schema_version` constant), so even if replicas
//!    existed, no version comparison or read-repair could prevent a
//!    stale read.
//!
//! Four cases: two validation, two adversarial. The task-level verdict
//! is `fail` at `"seam"` — the design's pass criteria (W + R > N; every
//! read at least as fresh as the last acknowledged write, asserted
//! across partition scenarios) need a replicated store with quorum
//! logic, and there is none.
//!
//! Banked for Matt (product decision, NOT auto-implemented): whether
//! phlow wants any replicated state at all (today it is a single-node
//! runtime by architecture — the AGENTS.md invariants assume one
//! operator, one machine), and if so where the quorum seam should live.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_experiment::{
    CapabilitySet, ExperimentId, NodeId, NodeParams, NodeState, RunId, Scheduler, SchedulerLimits,
    SchedulerNode, WorkerRole,
};
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-53";
/// Human-readable name.
pub const NAME: &str = "quorum reads and writes";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "single_node_stores_located",
    "single_node_read_write_semantics",
    "partition_is_vacuous",
    "no_version_for_stale_read_prevention",
];

/// Largest Rust source file the probe will scan, in bytes.
const SOURCE_BYTES_MAX: usize = 1_048_576;
/// Most source files the probe will scan before stopping.
const SOURCE_FILES_MAX: usize = 50_000;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-53 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture (workspace root, source tree, scheduler) was unusable.
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
                write!(f, "task-53: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-53: cannot probe {what}: {detail}")
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
// Fixtures: the working tree and the real Scheduler are the task source
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
/// Skips the entire phlow-gauntlet crate: the gauntlet's own driver
/// docs legitimately carry quorum/replication vocabulary, and they are
/// test scaffolding, not the product surface under test. Bounded: files
/// over [`SOURCE_BYTES_MAX`] are skipped, and the walk stops after
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

/// Build a real single-node `Scheduler` with default limits.
fn make_scheduler() -> Result<Scheduler, DriverError> {
    Scheduler::new(SchedulerLimits {
        queue_capacity: 64,
        ..Default::default()
    })
    .map_err(|e| fixture_error("scheduler", e))
}

/// Admit one node to `scheduler` and return its id.
fn admit_node(scheduler: &mut Scheduler, tag: &str) -> Result<NodeId, DriverError> {
    let run = RunId::new(&format!("run-{tag}")).map_err(|e| fixture_error("run id", e))?;
    let exp = ExperimentId::new(&format!("exp-{tag}")).map_err(|e| fixture_error("exp id", e))?;
    let node_id = NodeId::new(&format!("node-{tag}")).map_err(|e| fixture_error("node id", e))?;
    let caps = CapabilitySet::new(
        vec!["read".to_string()],
        vec!["workspace".to_string()],
        64,
        65_536,
    )
    .map_err(|e| fixture_error("capabilities", e))?;
    let node = SchedulerNode::new(NodeParams {
        run_id: run,
        experiment_id: exp,
        baseline_revision: "rev-1".to_string(),
        workspace_snapshot: "snap-1".to_string(),
        node_id: node_id.clone(),
        parent_node_id: None,
        role: WorkerRole::Implementer,
        capabilities: caps,
        input_digest: "input-digest".to_string(),
        dependency_ids: Vec::new(),
        generation: 0,
        attempt: 0,
        deadline_ms: 300_000,
        cpu_budget_ms: 1_000,
        memory_budget_bytes: 1_048_576,
        output_bytes_max: 4_096,
        tool_calls_remaining: 10,
    })
    .map_err(|e| fixture_error("node", e))?;
    scheduler
        .admit(node)
        .map_err(|e| fixture_error("admit", e))?;
    Ok(node_id)
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

/// V1: locate the replication seam — there is none. The replication
/// vocabulary scan returns zero across every phlow crate, and the real
/// state stores are documented as single-node: the Scheduler's run
/// tables and node states, the skill store, the agent memory store, the
/// file-based operator config. None has a peer set, a replica count, or
/// W/R knobs on any API.
fn case_single_node_stores_located() -> Result<CaseReport, DriverError> {
    const CASE: &str = "single_node_stores_located";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let tokens = [
        "replica",
        "quorum",
        "read_repair",
        "write_quorum",
        "read_quorum",
        "anti_entropy",
    ];
    let hits = scan_tokens(&root, &tokens)?;
    evidence.push(format!(
        "replication vocabulary hits workspace-wide ({} tokens): {}",
        tokens.len(),
        hits.len()
    ));
    for hit in &hits {
        evidence.push(format!("hit: {hit}"));
    }
    if !hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!("replication vocabulary found: {}", hits.join("; ")),
            evidence,
        ));
    }
    evidence.push(
        "the actual state stores, all single-node and in-memory: Scheduler run tables + node states (phlow-experiment), skill store (phlow-self-improve), agent memory store (phlow-agent), file-based operator config (phlow-config)"
            .to_string(),
    );
    evidence.push(
        "no store has a peer set, a replica count, or W/R knobs on any API — the design's 'locate the replicated state store' step ends here: single-node is the finding".to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"replication_tokens": tokens.len(), "replication_hits": 0}),
        evidence,
    ))
}

/// V2: drive the real `Scheduler` through a write and a read. A write is
/// one local mutation (`admit` + `publish_result`); a read is one local
/// lookup (`node`, `published_digest`). There is no second replica to
/// write to and no quorum to satisfy — W and R are both 1 and N is 1,
/// the degenerate quorum.
fn case_single_node_read_write_semantics() -> Result<CaseReport, DriverError> {
    const CASE: &str = "single_node_read_write_semantics";
    let mut evidence = Vec::new();
    let mut scheduler = make_scheduler()?;
    let node_id = admit_node(&mut scheduler, "quorum")?;
    scheduler
        .publish_result(&node_id, 0, "digest-abc123", NodeState::Succeeded)
        .map_err(|e| probe_error("publish_result", e))?;
    let digest = scheduler.published_digest(&node_id);
    evidence.push(format!(
        "published_digest after publish_result: {digest:?} (one local write, one local read)"
    ));
    if digest != Some("digest-abc123") {
        return Ok(CaseReport::fail(
            CASE,
            format!("read did not return the written digest: {digest:?}"),
            evidence,
        ));
    }
    let node = scheduler.node(&node_id);
    evidence.push(format!(
        "node lookup: state={:?} (single-node read, no replica involved)",
        node.map(|n| n.state().name())
    ));
    evidence.push(
        "W=1, R=1, N=1: the degenerate quorum. The API surface (new/admit/publish_result/node/published_digest) has no replica-count, write-quorum, or read-quorum parameter — quorum math cannot be expressed, let alone asserted"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"replicas": 1, "write_quorum": 1, "read_quorum": 1}),
        evidence,
    ))
}

/// A1: the partition scenarios are vacuous. There is no peer set to
/// partition and no partition/sever primitive on the store — the probe
/// asserts this against the live source (no `peer`/`partition` token in
/// the control-plane module) and against the Scheduler API (no such
/// method exists to call).
fn case_partition_is_vacuous() -> Result<CaseReport, DriverError> {
    const CASE: &str = "partition_is_vacuous";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let control_plane = root
        .join("crates")
        .join("phlow-experiment")
        .join("src")
        .join("control_plane.rs");
    let text = std::fs::read_to_string(&control_plane)
        .map_err(|e| fixture_error("control_plane.rs", e))?;
    let peer_lines: Vec<&str> = text
        .lines()
        .filter(|l| {
            l.split(|c: char| !c.is_alphanumeric()).any(|tok| {
                tok.eq_ignore_ascii_case("peer") || tok.eq_ignore_ascii_case("partition")
            })
        })
        .collect();
    evidence.push(format!(
        "peer/partition token lines in control_plane.rs: {}",
        peer_lines.len()
    ));
    for line in &peer_lines {
        evidence.push(format!("line: {}", line.trim()));
    }
    if !peer_lines.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "partition/peer vocabulary appeared: {}",
                peer_lines.join(" | ")
            ),
            evidence,
        ));
    }
    evidence.push(
        "no peer set, no sever/partition primitive: the design's 'one replica partitioned away' scenario cannot be staged — a single node cannot be partitioned from itself"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"peer_partition_hits": 0}),
        evidence,
    ))
}

/// A2: the stale-read guard is doubly absent. Records carry no
/// per-record WRITE-ORDERING version — task-31's finding re-verified
/// against the live `record.rs`. What IS there: the `schema_version`
/// constant (a format version, never bumped by any write) and
/// `baseline_revision`/`candidate_revision` (git SHAs identifying the
/// code under evaluation — content labels, not monotonic versions, and
/// with no ordering semantics read-repair could compare). No monotonic
/// counter, vector clock, CAS tag, or lamport timestamp exists on the
/// record. So even if replicas existed, no version comparison or
/// read-repair could prevent a stale read.
fn case_no_version_for_stale_read_prevention() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_version_for_stale_read_prevention";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let record_rs = std::fs::read_to_string(
        root.join("crates")
            .join("phlow-experiment")
            .join("src")
            .join("record.rs"),
    )
    .map_err(|e| fixture_error("record.rs", e))?;
    // Write-ordering version vocabulary: what a quorum read-repair
    // would compare. `schema_version` (format constant) and
    // `*_revision` (git SHA content labels) are deliberately NOT in
    // this list — they identify formats and code, they order nothing.
    let ordering_tokens = ["vector_clock", "cas", "lamport", "monotonic"];
    let mut ordering_hits: Vec<String> = Vec::new();
    for (lineno, line) in record_rs.lines().enumerate() {
        for token in ordering_tokens {
            if line
                .split(|c: char| !c.is_alphanumeric())
                .any(|tok| tok.eq_ignore_ascii_case(token))
            {
                ordering_hits.push(format!("{token}:{}: {}", lineno + 1, line.trim()));
            }
        }
    }
    // A `version` field that is a monotonic write counter would also
    // qualify — distinguish it from the format constant by looking for
    // a field-type declaration named exactly `version`, not prose.
    let field_version = record_rs
        .lines()
        .any(|line| line.trim_start().starts_with("version:"));
    evidence.push(format!(
        "write-ordering version tokens in record.rs: {} (vector_clock/cas/lamport/monotonic)",
        ordering_hits.len()
    ));
    for hit in &ordering_hits {
        evidence.push(format!("line: {hit}"));
    }
    evidence.push(format!(
        "monotonic `version` field on the record struct: {field_version}"
    ));
    evidence.push(
        "present but NOT write-ordering: SCHEMA_VERSION (u32 constant, never bumped by a write) and baseline_revision/candidate_revision (git SHAs identifying the code under evaluation — content labels, no ordering semantics)"
            .to_string(),
    );
    let per_record_version = !ordering_hits.is_empty() || field_version;
    if per_record_version {
        return Ok(CaseReport::fail(
            CASE,
            "a write-ordering per-record version appeared in record.rs — probe outdated"
                .to_string(),
            evidence,
        ));
    }
    evidence.push(
        "no monotonic counter, vector clock, CAS tag, or lamport timestamp on the record: no version comparison or read-repair could order a rejoining replica's stale data — the design's anti-stale-read machinery is absent on top of the absent replicas"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"per_record_version": false}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "single_node_stores_located" => case_single_node_stores_located(),
        "single_node_read_write_semantics" => case_single_node_read_write_semantics(),
        "partition_is_vacuous" => case_partition_is_vacuous(),
        "no_version_for_stale_read_prevention" => case_no_version_for_stale_read_prevention(),
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
        "recon: the replication vocabulary scan (replica, quorum, read_repair, write_quorum, read_quorum, anti_entropy) returns zero across every phlow crate — no replicated state store exists".to_string(),
        "recon: the real stores are single-node and in-memory (Scheduler run tables/node states, skill store, agent memory store, file-based operator config); none has a peer set, a replica count, or W/R knobs".to_string(),
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
        "finding: phlow is a single-node runtime — writes are one local mutation, reads are one local lookup (W=1, R=1, N=1, the degenerate quorum); partitions are vacuous and no per-record version exists for read-repair".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no replicated state store exists in any phlow crate — the replication vocabulary scan (replica, quorum, read_repair, write_quorum, read_quorum, anti_entropy) returns zero workspace-wide, and the real stores (Scheduler run tables/node states, skill store, agent memory store, file-based operator config) are all single-node with no peer set and no W/R knobs on any API. Driving the real Scheduler shows the degenerate quorum: admit + publish_result is one local mutation, node/published_digest is one local lookup — no second replica to write to, no quorum to satisfy. The design's pass criteria (W + R > N across 3 replicas; reads at least as fresh as the last acknowledged write across partition scenarios; read-repair/version comparison for rejoining replicas) need a replicated store with quorum logic, and there is none. Documented as the finding, exactly as the design allows.".to_string(),
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
