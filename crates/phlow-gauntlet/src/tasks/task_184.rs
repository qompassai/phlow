// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 184 — bridge cross-talk isolation (rust, A).
//!
//! The seam is two concurrent bridges serving two tasks. Tasks are
//! isolated: no message, target, or stdio byte crosses from one bridge
//! to another. A1: T1's page exfiltrates a marker string via an
//! evaluate result → the marker never appears in T2's result stream
//! (asserted by scanning). A2: T1's bridge is fed a hostile payload
//! addressing T2's target id → refused with
//! [`BridgeError::UnknownTarget`] in T1's namespace. Target ids are
//! namespaced per bridge as `<task-id>:<raw-id>` — disjoint by
//! construction, asserted.
//!
//! Note the fixture sharpness: both scripted ports mint the *same*
//! raw id (`scripted-0000`). Namespacing is what keeps them apart —
//! the raw id alone must never resolve.

use crate::bridge::{Bridge, BridgeConfig, BridgeError, McpContent, ScriptedPort, cdp_string};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-184";
/// Task name.
pub const NAME: &str = "bridge cross-talk isolation";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 3 adversarial.
pub const CASES: [&str; 3] = [
    "no_result_crosstalk",
    "cross_target_id_unknown",
    "namespaces_disjoint_by_construction",
];

const MARKER: &str = "EXFIL-MARKER-T1-9f2c";

fn launch_pair() -> Result<(Bridge, Bridge, String, String), BridgeError> {
    let mut port1 = ScriptedPort::new();
    port1.on_evaluate("exfil", cdp_string(MARKER));
    let mut port2 = ScriptedPort::new();
    port2.on_evaluate("benign", cdp_string("T2 benign result"));
    let t1 = Bridge::launch(
        BridgeConfig::single_task("T-184a", vec!["example.com".to_string()]),
        Box::new(port1),
    )?;
    let t2 = Bridge::launch(
        BridgeConfig::single_task("T-184b", vec!["example.com".to_string()]),
        Box::new(port2),
    )?;
    let t1_id = t1.target_id().to_string();
    let t2_id = t2.target_id().to_string();
    Ok((t1, t2, t1_id, t2_id))
}

fn content_text(content: McpContent) -> String {
    match content {
        McpContent::Text { text, .. } => text,
        McpContent::Null => "<null>".to_string(),
    }
}

/// A1: T1's page exfiltrates a marker via evaluate; T2 evaluates
/// benignly. The marker must never appear anywhere in T2's result
/// stream.
fn case_no_result_crosstalk() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let (mut t1, mut t2, t1_id, t2_id) = launch_pair().map_err(fixture_err)?;
    let t1_text = content_text(t1.evaluate("exfil").map_err(fixture_err)?);
    if !t1_text.contains(MARKER) {
        failures.push("T1's own result lacks the marker (fixture broken)".to_string());
    }
    evidence.push(format!("T1 result carries the marker: '{t1_text}'"));
    // T2's full result stream, several evaluates deep.
    let mut t2_stream = String::new();
    for _ in 0..3 {
        let text = content_text(t2.evaluate("benign").map_err(fixture_err)?);
        t2_stream.push_str(&text);
        t2_stream.push('\n');
    }
    if t2_stream.contains(MARKER) {
        failures.push("CROSSTALK: T1's marker appeared in T2's result stream".to_string());
    } else {
        evidence.push("T2 result stream scanned: marker absent (3 evaluates)".to_string());
    }
    evidence.push(format!("T1 id: {t1_id}"));
    evidence.push(format!("T2 id: {t2_id}"));
    let s1 = t1.shutdown();
    let s2 = t2.shutdown();
    if !s1.reaped || !s2.reaped {
        failures.push("a bridge was not reaped".to_string());
    }
    finish_case(
        CASES[0],
        failures,
        evidence,
        serde_json::json!({
            "marker": MARKER,
            "t2_stream_bytes": t2_stream.len(),
            "crosstalk": false,
        }),
    )
}

