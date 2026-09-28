//! Task 132 — scope diffing: added/removed targets (rust, V).
//!
//! The agent must know what changed between polls to queue and cancel
//! correctly. [`diff_scope`] computes added / removed / changed
//! between two [`ScopeSnapshot`]s, keyed by target id: a stable id
//! with a changed value is one `changed` pair — never an add+remove.
//! The diff is O(n) via id-indexed maps, so 10k-target snapshots
//! diff in well under a second.
//!
//! All fixtures are synthetic; no clock is needed.

use crate::bounty::diff::diff_scope;
use crate::bounty::feed::{snapshot, target};
use crate::bounty::types::{ScopeSnapshot, TargetKind};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-132";
/// Task name.
pub const NAME: &str = "scope diffing: added/removed targets";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "added_removed",
    "changed_is_pair",
    "empty_new_snapshot",
    "ten_k_diff_bounded",
];
/// Target count for the bounded-work adversarial case.
pub const LARGE_SNAPSHOT_TARGETS: usize = 10_000;
/// Wall-clock budget for the 10k diff (seconds).
pub const DIFF_WALL_SECS_MAX: f64 = 1.0;

fn snap_v1() -> ScopeSnapshot {
    snapshot(
        1,
        1_000_000,
        vec![
            target("a", TargetKind::Domain, "a.example.com"),
            target("b", TargetKind::Domain, "b.example.com"),
            target("c", TargetKind::Domain, "c.example.com"),
        ],
    )
}

fn snap_v2() -> ScopeSnapshot {
    snapshot(
        2,
        1_000_060,
        vec![
            target("b", TargetKind::Domain, "b.example.com"),
            // c keeps its id but gets a new value: changed, not
            // add+remove.
            target("c", TargetKind::Domain, "c2.example.com"),
            target("d", TargetKind::Domain, "d.example.com"),
        ],
    )
}

fn id_set(targets: &[crate::bounty::types::Target]) -> Vec<String> {
    let mut ids: Vec<String> = targets.iter().map(|t| t.id.0.clone()).collect();
    ids.sort();
    ids
}

