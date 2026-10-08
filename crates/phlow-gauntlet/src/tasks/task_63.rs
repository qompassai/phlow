//! task-63: decomposition depth bound (rust).
//!
//! Recon probe: the design asks for the goal-decomposition routine —
//! the planner splitting a goal into subgoals — and a DEPTH bound on it:
//! an adversarial self-similar goal ("refine this plan", "break this
//! down further") must hit a named depth cap and return the partial
//! decomposition with `DepthExceeded`, never hang. Distinct from task-23
//! (fan-out bounds breadth — this bounds depth).
//!
//! Honest result: the seam is ABSENT. The decomposition vocabulary
//! scan (`decompose`, `decomposition`, `decomposer`, `subgoal`,
//! `subgoals`, `sub_goal`) returns zero across every phlow crate's `src`
//! tree, and `phlow-agent`'s public module list (read from the live
//! `lib.rs` at probe time) contains no planner or decomposer module.
//! What DOES exist under the "planner" name is the planner *role*: an
//! LLM role name in `phlow-config` (`ModelsConfig.planner`,
//! `ModelRole::Planner`) and prose ("planner -> coder -> verification ->
//! reviewer loop") in `phlow-agent/src/orchestrator.rs`, plus one
//! consumer of the role: the autoresearch proposer binding
//! (`phlow-autoresearch/src/ollama_proposer.rs`), which binds the
//! planner specialist to emit change-sets for the bounded experiment
//! loop — classified in V2 as a role consumer, not a routine.
//! `Orchestrator::run`
//! is a single pass into `Runtime::run` — one call, no goal-splitting
//! loop. There is no decomposition routine, so there is no depth to
//! bound and no cap to name; the adversarial self-similar goal has no
//! decomposer it could loop in.
//!
//! Four cases: two validation, two adversarial. The task-level verdict
//! is `fail` at `"seam"` — the design's pass criteria (always
//! terminates, proven by the cap; the cap is a named constant; partial
//! results at the cap are usable) need a decomposition routine with a
//! depth bound, and there is none. Fail-closed: if decomposition
//! machinery appears, the verdict flips to `where = "recon"` (premise
//! changed).
//!
//! Banked for Matt (product decision, NOT auto-implemented): whether
//! phlow wants goal decomposition at all (today `Orchestrator::run` is
//! a single LLM pass per user message), and if so where the
//! decomposition seam and its depth cap should live.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-63";
/// Human-readable name.
pub const NAME: &str = "decomposition depth bound";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "decompose_vocabulary_absent",
    "planner_is_role_not_routine",
    "self_similar_goal_has_no_decomposer",
    "fail_closed_if_decomposer_appears",
];

/// Largest Rust source file the probe will scan, in bytes.
const SOURCE_BYTES_MAX: usize = 1_048_576;
/// Most source files the probe will scan before stopping.
const SOURCE_FILES_MAX: usize = 50_000;

/// Adversarial self-similar goals from the design: engineered to make a
/// decomposer recurse forever. They have no decomposer to reach.
const ADVERSARIAL_GOALS: [&str; 3] = [
    "refine this plan",
    "break this down further",
    "decompose this goal into subgoals, then decompose each subgoal",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-63 driver itself (not of the code under test).
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
                write!(f, "task-63: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-63: cannot probe {what}: {detail}")
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
// Case report
// ---------------------------------------------------------------------------

/// One probe case's outcome.
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

/// Walk `crates/` under the workspace root and return `path:line` hits
/// for `.rs` files inside a `src` tree whose alphanumeric-token stream
/// contains `token` (case-insensitive, exact token — not a substring).
/// Skips the entire phlow-gauntlet crate: the gauntlet's own driver
/// docs legitimately carry decomposition vocabulary, and they are
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

/// Each hit carries its source line: `path:lineno: trimmed line`.
/// Used where the classification needs the line's content (control
/// samples), not just its location.
fn scan_tokens_with_lines(root: &Path, tokens: &[&str]) -> Result<Vec<String>, DriverError> {
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
                    for token in tokens {
                        let found = line
                            .split(|c: char| !c.is_alphanumeric())
                            .any(|tok| tok.eq_ignore_ascii_case(token));
                        if found {
                            hits.push(format!(
                                "{}:{}: {}",
                                path.display(),
                                lineno + 1,
                                line.trim()
                            ));
                            break;
                        }
                    }
                }
            }
        }
    }
    Ok(hits)
}

