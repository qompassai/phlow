//! task-94: improvement rollback (rust recon + harness).
//!
//! The design asks for an applied self-improvement to be revertible to
//! the exact pre-apply tree state — byte-identical, verified by hash —
//! where the revert is itself a gated, human-approved action (a revert
//! is a modification too); subsequent conflicting changes make the
//! revert refuse as unsafe (or proceed three-way only with explicit
//! human confirmation), never a silent clobber; and a revert of a
//! revert is rejected as a no-op with a typed result.
//!
//! Seam mapping (verified, not invented): the rollback path is
//! DATA-ONLY. `ImprovementProposal` carries a `rollback_target: String`
//! ("The exact revision to restore on rollback") and
//! [`PromotionRecord`] copies it, and [`Lifecycle::RolledBack`] names
//! the terminal state — but NO code restores a tree from it. The only
//! `revert` code paths in the workspace are explicit denials:
//! `phlow-self-improve`'s `SkillStore::revert_skill` always fails with
//! `GitMutationDisabled` ("Automatic Git mutation is disabled; revert
//! manually"). Bounded exact-token scans over every product crate's
//! `src/**/*.rs` (the gauntlet crate itself excluded, per the
//! harness-probe principle) find zero hits for revert-mechanism
//! vocabulary (git checkout/restore, worktree manipulation,
//! three-way merge).
//!
//! Four cases: two validation, two adversarial — each a bounded source
//! recon that fails closed (premise changed) if revert machinery ever
//! appears. The task-level verdict is `fail` at `"seam"`: the fields
//! name a rollback target, but no mechanism performs the rollback.
//!
//! Product-decision bank: whether phlow-experiment should gain a real
//! gated rollback path (human-approved revert, byte-identical restore
//! verified by hash, unsafe-revert refusal with conflicts named,
//! revert-of-revert typed no-op) is Matt's call — not implemented on
//! gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-94";
/// Human-readable name.
pub const NAME: &str = "improvement rollback";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "no_revert_mechanism",
    "rollback_target_is_data_only",
    "revert_approval_unverifiable",
    "no_unsafe_revert_guard",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-94 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture was unusable.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
    /// A recon probe failed.
    Probe {
        /// Which probe.
        case: String,
        /// The underlying error.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixture { what, detail } => {
                write!(f, "task-94: cannot build fixture {what}: {detail}")
            }
            Self::Probe { case, detail } => {
                write!(f, "task-94: probe {case} failed: {detail}")
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

fn probe_error(case: &str, detail: impl fmt::Display) -> DriverError {
    DriverError::Probe {
        case: case.to_string(),
        detail: detail.to_string(),
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
            failures: vec![],
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

// ---------------------------------------------------------------------------
// Source probe (task_48 scan pattern)
// ---------------------------------------------------------------------------

/// Maximum source files the probe may read.
const SOURCE_FILES_MAX: usize = 4000;
/// Maximum bytes per source file the probe reads.
const SOURCE_BYTES_MAX: usize = 512 * 1024;

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

/// The gauntlet's own crate root, excluded from the product scan: the
/// harness's own probes use the design vocabulary.
fn excluded_crate_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Exact-token (case-insensitive) hits for `token` over every product
/// crate's `src/**/*.rs` — the gauntlet crate itself excluded. Bounded
/// like task_48. Returns the hit paths.
fn scan_workspace(root: &Path, token: &str) -> Result<Vec<String>, DriverError> {
    let excluded = excluded_crate_root();
    let wanted = token.to_lowercase();
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
                if path != excluded {
                    stack.push(path);
                }
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
                    return Err(probe_error(
                        "source scan",
                        format!("{} exceeds {SOURCE_BYTES_MAX} bytes", path.display()),
                    ));
                }
                let text = String::from_utf8_lossy(&bytes).to_lowercase();
                let mut token_start: Option<usize> = None;
                let mut found = false;
                for (idx, ch) in text.char_indices() {
                    let is_token = ch.is_alphanumeric() || ch == '_' || ch == '/' || ch == '-';
                    if is_token {
                        if token_start.is_none() {
                            token_start = Some(idx);
                        }
                    } else if let Some(start) = token_start.take()
                        && text[start..idx] == wanted
                    {
                        found = true;
                        break;
                    }
                }
                if !found
                    && let Some(start) = token_start
                    && text[start..] == wanted
                {
                    found = true;
                }
                if found {
                    hits.push(path.display().to_string());
                }
            }
        }
    }
    Ok(hits)
}