/// A2: T1 is fed a hostile payload addressing T2's target — both the
/// raw port id and T2's namespaced id → `UnknownTarget` in T1's
/// namespace, every time. The bridge never resolves outside its own
/// namespace.
fn case_cross_target_id_unknown() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let (mut t1, t2, t1_id, t2_id) = launch_pair().map_err(fixture_err)?;
    // T2's raw id: strip T2's namespace prefix. Both scripted ports
    // mint "scripted-0000" — the collision is the point.
    let t2_raw = t2_id
        .strip_prefix("T-184b:")
        .expect("T2 id namespaced")
        .to_string();
    evidence.push(format!(
        "T2 raw id: '{t2_raw}' (collides with T1's raw id by design)"
    ));
    for hostile_id in [t2_raw.as_str(), t2_id.as_str(), "T-184b:scripted-9999"] {
        match t1.evaluate_on(hostile_id, "exfil") {
            Err(BridgeError::UnknownTarget { id }) => {
                evidence.push(format!(
                    "T1 + '{hostile_id}' -> UnknownTarget{{id: '{id}'}}"
                ));
            }
            other => failures.push(format!("T1 resolved foreign id '{hostile_id}': {other:?}")),
        }
    }
    // And T1 still resolves its own id.
    if t1.evaluate("exfil").is_err() {
        failures.push("T1 stopped resolving its own target".to_string());
    }
    evidence.push(format!("T1 id: {t1_id} still resolves"));
    let s1 = t1.shutdown();
    let s2 = t2.shutdown();
    if !s1.reaped || !s2.reaped {
        failures.push("a bridge was not reaped".to_string());
    }
    finish_case(
        CASES[1],
        failures,
        evidence,
        serde_json::json!({
            "foreign_ids_refused": 3,
        }),
    )
}

/// Target-id namespaces are disjoint by construction: every id is
/// `<task-id>:<raw-id>`, the task ids differ, and the two bridges'
/// known-id sets share no element.
fn case_namespaces_disjoint_by_construction() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let (t1, t2, t1_id, t2_id) = launch_pair().map_err(fixture_err)?;
    for id in t1.known_target_ids() {
        if !id.starts_with("T-184a:") {
            failures.push(format!("T1 id '{id}' lacks its namespace prefix"));
        }
    }
    for id in t2.known_target_ids() {
        if !id.starts_with("T-184b:") {
            failures.push(format!("T2 id '{id}' lacks its namespace prefix"));
        }
    }
    let set1: std::collections::BTreeSet<String> = t1.known_target_ids().into_iter().collect();
    let ids2 = t2.known_target_ids();
    let overlap: Vec<&String> = ids2.iter().filter(|id| set1.contains(*id)).collect();
    if !overlap.is_empty() {
        failures.push(format!("namespace overlap: {overlap:?}"));
    }
    evidence.push(format!("T1 namespace: {t1_id}"));
    evidence.push(format!("T2 namespace: {t2_id}"));
    evidence.push(
        "known-id sets disjoint; every id carries its task prefix by construction".to_string(),
    );
    let overlap_count = overlap.len();
    let s1 = t1.shutdown();
    let s2 = t2.shutdown();
    if !s1.reaped || !s2.reaped {
        failures.push("a bridge was not reaped".to_string());
    }
    finish_case(
        CASES[2],
        failures,
        evidence,
        serde_json::json!({
            "overlap": overlap_count,
        }),
    )
}

fn fixture_err(e: BridgeError) -> TaskDriverError {
    TaskDriverError::Fixture {
        what: "bridge".to_string(),
        detail: format!("task-184: {e}"),
    }
}

fn finish_case(
    case: &'static str,
    failures: Vec<String>,
    mut evidence: Vec<String>,
    metrics: serde_json::Value,
) -> Result<CaseReport, TaskDriverError> {
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(case, metrics, evidence);
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "no_result_crosstalk" => case_no_result_crosstalk(),
        "cross_target_id_unknown" => case_cross_target_id_unknown(),
        "namespaces_disjoint_by_construction" => case_namespaces_disjoint_by_construction(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-184: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// exfiltration isolation itself.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-184".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-184".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