/// Split decomposition-vocabulary hits into control samples vs real
/// machinery. The known control sample is mixed-radix decomposition
/// (`phlow-compute/src/partition.rs` — number-theoretic/tensor-index
/// decomposition, "Mixed-radix decomposition without the bounds
/// check"): decomposition vocabulary, but not goal decomposition.
/// Classified by line content ("radix"), not by path.
fn classify_decompose_hits(hits: &[String]) -> (Vec<String>, Vec<String>) {
    let mut control = Vec::new();
    let mut real = Vec::new();
    for hit in hits {
        if hit.to_lowercase().contains("radix") {
            control.push(hit.clone());
        } else {
            real.push(hit.clone());
        }
    }
    (control, real)
}

/// Decomposition vocabulary: what a goal-splitting routine would be
/// named. Exact alphanumeric tokens, case-insensitive.
const DECOMPOSE_TOKENS: [&str; 6] = [
    "decompose",
    "decomposition",
    "decomposer",
    "subgoal",
    "subgoals",
    "sub_goal",
];

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// V1: locate the decomposition seam — there is none. The decomposition
/// vocabulary scan returns zero GOAL-decomposition hits across every
/// phlow crate's `src` tree. The one decomposition-vocabulary hit is a
/// verified control sample: mixed-radix (tensor-index) decomposition in
/// `phlow-compute/src/partition.rs` — number-theoretic decomposition,
/// not goal decomposition — classified by line content ("radix"), not
/// by path.
fn case_decompose_vocabulary_absent() -> Result<CaseReport, DriverError> {
    const CASE: &str = "decompose_vocabulary_absent";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let hits = scan_tokens_with_lines(&root, &DECOMPOSE_TOKENS)?;
    let (control, real) = classify_decompose_hits(&hits);
    evidence.push(format!(
        "decomposition vocabulary hits workspace-wide ({} tokens: {}): {} ({} control, {} real)",
        DECOMPOSE_TOKENS.len(),
        DECOMPOSE_TOKENS.join(", "),
        hits.len(),
        control.len(),
        real.len()
    ));
    for hit in &control {
        evidence.push(format!(
            "classified control sample (UNRELATED): {hit} — mixed-radix (tensor-index) \
             decomposition, not goal decomposition"
        ));
    }
    for hit in &real {
        evidence.push(format!("UNCLASSIFIED decomposition hit: {hit}"));
    }
    if !real.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            "goal-decomposition machinery appeared — probe outdated".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "no goal-decomposition routine exists in any phlow crate: the only decomposition \
         vocabulary is mixed-radix tensor-index decomposition (phlow-compute), a verified \
         control sample"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"hits": hits.len(), "control_samples": control.len(), "real": 0}),
        evidence,
    ))
}

