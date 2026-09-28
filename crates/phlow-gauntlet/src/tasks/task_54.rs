//! task-54: gossip convergence (rust).
//!
//! Recon probe: the design asks for gossip convergence on the cluster
//! membership / config dissemination path — one worker learns new
//! config and all converge within N rounds without a central
//! coordinator; a partitioned worker converges after rejoin with no
//! permanent split; two racing conflicting updates converge to one
//! value (last-writer-wins with a documented tiebreak, never a
//! permanent fork).
//!
//! Honest result: the seam is ABSENT, and the finding is single-node
//! with no dissemination at all. The evidence is gathered at probe time
//! from the live working tree and from driving the real
//! [`phlow_config::load_config`]:
//!
//! 1. The module/type-level gossip-seam search (gossip/dissemination/
//!    swim/anti-entropy modules or declared types) returns zero across
//!    every phlow crate's `src` tree. Raw token greps are NOT the
//!    evidence: "membership"/"peer" appear in product code with
//!    unrelated meanings (JSON-schema enum membership, scoring-pool
//!    members, RPC framing peers) — documented as false friends.
//! 2. The behavioral demonstration (V2) loads two different operator
//!    configs through the real loader: each load is independent and
//!    per-process — there are no rounds, no peers, no convergence
//!    protocol. Config is explicit operator-supplied TOML (the
//!    AGENTS.md invariant), not disseminated state.
//! 3. The partition scenario is vacuous (A1): with no membership there
//!    is no peer to partition and no rejoin to converge from.
//! 4. The conflicting-updates scenario (A2) shows the two loads never
//!    converge — each process keeps its own value; there is no
//!    last-writer-wins-with-tiebreak protocol between them, only the
//!    operator's file if both wrote one.
//!
//! Four cases: two validation, two adversarial. The task-level verdict
//! is `fail` at `"seam"` — the design's pass criteria (convergence
//! within N measured rounds; no two workers disagree forever) need a
//! dissemination protocol, and there is none.
//!
//! Distinct from task-30: election picks *one leader*; gossip spreads
//! *data* to all — different primitive, different liveness argument,
//! same absent-seam outcome.
//!
//! Banked for Matt (product decision, NOT auto-implemented): phlow is
//! a single-node runtime today — there is no cluster, so there is
//! nothing to gossip. If multi-node operation is ever wanted, the
//! membership/dissemination seam would need to be designed from
//! scratch.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_config::{FlowConfig, LoadOptions};
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-54";
/// Human-readable name.
pub const NAME: &str = "gossip convergence";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "no_gossip_vocabulary",
    "config_does_not_disseminate",
    "no_membership_primitive",
    "conflicting_updates_never_converge",
];

/// Largest Rust source file the probe will scan, in bytes.
const SOURCE_BYTES_MAX: usize = 1_048_576;
/// Most source files the probe will scan before stopping.
const SOURCE_FILES_MAX: usize = 50_000;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-54 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture (workspace root, source tree, config files) was unusable.
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
                write!(f, "task-54: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-54: cannot probe {what}: {detail}")
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
// Fixtures: the working tree and the real config loader are the source
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

/// Is `path` inside the phlow-gauntlet crate (test scaffolding, not the
/// product surface)?
fn is_gauntlet(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "phlow-gauntlet")
}

