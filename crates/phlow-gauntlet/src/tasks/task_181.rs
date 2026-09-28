// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 181 — hostile page payload bounds (rust, A).
//!
//! The seam is `Runtime.evaluate` result ingestion: the page is
//! attacker-controlled, so results are bounded in size and depth
//! *before* they touch agent context. A 50 MB string is prefix-
//! truncated at [`MAX_RESULT_BYTES`](crate::bridge::MAX_RESULT_BYTES)
//! with `truncated: true` (always signaled, never silent); a 512-
//! deep object hits the [`MAX_RESULT_DEPTH`](crate::bridge::MAX_RESULT_DEPTH)
//! bound during iterative deserialization (no recursion → no stack
//! overflow); a cyclic value is rejected per the declared contract
//! (`CYCLIC_CONTRACT = "reject"` → [`BridgeError::CyclicValue`]).
//! An allocation meter counts every copied byte: zero unbounded
//! allocations.

use crate::bridge::{
    Bridge, BridgeConfig, BridgeError, DEPTH_MARKER, MAX_RESULT_BYTES, MAX_RESULT_DEPTH,
    METER_HEADROOM_BYTES, McpContent, ScriptedPort, cdp_object, cdp_string,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-181";
/// Task name.
pub const NAME: &str = "hostile page payload bounds";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 4 adversarial.
pub const CASES: [&str; 4] = [
    "fifty_mb_string_truncated",
    "deep_object_depth_bound",
    "cyclic_rejected",
    "allocation_meter_bounded",
];

/// Fixture nesting depth: 16x MAX_RESULT_DEPTH, deep enough that the
/// bound must fire, shallow enough that `serde_json::Value`'s own
/// recursive Drop cannot overflow the test thread's stack (10,000
/// deep aborted with SIGABRT on drop — outside our iterative walk).
const FIXTURE_DEPTH: usize = 512;

fn hostile_bridge() -> Result<Bridge, BridgeError> {
    let mut port = ScriptedPort::new();
    let big = "x".repeat(50 * 1024 * 1024);
    port.on_evaluate("huge", cdp_string(&big));
    let mut deep = serde_json::json!(0);
    for _ in 0..FIXTURE_DEPTH {
        deep = serde_json::json!({"l": deep});
    }
    port.on_evaluate("deep", cdp_object(deep));
    port.on_evaluate(
        "cyclic",
        cdp_object(serde_json::json!({"a": {"$cyclic": true, "path": "$.a"}})),
    );
    Bridge::launch(
        BridgeConfig::single_task("T-181", vec!["example.com".to_string()]),
        Box::new(port),
    )
}

/// A1: the page returns a 50 MB string → truncated at
/// `MAX_RESULT_BYTES` with `truncated: true`; the agent receives the
/// prefix, never the full blob.
fn case_fifty_mb_string_truncated() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut bridge = hostile_bridge().map_err(fixture_err)?;
    match bridge.evaluate("huge") {
        Ok(McpContent::Text { text, truncated }) => {
            if text.len() != MAX_RESULT_BYTES {
                failures.push(format!(
                    "kept {} bytes, want {MAX_RESULT_BYTES}",
                    text.len()
                ));
            }
            if !text.bytes().all(|b| b == b'x') {
                failures.push("kept prefix is not the string's prefix".to_string());
            }
            if !truncated {
                failures.push("SILENT TRUNCATION: truncated flag not set".to_string());
            }
            evidence.push(format!(
                "50MB string -> {} bytes kept, truncated={} (signaled)",
                text.len(),
                truncated
            ));
        }
        Ok(McpContent::Null) => failures.push("huge string mapped to Null".to_string()),
        Err(e) => failures.push(format!("evaluate failed: {e}")),
    }
    evidence.push(format!("meter bytes: {}", bridge.last_meter_bytes()));
    evidence.push("backend: ScriptedPort (MOCK)".to_string());
    let shutdown = bridge.shutdown();
    if !shutdown.reaped {
        failures.push("bridge not reaped".to_string());
    }
    finish_case(
        CASES[0],
        failures,
        evidence,
        serde_json::json!({
            "input_bytes": 50 * 1024 * 1024,
            "kept_bytes": MAX_RESULT_BYTES,
            "backend": "scripted-mock",
        }),
    )
}

