// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 178 — ephemeral bridge launch (rust, V).
//!
//! The seam is `Bridge::launch` → the debugging port → stdio MCP: the
//! bridge is born per task and dies with it. Launch opens exactly one
//! DevTools target (`Target.created` observed once on the port) and the
//! MCP `initialize` handshake succeeds over the newline-delimited stdio
//! framing. Task end tears everything down: the bridge process exits,
//! the target closes, and the port shows zero lingering targets
//! (asserted via port query + process census).

use crate::bridge::{
    Bridge, BridgeConfig, ChromiumPort, DevToolsPort, ScriptedPort, chromium_available, free_port,
    process_census,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-178";
/// Task name.
pub const NAME: &str = "ephemeral bridge launch";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation (scripted) + 1 real-chromium integration.
pub const CASES: [&str; 3] = [
    "launch_one_target_initialize",
    "task_end_zero_lingering",
    "real_chromium_integration",
];

fn config(task: &str) -> BridgeConfig {
    BridgeConfig::single_task(task, vec!["example.com".to_string()])
}

/// V1: launch for task T → exactly one `Target.created` on the
/// debugging port; MCP initialize over stdio succeeds.
fn case_launch_one_target_initialize() -> Result<CaseReport, TaskDriverError> {
    let port = ScriptedPort::new();
    let mut bridge = Bridge::launch(config("T-178"), Box::new(port)).map_err(fixture_err)?;
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    // Exactly one Target.created: the port counted the creation event
    // and holds exactly one target.
    let created = bridge.port_created_events();
    let live = bridge.port_target_count();
    if created != 1 {
        failures.push(format!(
            "Target.created fired {created} times, want exactly 1"
        ));
    }
    if live != 1 {
        failures.push(format!("port holds {live} targets, want exactly 1"));
    }
    // MCP initialize over stdio.
    match bridge.mcp_initialize() {
        Ok(response) => {
            let version = response
                .get("result")
                .and_then(|r| r.get("protocolVersion"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if version != "2024-11-05" {
                failures.push(format!("initialize protocolVersion is '{version}'"));
            }
            evidence.push(format!("initialize ok, protocolVersion {version}"));
        }
        Err(e) => failures.push(format!("mcp_initialize failed: {e}")),
    }
    evidence.push(format!("target id: {}", bridge.target_id()));
    evidence.push("backend: ScriptedPort (MOCK)".to_string());
    let pid = bridge.pid();
    let report_shutdown = bridge.shutdown();
    if !report_shutdown.reaped {
        failures.push("bridge child not reaped on shutdown".to_string());
    }
    evidence.push(format!(
        "bridge pid {pid} reaped: {}",
        report_shutdown.reaped
    ));
    finish_case(
        CASES[0],
        failures,
        evidence,
        serde_json::json!({
            "target_created_events": created,
            "targets_closed": report_shutdown.targets_closed,
            "backend": "scripted-mock",
        }),
    )
}

/// V2: task ends → bridge process exits, target closed, zero lingering
/// targets on the port and zero trace of the PID in the census.
fn case_task_end_zero_lingering() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    // Probe the port lifecycle directly for exact counts.
    let mut probe = ScriptedPort::new();
    let before = probe.targets().len();
    let info = probe.create_target("about:blank").map_err(fixture_err)?;
    let created_events = probe.created_events();
    probe.close_target(&info.raw_id).map_err(fixture_err)?;
    let after = probe.targets().len();
    if before != 0 || created_events != 1 || after != 0 {
        failures.push(format!(
            "port lifecycle wrong: before={before} created_events={created_events} after={after}"
        ));
    }
    evidence.push(format!(
        "scripted port: {before} -> 1 (created_events={created_events}) -> {after} targets"
    ));
    // Full bridge: launch, then shut down; the PID must vanish from
    // the process census and no zombie may remain.
    let mut bridge =
        Bridge::launch(config("T-178-end"), Box::new(ScriptedPort::new())).map_err(fixture_err)?;
    let pid = bridge.pid();
    if !bridge.is_alive() {
        failures.push("bridge child not alive right after launch".to_string());
    }
    let shutdown = bridge.shutdown();
    if !shutdown.reaped {
        failures.push(format!("bridge pid {pid} not reaped"));
    }
    if shutdown.targets_closed != 1 {
        failures.push(format!(
            "targets_closed={}, want 1",
            shutdown.targets_closed
        ));
    }
    let lingering: Vec<u32> = process_census()
        .into_iter()
        .filter(|e| e.pid == pid)
        .map(|e| e.pid)
        .collect();
    if !lingering.is_empty() {
        failures.push(format!("pid {pid} still in process census after reap"));
    }
    evidence.push(format!(
        "pid {pid}: reaped={}, targets_closed={}, census_clean={}",
        shutdown.reaped,
        shutdown.targets_closed,
        lingering.is_empty()
    ));
    evidence.push("backend: ScriptedPort (MOCK) + real /proc census".to_string());
    finish_case(
        CASES[1],
        failures,
        evidence,
        serde_json::json!({
            "reaped": shutdown.reaped,
            "targets_closed": shutdown.targets_closed,
            "census_clean": lingering.is_empty(),
            "backend": "scripted-mock",
        }),
    )
}

/// Integration half: real chromium on primo, owned by the bridge.
/// Target census goes 0 → 1 → 0: no chromium processes before
/// launch, exactly one target (the bridge's) on the debugging port,
/// then shutdown closes it and reaps the browser.
fn case_real_chromium_integration() -> Result<CaseReport, TaskDriverError> {
    if !chromium_available() {
        let evidence = vec!["SKIP: no chromium binary on PATH".to_string()];
        return finish_case(
            CASES[2],
            Vec::new(),
            evidence,
            serde_json::json!({"backend": "skipped-no-chromium"}),
        );
    }
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    // 0: no chromium debugging processes before launch.
    let pre = chromium_pids();
    if !pre.is_empty() {
        failures.push(format!("chromium already running before launch: {pre:?}"));
    }
    let port_num = free_port();
    let profile = crate::bridge::next_temp_dir("w29-chromium-178");
    let config = BridgeConfig::chromium_task("T-178-real", vec![], port_num, profile.clone());
    let mut bridge = Bridge::launch(
        config,
        Box::new(ChromiumPort::new(port_num).map_err(fixture_err)?),
    )
    .map_err(fixture_err)?;
    let pid = bridge.pid();
    let bridge_raw = bridge
        .target_id()
        .strip_prefix("T-178-real:")
        .unwrap_or("?")
        .to_string();
    // 1: the bridge's target is live on the debugging port.
    // (primo's chromium spawns extension/service-worker targets of
    // its own; the census is scoped to the bridge-owned target.)
    let probe = ChromiumPort::new(port_num).map_err(fixture_err)?;
    verify_target_live(
        &mut bridge,
        &probe,
        &bridge_raw,
        pid,
        &mut failures,
        &mut evidence,
    );
    // 0: shutdown closes the target and reaps the browser.
    let shutdown = bridge.shutdown();
    if shutdown.targets_closed != 1 {
        failures.push(format!(
            "targets_closed={}, want 1",
            shutdown.targets_closed
        ));
    }
    if !shutdown.reaped {
        failures.push(format!("chromium pid {pid} not reaped"));
    }
    let census_clean = teardown_and_verify(
        &probe,
        &shutdown,
        pid,
        &profile,
        &mut failures,
        &mut evidence,
    );
    finish_case(
        CASES[2],
        failures,
        evidence,
        serde_json::json!({
            "targets_opened": 1,
            "targets_after_close": 0,
            "targets_closed": shutdown.targets_closed,
            "reaped": shutdown.reaped,
            "census_clean": census_clean,
            "backend": "real-chromium",
        }),
    )
}

/// Verify post-shutdown: the debugging port is dead (connection
/// refused proves the browser is gone, not hiding targets), no
/// chromium PIDs linger in the census, and the profile dir is
/// removed. Returns true when the census is clean.
fn teardown_and_verify(
    probe: &ChromiumPort,
    shutdown: &crate::bridge::ShutdownReport,
    pid: u32,
    profile: &std::path::Path,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> bool {
    match probe.probe_targets() {
        Err(e) => evidence.push(format!("debugging port down after reap ({e})")),
        Ok(ts) => failures.push(format!(
            "debugging port still answers: {} targets",
            ts.len()
        )),
    }
    let lingering: Vec<u32> = process_census()
        .into_iter()
        .filter(|e| e.pid == pid || e.cmdline.contains("remote-debugging-port"))
        .map(|e| e.pid)
        .collect();
    if !lingering.is_empty() {
        failures.push(format!("chromium pids survived teardown: {lingering:?}"));
    }
    evidence.push(format!(
        "targets_closed={}, reaped={}, census_clean={}",
        shutdown.targets_closed,
        shutdown.reaped,
        lingering.is_empty()
    ));
    let _ = std::fs::remove_dir_all(profile);
    lingering.is_empty()
}

/// The "1" half of the 0 → 1 → 0 census: the bridge's target is
/// live on the debugging port and MCP initialize works.
fn verify_target_live(
    bridge: &mut Bridge,
    probe: &ChromiumPort,
    bridge_raw: &str,
    pid: u32,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) {
    match probe.probe_targets() {
        Ok(listed) => {
            if !listed.iter().any(|t| t.raw_id == bridge_raw) {
                failures.push(format!(
                    "bridge target {bridge_raw} not on the debugging port"
                ));
            }
        }
        Err(e) => failures.push(format!("probe_targets failed: {e}")),
    }
    if bridge.target_count() != 1 {
        failures.push(format!("bridge target_count={}", bridge.target_count()));
    }
    if bridge.mcp_initialize().is_err() {
        failures.push("mcp_initialize failed on the real bridge".to_string());
    }
    evidence.push(format!(
        "chromium pid {pid}: bridge target {bridge_raw} live on the debugging port"
    ));
}

/// PIDs whose command line shows a chromium remote-debugging port.
fn chromium_pids() -> Vec<u32> {
    process_census()
        .into_iter()
        .filter(|e| e.cmdline.contains("remote-debugging-port"))
        .map(|e| e.pid)
        .collect()
}

fn fixture_err(e: crate::bridge::BridgeError) -> TaskDriverError {
    TaskDriverError::Fixture {
        what: "bridge".to_string(),
        detail: format!("task-178: {e}"),
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
        "launch_one_target_initialize" => case_launch_one_target_initialize(),
        "task_end_zero_lingering" => case_task_end_zero_lingering(),
        "real_chromium_integration" => case_real_chromium_integration(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-178: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// benign launch itself.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-178".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-178".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