/// Module/type-level seam search: the honest "locate the seam" step.
/// Returns hits for (a) source file names containing any of
/// `file_fragments` (case-insensitive), and (b) `struct`/`enum`/`trait`
/// declarations whose name contains any of `type_fragments`. Both are
/// the shapes a real membership/dissemination seam would take in this
/// codebase; raw token greps for words like "member"/"peer" are
/// documented separately as false friends, never as seam evidence.
fn seam_name_scan(
    root: &Path,
    file_fragments: &[&str],
    type_fragments: &[&str],
) -> Result<Vec<String>, DriverError> {
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
                continue;
            }
            if is_gauntlet(&path) {
                continue;
            }
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if file_fragments.iter().any(|frag| file_name.contains(frag)) {
                hits.push(format!("module: {}", path.display()));
            }
            if path.extension().is_some_and(|e| e == "rs")
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
                for (lineno, line) in text.lines().enumerate() {
                    let trimmed = line.trim_start();
                    let is_decl = trimmed.starts_with("pub struct ")
                        || trimmed.starts_with("struct ")
                        || trimmed.starts_with("pub enum ")
                        || trimmed.starts_with("enum ")
                        || trimmed.starts_with("pub trait ")
                        || trimmed.starts_with("trait ");
                    if is_decl
                        && type_fragments
                            .iter()
                            .any(|frag| line.to_lowercase().contains(frag))
                    {
                        hits.push(format!(
                            "{}:{}: {}",
                            path.display(),
                            lineno + 1,
                            line.trim()
                        ));
                    }
                }
            }
        }
    }
    Ok(hits)
}

/// Scratch directory for the probe's config fixtures, unique per case
/// invocation so parallel tests never share fixture files.
///
/// A per-process directory is NOT enough: the four integration tests run
/// in parallel threads of one process, and every
/// `load_config_with_workspace` truncates and rewrites the same
/// `config-{tag}.toml` paths. A reader in another thread can observe a
/// fixture file between truncate and write, parse an empty TOML
/// document, and fall back to the process working directory instead of
/// the fixture's workspace — observed on primo as A2 failing with "a
/// load changed value between rounds without its file changing".
/// (The task-level `run` also re-executes every case in the calling
/// thread, so even sequential reuse across cases would collide.)
fn probe_scratch(case: &str) -> Result<PathBuf, DriverError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "gauntlet-task-54-{}-{case}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).map_err(|e| fixture_error("scratch dir", e))?;
    Ok(dir)
}

/// Write a minimal operator config whose `workspace_dir` is `ws_dir`,
/// and load it through the real [`phlow_config::load_config`].
fn load_config_with_workspace(scratch: &Path, tag: &str) -> Result<FlowConfig, DriverError> {
    let ws_dir = scratch.join(format!("ws-{tag}"));
    std::fs::create_dir_all(&ws_dir).map_err(|e| fixture_error("workspace dir", e))?;
    let config_path = scratch.join(format!("config-{tag}.toml"));
    // Minimal TOML: only workspace_dir. The loader validates it is an
    // existing directory and absolutizes it.
    let toml = format!("workspace_dir = \"{}\"\n", ws_dir.display());
    std::fs::write(&config_path, toml).map_err(|e| fixture_error("config write", e))?;
    phlow_config::load_config(&LoadOptions {
        config_path: Some(config_path),
        ..Default::default()
    })
    .map_err(|e| probe_error("load_config", e))
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

/// V1: locate the gossip/dissemination seam — module/type level. A
/// real gossip protocol would own a module (`gossip.rs`,
/// `dissemination.rs`, `swim.rs`) or a declared type (`Gossip`,
/// `Dissemination`, `AntiEntropy`, `Swim`). None exists in any phlow
/// crate. Raw token greps are deliberately NOT the evidence here: the
/// word "membership" appears in product code with unrelated meanings
/// (JSON-schema enum membership in phlow-mcp/src/schema.rs; two-stage
/// scoring-pool members in phlow-inference/src/two_stage.rs) — those
/// are documented false friends, not dissemination.
fn case_no_gossip_vocabulary() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_gossip_vocabulary";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let hits = seam_name_scan(
        &root,
        &["gossip", "disseminat", "swim", "anti_entropy"],
        &["gossip", "disseminat", "swim", "anti_entropy"],
    )?;
    evidence.push(format!(
        "module/type-level gossip-seam hits across phlow crates: {}",
        hits.len()
    ));
    for hit in &hits {
        evidence.push(format!("hit: {hit}"));
    }
    evidence.push(
        "false friends, verified unrelated: 'membership' in phlow-mcp/src/schema.rs is JSON-schema enum membership; 'members'/'membership' in phlow-inference/src/two_stage.rs is the two-stage scoring pool — neither moves config between processes"
            .to_string(),
    );
    if !hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!("gossip seam found: {}", hits.join("; ")),
            evidence,
        ));
    }
    evidence.push(
        "no cluster membership / config dissemination path exists to locate — the design's 'locate; document if absent' step ends here"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"gossip_seam_hits": 0}),
        evidence,
    ))
}