/// Assert an exact-token scan finds zero hits; fail closed (premise
/// changed) when the absence finding is refuted.
fn assert_absent(
    case: &'static str,
    root: &Path,
    tokens: &[&str],
) -> Result<CaseReport, DriverError> {
    let mut evidence = Vec::new();
    let mut total_hits = 0usize;
    for token in tokens {
        let hits = scan_workspace(root, token)?;
        evidence.push(format!(
            "exact-token scan for '{token}' over crates/*/src/**/*.rs: {} hit(s)",
            hits.len()
        ));
        for hit in &hits {
            evidence.push(format!("  unexpected hit: {hit}"));
        }
        total_hits += hits.len();
    }
    if total_hits != 0 {
        return Ok(CaseReport::fail(
            case,
            format!("{total_hits} hit(s) — the absence finding is refuted (premise changed)"),
            evidence,
        ));
    }
    Ok(CaseReport::pass(
        case,
        serde_json::json!({"hits": 0}),
        evidence,
    ))
}

/// Read one product-crate source file, bounded.
fn read_product_source(root: &Path, relative: &str) -> Result<String, DriverError> {
    let path = root.join(relative);
    let bytes = std::fs::read(&path)
        .map_err(|e| fixture_error("source read", format!("{}: {e}", path.display())))?;
    if bytes.len() > SOURCE_BYTES_MAX {
        return Err(probe_error(
            "source read",
            format!("{} exceeds {SOURCE_BYTES_MAX} bytes", path.display()),
        ));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// V1: no revert mechanism exists. The only `revert` code paths in the
/// workspace are explicit denials (`SkillStore::revert_skill` always
/// fails with `GitMutationDisabled`); no git-checkout/restore, no
/// worktree manipulation, no tree-restoring function exists.
fn case_no_revert_mechanism() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_revert_mechanism";
    let root = workspace_root()?;
    let mut report = assert_absent(
        CASE,
        &root,
        &[
            "git_checkout",
            "restore_tree",
            "checkout_tree",
            "three_way",
            "git2",
        ],
    )?;
    // The bare token `revert` DOES hit — but only denial paths. Verify
    // every hit is in phlow-self-improve's disabled surface.
    let revert_hits = scan_workspace(&root, "revert")?;
    report.evidence.push(format!(
        "bare-token scan for 'revert': {} hit(s)",
        revert_hits.len()
    ));
    for hit in &revert_hits {
        report.evidence.push(format!("  revert hit: {hit}"));
        if !hit.contains("phlow-self-improve") {
            return Ok(CaseReport::fail(
                CASE,
                format!(
                    "revert vocabulary outside the disabled self-improve surface: {hit} \
                 — premise changed"
                ),
                report.evidence,
            ));
        }
    }
    let denials = read_product_source(&root, "crates/phlow-self-improve/src/skill_store.rs")?;
    if !denials.contains("GitMutationDisabled") {
        return Ok(CaseReport::fail(
            CASE,
            "the revert denial path changed shape — premise changed".to_string(),
            report.evidence,
        ));
    }
    report.evidence.push(
        "every 'revert' hit is inside phlow-self-improve's disabled surface \
         (revert_skill → GitMutationDisabled, 'revert manually'); no code \
         path restores a tree"
            .to_string(),
    );
    Ok(report)
}

/// V2: `rollback_target` is data only. It is declared on
/// `ProposalParams`/`ImprovementProposal`, exposed via an accessor, and
/// copied into `PromotionRecord` — but nothing reads it to perform a
/// revert: no fs or git operation consumes it.
fn case_rollback_target_is_data_only() -> Result<CaseReport, DriverError> {
    const CASE: &str = "rollback_target_is_data_only";
    let root = workspace_root()?;
    let promotion = read_product_source(&root, "crates/phlow-experiment/src/promotion.rs")?;
    let mut evidence = Vec::new();
    let occurrences = promotion.match_indices("rollback_target").count();
    evidence.push(format!(
        "'rollback_target' occurs {occurrences} time(s) in promotion.rs"
    ));
    if occurrences == 0 {
        return Ok(CaseReport::fail(
            CASE,
            "rollback_target vanished from promotion.rs — premise changed".to_string(),
            evidence,
        ));
    }
    // A real revert would shell out to git, touch the fs via a VCS
    // library, or call a worktree API near the field. None of those
    // appear in the module. (The one `std::fs::rename` in promotion.rs
    // is atomic persistence of the consumed-approvals ledger, not a
    // tree restore — verified by inspection, not by token.)
    for mechanism in ["Command::new", "git2", "worktree", "checkout"] {
        if promotion.contains(mechanism) {
            return Ok(CaseReport::fail(
                CASE,
                format!(
                    "promotion.rs now contains '{mechanism}' — a revert mechanism may \
                 have appeared (premise changed)"
                ),
                evidence,
            ));
        }
    }
    evidence.push(
        "rollback_target is a String field + accessor + record copy; \
         promotion.rs contains no subprocess, git, worktree, or checkout \
         machinery that could act on it"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"revert_mechanisms": 0}),
        evidence,
    ))
}

