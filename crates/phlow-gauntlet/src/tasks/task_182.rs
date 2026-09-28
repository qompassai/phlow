// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 182 — bridge navigation allowlist (rust, A).
//!
//! The seam is `Page.navigate` / target creation through the bridge.
//! The bridge is scoped to its task: navigation is allowlisted
//! (`http`/`https` to declared hosts, exact or dot-subdomain —
//! `example.com.evil.com` does not match `example.com`), and escapes
//! are refused with typed [`BridgeError::NavigationDenied`] *before*
//! the port is ever touched. A page-side `window.open` (an external
//! `Target.created`) is refused and closed unless the task declared
//! multi-target, and the live target count never exceeds the declared
//! bound.

use crate::bridge::{
    Bridge, BridgeConfig, BridgeError, DevToolsPort, ScriptedPort, SharedScriptedPort, StubKind,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use std::sync::{Arc, Mutex};

/// Task id.
pub const ID: &str = "task-182";
/// Task name.
pub const NAME: &str = "bridge navigation allowlist";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 4 adversarial.
pub const CASES: [&str; 4] = [
    "file_url_denied",
    "off_allowlist_host_denied",
    "window_open_refused_single_target",
    "multi_target_declared_bound",
];

fn launch_shared(task: &str, multi: bool, max: usize) -> (Bridge, Arc<Mutex<ScriptedPort>>) {
    let shared = SharedScriptedPort::default();
    let port = shared.handle();
    let config = BridgeConfig {
        task_id: task.to_string(),
        allowed_hosts: vec!["example.com".to_string()],
        allow_multi_target: multi,
        max_targets: max,
        stub: StubKind::Sleep,
    };
    let bridge = Bridge::launch(config, Box::new(shared)).expect("task-182: bridge launch failed");
    (bridge, port)
}

/// A1a: navigation to `file:///etc/passwd` → `NavigationDenied`, and
/// the port never sees the request.
fn case_file_url_denied() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let (mut bridge, port) = launch_shared("T-182a", false, 1);
    match bridge.navigate("file:///etc/passwd") {
        Err(BridgeError::NavigationDenied { url, reason }) => {
            evidence.push(format!(
                "file:///etc/passwd -> NavigationDenied: {reason} ({url})"
            ));
        }
        other => failures.push(format!("file URL: wrong outcome {other:?}")),
    }
    // Also a data: URL and a bare javascript: attempt.
    for url in ["data:text/html,<h1>x</h1>", "javascript:alert(1)"] {
        match bridge.navigate(url) {
            Err(BridgeError::NavigationDenied { .. }) => {}
            other => failures.push(format!("'{url}': wrong outcome {other:?}")),
        }
    }
    if !port.lock().unwrap().navigations_observed().is_empty() {
        failures.push("the port was touched for a denied navigation".to_string());
    }
    evidence.push("port observed zero navigations: allowlist fires before the port".to_string());
    let shutdown = bridge.shutdown();
    if !shutdown.reaped {
        failures.push("bridge not reaped".to_string());
    }
    finish_case(
        CASES[0],
        failures,
        evidence,
        serde_json::json!({
            "denied": 3,
            "port_navigations": 0,
        }),
    )
}

/// A1b: `http://` outside the declared hosts → denied; the
/// dot-boundary holds (`example.com.evil.com` is not `example.com`);
/// a legitimate subdomain sails through.
fn case_off_allowlist_host_denied() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let (mut bridge, port) = launch_shared("T-182b", false, 1);
    for url in [
        "http://evil.com/x",
        "https://example.com.evil.com/",
        "http://notexample.com/",
    ] {
        match bridge.navigate(url) {
            Err(BridgeError::NavigationDenied { reason, .. }) => {
                evidence.push(format!("'{url}' denied: {reason}"));
            }
            other => failures.push(format!("'{url}': wrong outcome {other:?}")),
        }
    }
    match bridge.navigate("https://sub.example.com/ok") {
        Ok(()) => evidence.push("https://sub.example.com/ok allowed (dot-subdomain)".to_string()),
        Err(e) => failures.push(format!("legitimate subdomain denied: {e}")),
    }
    let observed = port.lock().unwrap().navigations_observed();
    if observed != vec!["https://sub.example.com/ok".to_string()] {
        failures.push(format!("port saw wrong navigations: {observed:?}"));
    }
    let shutdown = bridge.shutdown();
    if !shutdown.reaped {
        failures.push("bridge not reaped".to_string());
    }
    finish_case(
        CASES[1],
        failures,
        evidence,
        serde_json::json!({
            "denied": 3,
            "allowed": 1,
        }),
    )
}