/// V2: drive the real config loader twice with different operator
/// configs. Each load is independent and per-process: there are no
/// rounds, no peers, no convergence protocol. Config is explicit
/// operator-supplied TOML (the AGENTS.md invariant), not disseminated
/// state — "one worker learns new config" has no mechanism to reach
/// any other worker.
fn case_config_does_not_disseminate() -> Result<CaseReport, DriverError> {
    const CASE: &str = "config_does_not_disseminate";
    let mut evidence = Vec::new();
    let scratch = probe_scratch(CASE)?;
    let config_a = load_config_with_workspace(&scratch, "a")?;
    let config_b = load_config_with_workspace(&scratch, "b")?;
    let ws_a = config_a.workspace_dir().to_string_lossy().to_string();
    let ws_b = config_b.workspace_dir().to_string_lossy().to_string();
    evidence.push(format!("load A: workspace_dir={ws_a}"));
    evidence.push(format!("load B: workspace_dir={ws_b}"));
    if ws_a == ws_b {
        return Ok(CaseReport::fail(
            CASE,
            "the two loads converged — a dissemination protocol exists after all".to_string(),
            evidence,
        ));
    }
    if !ws_a.ends_with("ws-a") || !ws_b.ends_with("ws-b") {
        return Ok(CaseReport::fail(
            CASE,
            format!("loads did not return their own files: {ws_a} vs {ws_b}"),
            evidence,
        ));
    }
    evidence.push(
        "each load returned exactly its own file's value: no rounds elapsed, no peers were contacted, no state moved between the loads — config does not disseminate".to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"loads": 2, "converged": false, "rounds": 0}),
        evidence,
    ))
}

/// A1: the partition-during-dissemination scenario is vacuous. With no
/// membership there is no peer to partition and no rejoin to converge
/// from — the probe asserts the absence of a membership primitive at
/// module/type level against the live tree. Raw token greps are
/// deliberately NOT the evidence: "peer" in phlow-runtime's msgpack
/// transport and phlow-tuios is RPC framing (a client/server peer that
/// must not make the daemon echo unbounded text) — point-to-point
/// transport, not a membership set — and "member" is the workspace
/// SKIP_DIRS list and the scoring pool. None is a join/leave primitive.
fn case_no_membership_primitive() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_membership_primitive";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let hits = seam_name_scan(
        &root,
        &["member", "cluster"],
        &["member", "cluster", "join_cluster", "leave_cluster"],
    )?;
    evidence.push(format!(
        "module/type-level membership-seam hits across phlow crates: {}",
        hits.len()
    ));
    for hit in &hits {
        evidence.push(format!("hit: {hit}"));
    }
    evidence.push(
        "false friends, verified unrelated: 'peer' in phlow-runtime/src/transport/msgpack.rs and phlow-tuios is msgpack-RPC framing (client/server peers; 'a hostile peer cannot make the daemon echo unbounded text') — transport, not cluster membership; 'member' in phlow-workspace/src/workspace.rs is the SKIP_DIRS list and in phlow-inference the scoring pool — neither is a join/leave primitive"
            .to_string(),
    );
    if !hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!("membership seam found: {}", hits.join("; ")),
            evidence,
        ));
    }
    evidence.push(
        "no membership primitive, no join/leave, no peer set: the design's 'worker partitioned during dissemination, converges after rejoin' has no worker set to partition and no rejoin to converge from — no permanent split is possible because there is no split at all".to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"membership_seam_hits": 0}),
        evidence,
    ))
}