/// A1: the design demands the revert be a gated, human-approved action.
/// With no revert path at all, that property is unverifiable — it holds
/// vacuously (nothing reverts, approved or not), which is fail-closed
/// but not the design's gated-revert. Confirm no approval-gated revert
/// entry point exists.
fn case_revert_approval_unverifiable() -> Result<CaseReport, DriverError> {
    const CASE: &str = "revert_approval_unverifiable";
    let root = workspace_root()?;
    let mut report = assert_absent(
        CASE,
        &root,
        &[
            "revert_with_approval",
            "approved_revert",
            "rollback_with_approval",
        ],
    )?;
    report.evidence.push(
        "no approval-gated revert entry point exists: the design's \
         'revert requires its own human approval' cannot be verified \
         because there is no revert to gate — fail-closed by absence, \
         not by enforcement"
            .to_string(),
    );
    Ok(report)
}

/// A2: no unsafe-revert guard exists — no conflict detection, no
/// three-way merge, no 'revert refused as unsafe' path. A future revert
/// implementation would need all three; today there is nothing to be
/// unsafe.
fn case_no_unsafe_revert_guard() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_unsafe_revert_guard";
    let root = workspace_root()?;
    let mut report = assert_absent(
        CASE,
        &root,
        &[
            "revert_conflict",
            "unsafe_revert",
            "revert_refused",
            "three_way_merge",
        ],
    )?;
    report.evidence.push(
        "no conflict/three-way/unsafe-revert vocabulary: subsequent \
         changes landing after an improvement have no revert guard to \
         trip — because no revert exists to guard"
            .to_string(),
    );
    Ok(report)
}

/// Run one case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, DriverError> {
    match case {
        "no_revert_mechanism" => case_no_revert_mechanism(),
        "rollback_target_is_data_only" => case_rollback_target_is_data_only(),
        "revert_approval_unverifiable" => case_revert_approval_unverifiable(),
        "no_unsafe_revert_guard" => case_no_unsafe_revert_guard(),
        _ => Err(fixture_error("case", format!("unknown case '{case}'"))),
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

fn run_inner() -> Result<Vec<String>, TaskFailure> {
    let mut evidence = vec![
        "seam: DATA-ONLY — ImprovementProposal.rollback_target names 'the \
         exact revision to restore on rollback' and PromotionRecord copies \
         it, but no code restores a tree; the only revert paths are \
         explicit GitMutationDisabled denials"
            .to_string(),
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
        "finding: the rollback path is data-only — fields name a rollback \
         target, but no mechanism performs the rollback, gates it, or \
         guards it"
            .to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: improvement rollback is data-only, not a mechanism. \
         ImprovementProposal.rollback_target ('The exact revision to restore on rollback') is a \
         String field copied into PromotionRecord; Lifecycle::RolledBack names the terminal \
         state; but promotion.rs contains no subprocess, git, worktree, or checkout machinery \
         that could act on it. The only `revert` code paths in the workspace are explicit \
         denials — phlow-self-improve's SkillStore::revert_skill always fails with \
         GitMutationDisabled ('Automatic Git mutation is disabled; revert manually'). Bounded \
         exact-token scans over every product crate's src/**/*.rs (gauntlet crate excluded per \
         the harness-probe principle) find zero hits for revert-mechanism vocabulary \
         (git_checkout, restore_tree, checkout_tree, three_way, git2), zero hits for \
         approval-gated revert entry points, and zero hits for unsafe-revert guards. The \
         design's properties — byte-identical restore verified by hash, revert as a \
         human-approved action, unsafe-revert refusal with conflicts named, revert-of-revert \
         typed no-op — have no implementation to probe. Product decision banked for Matt: \
         whether phlow-experiment should gain a real gated rollback path is not implemented \
         on gauntlet authority."
            .to_string(),
        evidence,
    })
}

/// Attempt the task.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_inner() {
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