/// A2a: single-target task; the page calls `window.open` → the
/// external `Target.created` is refused and closed. No silent second
/// target: the live count stays 1.
fn case_window_open_refused_single_target() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let (mut bridge, port) = launch_shared("T-182c", false, 1);
    port.lock()
        .unwrap()
        .simulate_external_target("https://example.com/popup");
    let refusals = bridge.drain_external_events();
    if refusals.len() != 1 {
        failures.push(format!("{} refusals, want 1", refusals.len()));
    }
    match refusals.first() {
        Some(BridgeError::TargetRefused { raw_id }) => {
            evidence.push(format!(
                "external target '{raw_id}' -> TargetRefused (typed)"
            ));
        }
        other => failures.push(format!("wrong refusal: {other:?}")),
    }
    if bridge.target_count() != 1 {
        failures.push(format!("target count {}, want 1", bridge.target_count()));
    }
    if bridge.external_refusals() != 1 {
        failures.push("refusal not counted".to_string());
    }
    if port.lock().unwrap().targets().len() != 1 {
        failures.push("external target not closed on the port".to_string());
    }
    evidence.push("live targets: 1 (no silent second target)".to_string());
    let shutdown = bridge.shutdown();
    if !shutdown.reaped || shutdown.targets_closed != 1 {
        failures.push("teardown wrong".to_string());
    }
    finish_case(
        CASES[2],
        failures,
        evidence,
        serde_json::json!({
            "refusals": refusals.len(),
            "live_targets": 1,
        }),
    )
}

/// A2b: the task declared multi-target with bound 2. The first
/// `window.open` is adopted (namespaced); the second exceeds the
/// bound → `TargetLimitExceeded`; the count never exceeds 2.
fn case_multi_target_declared_bound() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let (mut bridge, port) = launch_shared("T-182d", true, 2);
    port.lock()
        .unwrap()
        .simulate_external_target("https://example.com/a");
    let first = bridge.drain_external_events();
    if !first.is_empty() {
        failures.push(format!("declared second target refused: {first:?}"));
    }
    if bridge.target_count() != 2 {
        failures.push(format!("target count {}, want 2", bridge.target_count()));
    }
    let ids = bridge.known_target_ids();
    if ids.len() != 2 || !ids.iter().all(|id| id.starts_with("T-182d:")) {
        failures.push(format!("target ids not namespaced: {ids:?}"));
    }
    evidence.push(format!("adopted second target; ids: {ids:?}"));
    port.lock()
        .unwrap()
        .simulate_external_target("https://example.com/b");
    let second = bridge.drain_external_events();
    match second.as_slice() {
        [BridgeError::TargetLimitExceeded { limit: 2 }] => {
            evidence.push("third target -> TargetLimitExceeded{limit: 2} (typed)".to_string());
        }
        other => failures.push(format!("wrong bound refusal: {other:?}")),
    }
    if bridge.target_count() != 2 {
        failures.push(format!(
            "target count {}, bound is 2",
            bridge.target_count()
        ));
    }
    evidence.push("live targets never exceeded the declared bound 2".to_string());
    let shutdown = bridge.shutdown();
    if !shutdown.reaped {
        failures.push("bridge not reaped".to_string());
    }
    if shutdown.targets_closed != 2 {
        failures.push(format!(
            "closed {} targets, want 2",
            shutdown.targets_closed
        ));
    }
    finish_case(
        CASES[3],
        failures,
        evidence,
        serde_json::json!({
            "bound": 2,
            "live_targets": 2,
            "targets_closed": shutdown.targets_closed,
        }),
    )
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
        "file_url_denied" => case_file_url_denied(),
        "off_allowlist_host_denied" => case_off_allowlist_host_denied(),
        "window_open_refused_single_target" => case_window_open_refused_single_target(),
        "multi_target_declared_bound" => case_multi_target_declared_bound(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-182: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// `file://` escape refused.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-182".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-182".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