/// V2: the "planner" in phlow is a ROLE, not a routine. The token scan
/// finds `planner` across the workspace — every hit file is verified
/// (not asserted) to be role-related: either the file also contains
/// the role-loop siblings `coder`/`reviewer` (the planner role never
/// appears without them in the runtime codebase), or it is the
/// autoresearch proposer binding — the experimental crate's consumer
/// of the planner specialist, classified by its `Proposer` binding.
/// And no planner-hit file contains any real decomposition-token hit
/// (no planner-decomposer coupling). A file failing every check is an
/// unclassified routine hit.
fn case_planner_is_role_not_routine() -> Result<CaseReport, DriverError> {
    const CASE: &str = "planner_is_role_not_routine";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let hits = scan_sources(&root, "planner")?;
    evidence.push(format!(
        "'planner' token hits workspace-wide: {}",
        hits.len()
    ));
    // Distinct files with planner hits.
    let mut files: Vec<String> = Vec::new();
    for hit in &hits {
        if let Some(path) = hit.rsplit_once(':').map(|(p, _)| p.to_string())
            && !files.contains(&path)
        {
            files.push(path);
        }
    }
    evidence.push(format!(
        "distinct files with 'planner' hits: {}",
        files.len()
    ));
    let decompose_hits = scan_tokens_with_lines(&root, &DECOMPOSE_TOKENS)?;
    let (_, real_decompose) = classify_decompose_hits(&decompose_hits);
    let mut routine_hits = 0usize;
    for file in &files {
        let text = std::fs::read_to_string(file)
            .map_err(|e| fixture_error("planner-hit file read", format!("{file}: {e}")))?;
        let low = text.to_lowercase();
        let has_siblings = low.contains("coder") || low.contains("reviewer");
        let has_decompose = real_decompose.iter().any(|h| h.starts_with(file.as_str()));
        // The autoresearch proposer binding is a CONSUMER of the
        // planner role inside the experimental crate: it binds the
        // [specialists] planner model to emit one change-set per loop
        // iteration. Not a planner/decomposer routine in the runtime.
        // The classification requires the `Proposer` binding in the
        // file and holds only while no real decomposition vocabulary
        // appears there (fail-closed: decomposition vocabulary
        // revokes it).
        let is_proposer_binding = (file.contains("phlow-autoresearch/src/ollama_proposer.rs")
            || file.contains("phlow-autoresearch/src/proposer.rs")
            || file.contains("phlow-autoresearch/src/lib.rs"))
            && text.contains("Proposer");
        if is_proposer_binding && !has_decompose {
            evidence.push(format!(
                "classified role consumer (autoresearch proposer binding surface of the \
                 planner specialist — trait seam, live binding, crate surface; Proposer \
                 binding present, no decomposition vocabulary): {file}"
            ));
        } else if has_siblings && !has_decompose {
            evidence.push(format!(
                "classified role site (coder/reviewer siblings present, no real decomposition \
                 vocabulary): {file}"
            ));
        } else {
            routine_hits += 1;
            evidence.push(format!(
                "UNCLASSIFIED planner file (siblings={has_siblings}, decompose={has_decompose}): {file}"
            ));
        }
    }
    if routine_hits > 0 {
        return Ok(CaseReport::fail(
            CASE,
            "a planner hit outside the role loop — probe outdated".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "every 'planner' hit is the planner ROLE (LLM role in the planner->coder->reviewer \
         loop: role prompts, model config, pipeline prose, and the autoresearch \
         proposer's binding of the planner specialist) — verified per file by the \
         coder/reviewer sibling check or the Proposer-binding check; no routine \
         splits goals into subgoals"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"planner_files": files.len(), "routine_hits": 0}),
        evidence,
    ))
}

/// A1: the adversarial self-similar goals ("refine this plan", "break
/// this down further") have no decomposer to loop in. The case asserts
/// against the LIVE tree: (a) the decomposition vocabulary scan is
/// zero, and (b) `phlow-agent`'s public module list — read from the
/// real `lib.rs` at probe time — contains no planner/decomposer
/// module. The closest goal-accepting entry point,
/// `Orchestrator::run`, is a single pass into `Runtime::run` (one call
/// per user message, no goal-splitting loop): an adversarial goal
/// cannot diverge because there is no recursion over goals to diverge
/// in.
fn case_self_similar_goal_has_no_decomposer() -> Result<CaseReport, DriverError> {
    const CASE: &str = "self_similar_goal_has_no_decomposer";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    for goal in ADVERSARIAL_GOALS {
        evidence.push(format!("adversarial goal: \"{goal}\""));
    }
    let hits = scan_tokens_with_lines(&root, &DECOMPOSE_TOKENS)?;
    let (control, real) = classify_decompose_hits(&hits);
    evidence.push(format!(
        "decomposition vocabulary hits for the adversarial goals to reach: {} ({} control, {} real)",
        hits.len(),
        control.len(),
        real.len()
    ));
    for hit in &control {
        evidence.push(format!(
            "classified control sample (UNRELATED): {hit} — mixed-radix (tensor-index) \
             decomposition, not goal decomposition"
        ));
    }
    let lib_rs = std::fs::read_to_string(
        root.join("crates")
            .join("phlow-agent")
            .join("src")
            .join("lib.rs"),
    )
    .map_err(|e| fixture_error("phlow-agent/src/lib.rs", e))?;
    let mut modules = Vec::new();
    for line in lib_rs.lines() {
        let trimmed = line.trim_start();
        if let Some(name) = trimmed.strip_prefix("pub mod ") {
            let name = name.trim_end_matches(';').trim();
            modules.push(name.to_string());
        }
    }
    evidence.push(format!(
        "phlow-agent public modules (live lib.rs): {}",
        modules.join(", ")
    ));
    let suspicious: Vec<&String> = modules
        .iter()
        .filter(|m| {
            let low = m.to_lowercase();
            low.contains("plan") || low.contains("decompos") || low.contains("goal")
        })
        .collect();
    if !real.is_empty() || !suspicious.is_empty() {
        for hit in &real {
            evidence.push(format!("hit: {hit}"));
        }
        for module in &suspicious {
            evidence.push(format!("suspicious module: {module}"));
        }
        return Ok(CaseReport::fail(
            CASE,
            "decomposition machinery appeared — probe outdated".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "no real goal-decomposition hits (only the mixed-radix control sample) and no \
         planner/decomposer/goal module in phlow-agent: the adversarial self-similar \
         goals cannot reach a decomposition routine — there is no recursion over goals \
         to diverge in (Orchestrator::run is one pass into Runtime::run per user message)"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"adversarial_goals": ADVERSARIAL_GOALS.len(), "decompose_hits": 0}),
        evidence,
    ))
}