/// A2: two racing conflicting updates never converge. The two real
/// loads from V2 keep their own values indefinitely — there is no
/// last-writer-wins-with-tiebreak protocol between processes, only the
/// operator's file if both wrote the same one. The design's "converge
/// to one value, never a permanent fork" is not violated by forking;
/// it is vacuous, because there is no shared state to fork.
fn case_conflicting_updates_never_converge() -> Result<CaseReport, DriverError> {
    const CASE: &str = "conflicting_updates_never_converge";
    let mut evidence = Vec::new();
    let scratch = probe_scratch(CASE)?;
    // Two "workers" learn conflicting configs "at the same time".
    let config_a = load_config_with_workspace(&scratch, "a")?;
    let config_b = load_config_with_workspace(&scratch, "b")?;
    // "Rounds" pass: re-read both. Nothing moves.
    let config_a2 = load_config_with_workspace(&scratch, "a")?;
    let config_b2 = load_config_with_workspace(&scratch, "b")?;
    let (wa, wb) = (
        config_a.workspace_dir().to_string_lossy().to_string(),
        config_b.workspace_dir().to_string_lossy().to_string(),
    );
    let (wa2, wb2) = (
        config_a2.workspace_dir().to_string_lossy().to_string(),
        config_b2.workspace_dir().to_string_lossy().to_string(),
    );
    evidence.push(format!("round 1: A={wa} B={wb}"));
    evidence.push(format!("round 2: A={wa2} B={wb2}"));
    if wa != wa2 || wb != wb2 {
        return Ok(CaseReport::fail(
            CASE,
            "a load changed value between rounds without its file changing".to_string(),
            evidence,
        ));
    }
    if wa == wb {
        return Ok(CaseReport::fail(
            CASE,
            "conflicting updates converged — a dissemination protocol exists after all".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "the two workers disagree forever and nothing converges them: no LWW tiebreak, no rounds, no liveness argument — the design's convergence criterion has no protocol to measure. This is not a fork to heal; there is no shared state at all"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"workers": 2, "rounds_observed": 2, "converged": false, "tiebreak_protocol": false}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "no_gossip_vocabulary" => case_no_gossip_vocabulary(),
        "config_does_not_disseminate" => case_config_does_not_disseminate(),
        "no_membership_primitive" => case_no_membership_primitive(),
        "conflicting_updates_never_converge" => case_conflicting_updates_never_converge(),
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
        "recon: the module/type-level gossip-seam search (gossip/dissemination/swim/anti_entropy modules or declared types) returns zero across every phlow crate — no cluster membership or config dissemination path exists; raw 'membership'/'peer' tokens are verified false friends (enum membership, scoring pool, RPC framing)".to_string(),
        "recon: config is explicit operator-supplied TOML loaded per process (AGENTS.md invariant) — the module/type-level membership search (member/cluster modules or declared types) also returns zero".to_string(),
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
        "finding: two real config loads never converge and no rounds exist — workers cannot disagree-then-converge because there is no shared state and no dissemination protocol; the partition and racing-update scenarios are vacuous".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no cluster membership / config dissemination path exists in any phlow crate — the module/type-level gossip-seam search (gossip/dissemination/swim/anti_entropy modules or declared types) and the membership search (member/cluster modules or declared types) both return zero across every phlow crate; raw 'membership'/'peer' tokens are verified false friends (JSON-schema enum membership, scoring-pool members, msgpack-RPC framing peers). Driving the real phlow_config::load_config twice with conflicting operator configs shows each load is independent and per-process: no rounds, no peers, no convergence — the two 'workers' disagree forever with nothing to converge them. The design's pass criteria (convergence within N measured rounds; no two workers disagree forever — a liveness assertion) need a dissemination protocol, and there is none. Documented as the finding, exactly as the design allows.".to_string(),
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