/// V1: added == {d}, removed == {a} — exact set equality.
fn case_added_removed() -> Result<CaseReport, TaskDriverError> {
    let diff = diff_scope(&snap_v1(), &snap_v2());
    let mut failures = Vec::new();
    let added = id_set(&diff.added);
    let removed = id_set(&diff.removed);
    if added != ["d"] {
        failures.push(format!("added = {added:?}, want [\"d\"]"));
    }
    if removed != ["a"] {
        failures.push(format!("removed = {removed:?}, want [\"a\"]"));
    }
    let mut evidence = vec![
        format!("v1 -> v2 diff: added {added:?}, removed {removed:?}"),
        format!(
            "changed pairs: {}",
            diff.changed
                .iter()
                .map(|(o, n)| format!("{}: {} -> {}", o.id.0, o.value, n.value))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "added": added,
            "removed": removed,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: the stable id `c` with a changed value is exactly one
/// `changed` pair — it appears in neither added nor removed.
fn case_changed_is_pair() -> Result<CaseReport, TaskDriverError> {
    let diff = diff_scope(&snap_v1(), &snap_v2());
    let mut failures = Vec::new();
    if diff.changed.len() != 1 {
        failures.push(format!(
            "changed has {} entries, want exactly 1",
            diff.changed.len()
        ));
    }
    if let Some((old, new)) = diff.changed.first() {
        if old.id.0 != "c" || old.value != "c.example.com" {
            failures.push(format!(
                "changed pair old = {}:{}, want c:c.example.com",
                old.id.0, old.value
            ));
        }
        if new.id.0 != "c" || new.value != "c2.example.com" {
            failures.push(format!(
                "changed pair new = {}:{}, want c:c2.example.com",
                new.id.0, new.value
            ));
        }
    }
    for t in diff.added.iter().chain(diff.removed.iter()) {
        if t.id.0 == "c" {
            failures.push("id c appears in added/removed: must be changed-only".to_string());
        }
    }
    let mut evidence = vec![format!(
        "changed pairs: {}",
        diff.changed
            .iter()
            .map(|(o, n)| format!("{}: {} -> {}", o.id.0, o.value, n.value))
            .collect::<Vec<_>>()
            .join(", ")
    )];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "changed_count": diff.changed.len(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1 (adversarial): the new snapshot is empty. Everything is
/// removed, nothing added, no panic.
fn case_empty_new_snapshot() -> Result<CaseReport, TaskDriverError> {
    let empty = snapshot(2, 1_000_060, Vec::new());
    let diff = diff_scope(&snap_v1(), &empty);
    let mut failures = Vec::new();
    let removed = id_set(&diff.removed);
    if removed != ["a", "b", "c"] {
        failures.push(format!("removed = {removed:?}, want [\"a\", \"b\", \"c\"]"));
    }
    if !diff.added.is_empty() {
        failures.push(format!("added = {:?}, want []", id_set(&diff.added)));
    }
    if !diff.changed.is_empty() {
        failures.push(format!("changed = {:?}, want []", diff.changed.len()));
    }
    let mut evidence = vec![format!(
        "v1 -> empty diff: added {}, removed {removed:?}, changed {}",
        diff.added.len(),
        diff.changed.len()
    )];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "added": 0usize,
            "removed": removed,
            "changed": 0usize,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2 (adversarial): 10k-target snapshots must diff in under a
/// second — the id-indexed O(n) contract, not O(n^2).
fn case_ten_k_diff_bounded() -> Result<CaseReport, TaskDriverError> {
    let n = LARGE_SNAPSHOT_TARGETS;
    let old_targets: Vec<_> = (0..n)
        .map(|i| {
            target(
                &format!("t{i:05}"),
                TargetKind::Domain,
                &format!("t{i:05}.example.com"),
            )
        })
        .collect();
    // New: drop the first 50, change the next 100 values, keep the
    // rest, add 50 fresh ids at the end.
    let mut new_targets: Vec<_> = old_targets
        .iter()
        .skip(50)
        .map(|t| {
            let idx: usize = t.id.0[1..].parse().map_err(|e| TaskDriverError::Arm {
                arm: "fixture".to_string(),
                detail: format!("bad fixture id {}: {e}", t.id.0),
            })?;
            if idx < 150 {
                Ok(target(
                    &t.id.0,
                    TargetKind::Domain,
                    &format!("{}.v2.example.com", t.id.0),
                ))
            } else {
                Ok(t.clone())
            }
        })
        .collect::<Result<Vec<_>, TaskDriverError>>()?;
    for i in n..n + 50 {
        new_targets.push(target(
            &format!("t{i:05}"),
            TargetKind::Domain,
            &format!("t{i:05}.example.com"),
        ));
    }
    let old = snapshot(1, 1_000_000, old_targets);
    let new = snapshot(2, 1_000_060, new_targets);
    let start = std::time::Instant::now();
    let diff = diff_scope(&old, &new);
    let elapsed = start.elapsed();
    let mut failures = Vec::new();
    if elapsed.as_secs_f64() >= DIFF_WALL_SECS_MAX {
        failures.push(format!(
            "10k diff took {:.3}s, budget is < {DIFF_WALL_SECS_MAX}s",
            elapsed.as_secs_f64()
        ));
    }
    if diff.added.len() != 50 {
        failures.push(format!("added = {}, want 50", diff.added.len()));
    }
    if diff.removed.len() != 50 {
        failures.push(format!("removed = {}, want 50", diff.removed.len()));
    }
    if diff.changed.len() != 100 {
        failures.push(format!("changed = {}, want 100", diff.changed.len()));
    }
    let mut evidence = vec![format!(
        "10k-target diff: {:.3}ms (budget < {}ms); added {}, removed {}, changed {}",
        elapsed.as_secs_f64() * 1000.0,
        DIFF_WALL_SECS_MAX * 1000.0,
        diff.added.len(),
        diff.removed.len(),
        diff.changed.len()
    )];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "targets": n,
            "elapsed_secs": elapsed.as_secs_f64(),
            "budget_secs": DIFF_WALL_SECS_MAX,
            "added": diff.added.len(),
            "removed": diff.removed.len(),
            "changed": diff.changed.len(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "added_removed" => case_added_removed(),
        "changed_is_pair" => case_changed_is_pair(),
        "empty_new_snapshot" => case_empty_new_snapshot(),
        "ten_k_diff_bounded" => case_ten_k_diff_bounded(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-132: unknown case '{case}'"),
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
            where_: "task-132".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-132".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