/// A2: fail-closed. The union of the real goal-decomposition
/// vocabulary (control samples excluded) and the planner-module check:
/// any appearance of the machinery flips the task verdict to
/// `where = "recon"` (premise changed) instead of the seam absence. A
/// probe that crashes (`DriverError`) must never masquerade as the
/// finding — this case runs the union scan to completion and reports
/// which branch it took.
fn case_fail_closed_if_decomposer_appears() -> Result<CaseReport, DriverError> {
    const CASE: &str = "fail_closed_if_decomposer_appears";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let hits = scan_tokens_with_lines(&root, &DECOMPOSE_TOKENS)?;
    let (control, real) = classify_decompose_hits(&hits);
    let total = real.len();
    evidence.push(format!(
        "union scan ({} decompose tokens): {} real hits ({} control samples excluded)",
        DECOMPOSE_TOKENS.len(),
        total,
        control.len()
    ));
    if total > 0 {
        for hit in &real {
            evidence.push(format!("hit: {hit}"));
        }
        return Ok(CaseReport::fail(
            CASE,
            "decomposition machinery appeared — premise changed".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "union scan clean (mixed-radix control sample excluded): the fail-closed branch \
         is armed — any future decomposer flips the verdict to where=\"recon\""
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"union_hits": 0}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "decompose_vocabulary_absent" => case_decompose_vocabulary_absent(),
        "planner_is_role_not_routine" => case_planner_is_role_not_routine(),
        "self_similar_goal_has_no_decomposer" => case_self_similar_goal_has_no_decomposer(),
        "fail_closed_if_decomposer_appears" => case_fail_closed_if_decomposer_appears(),
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
        "recon: the decomposition vocabulary scan (decompose, decomposition, decomposer, subgoal, subgoals, sub_goal) returns zero across every phlow crate — no goal-decomposition routine exists".to_string(),
        "recon: 'planner' in phlow is the planner ROLE (LLM role name in phlow-config, prose in phlow-agent/src/orchestrator.rs, and the autoresearch proposer's binding of the planner specialist), never a routine that splits goals into subgoals".to_string(),
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
        "finding: phlow has no goal-decomposition routine, so the design's depth-bound question is moot — Orchestrator::run is a single pass into Runtime::run per user message; the adversarial self-similar goals (\"refine this plan\", \"break this down further\") have no decomposer to loop in".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no goal-decomposition routine exists in any phlow crate — the decomposition vocabulary scan (decompose, decomposition, decomposer, subgoal, subgoals, sub_goal) returns zero workspace-wide, and phlow-agent's public module list (read from the live lib.rs) contains no planner/decomposer/goal module. The 'planner' in phlow is an LLM role name (phlow-config ModelsConfig.planner / ModelRole::Planner), prose ('planner -> coder -> verification -> reviewer loop' in orchestrator.rs), and one role consumer (the autoresearch proposer binding of the planner specialist in the experimental crate); Orchestrator::run is a single pass into Runtime::run per user message with no goal-splitting loop. The design's pass criteria (always terminates, proven by the cap; the cap is a named constant; partial results at the cap are usable) need a decomposition routine with a depth bound, and there is none. Whether phlow wants goal decomposition at all is banked for Matt — a product decision, not a bug.".to_string(),
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