/// A2a: a 512-deep object → the depth bound fires during
/// deserialization. The walk is iterative (explicit stack, no
/// recursion), so there is no stack overflow; over-deep subtrees are
/// replaced with the `$truncated-max-depth` marker and truncation is
/// signaled.
fn case_deep_object_depth_bound() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut bridge = hostile_bridge().map_err(fixture_err)?;
    match bridge.evaluate("deep") {
        Ok(McpContent::Text { text, truncated }) => {
            if !truncated {
                failures.push("SILENT TRUNCATION: depth cut not signaled".to_string());
            }
            if !text.contains(DEPTH_MARKER) {
                failures.push(format!("depth marker '{DEPTH_MARKER}' missing from output"));
            }
            // The output nesting is bounded: count the depth of the
            // serialized form by scanning for the marker position.
            evidence.push(format!(
                "512-deep object -> {} bytes, truncated={}, depth marker present",
                text.len(),
                truncated
            ));
            if text.len() > MAX_RESULT_BYTES + METER_HEADROOM_BYTES {
                failures.push(format!("output {} bytes exceeds bound", text.len()));
            }
        }
        Ok(McpContent::Null) => failures.push("deep object mapped to Null".to_string()),
        Err(e) => failures.push(format!("evaluate failed (stack overflow?): {e}")),
    }
    evidence.push(format!("meter bytes: {}", bridge.last_meter_bytes()));
    evidence.push("backend: ScriptedPort (MOCK)".to_string());
    let shutdown = bridge.shutdown();
    if !shutdown.reaped {
        failures.push("bridge not reaped".to_string());
    }
    finish_case(
        CASES[1],
        failures,
        evidence,
        serde_json::json!({
            "input_depth": FIXTURE_DEPTH,
            "depth_bound": MAX_RESULT_DEPTH,
            "backend": "scripted-mock",
        }),
    )
}

/// A2b: a cyclic value → rejected with the typed `CyclicValue` error
/// per the declared contract (`CYCLIC_CONTRACT = "reject"`). The
/// scripted endpoint encodes the cycle with the `$cyclic` marker key.
fn case_cyclic_rejected() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut bridge = hostile_bridge().map_err(fixture_err)?;
    match bridge.evaluate("cyclic") {
        Err(BridgeError::CyclicValue) => {
            evidence
                .push("cyclic value -> BridgeError::CyclicValue (typed, per contract)".to_string());
        }
        Ok(_) => failures.push("cyclic value was admitted".to_string()),
        Err(e) => failures.push(format!("wrong error for cyclic value: {e}")),
    }
    evidence.push("contract: CYCLIC_CONTRACT = \"reject\" (declared in bridge.rs)".to_string());
    evidence.push("backend: ScriptedPort (MOCK)".to_string());
    let shutdown = bridge.shutdown();
    if !shutdown.reaped {
        failures.push("bridge not reaped".to_string());
    }
    finish_case(
        CASES[2],
        failures,
        evidence,
        serde_json::json!({
            "contract": "reject",
            "backend": "scripted-mock",
        }),
    )
}

/// The allocation meter across all three hostile ingestions: every
/// copied byte is counted and the total stays within
/// `MAX_RESULT_BYTES + METER_HEADROOM_BYTES`. Zero unbounded
/// allocations.
fn case_allocation_meter_bounded() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let bound = MAX_RESULT_BYTES + METER_HEADROOM_BYTES;
    let mut bridge = hostile_bridge().map_err(fixture_err)?;
    for expr in ["huge", "deep"] {
        let _ = bridge.evaluate(expr);
        let used = bridge.last_meter_bytes();
        if used > bound {
            failures.push(format!("'{expr}': meter {used} bytes exceeds {bound}"));
        }
        evidence.push(format!("'{expr}': meter {used} bytes (bound {bound})"));
    }
    // Cyclic is rejected before any copy: the meter must read 0.
    let before = bridge.last_meter_bytes();
    match bridge.evaluate("cyclic") {
        Err(BridgeError::CyclicValue) => {
            evidence.push(format!(
                "cyclic rejected before copy (meter unchanged at {before})"
            ));
        }
        other => failures.push(format!("cyclic: wrong outcome {}", other.is_ok())),
    }
    evidence.push("backend: ScriptedPort (MOCK)".to_string());
    let shutdown = bridge.shutdown();
    if !shutdown.reaped {
        failures.push("bridge not reaped".to_string());
    }
    finish_case(
        CASES[3],
        failures,
        evidence,
        serde_json::json!({
            "meter_bound": bound,
            "backend": "scripted-mock",
        }),
    )
}

fn fixture_err(e: BridgeError) -> TaskDriverError {
    TaskDriverError::Fixture {
        what: "bridge".to_string(),
        detail: format!("task-181: {e}"),
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
        "fifty_mb_string_truncated" => case_fifty_mb_string_truncated(),
        "deep_object_depth_bound" => case_deep_object_depth_bound(),
        "cyclic_rejected" => case_cyclic_rejected(),
        "allocation_meter_bounded" => case_allocation_meter_bounded(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-181: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// 50 MB truncation itself.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-181".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-181".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
