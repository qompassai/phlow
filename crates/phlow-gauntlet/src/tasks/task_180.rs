// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 180 — DevTools round trip (rust, V).
//!
//! The seam is MCP tool call → `Runtime.evaluate` → typed result: the
//! bridge actually drives the page and returns the value to the agent.
//! A CDP `result` object (`{"type": "string", "value": ...}`,
//! `{"type": "object", "value": ...}`, `{"type": "undefined"}`,
//! `{"type": "bigint", "unserializableValue": ...}`) is mapped to MCP
//! content types: strings round-trip exactly, nested objects map
//! without loss, `undefined` becomes a typed null (not a crash),
//! bigints arrive via `unserializableValue`.
//!
//! Primary source: the Chrome DevTools Protocol `Runtime.evaluate`
//! docs (result object shape, `unserializableValue` for values JSON
//! cannot carry). The scripted half covers evaluate semantics exactly;
//! the integration half drives real chromium to a URL and observes the
//! page title (no websocket transport exists in this harness, so the
//! real-browser `Runtime.evaluate` call itself is out of scope —
//! stated, not hidden).

use crate::bridge::{
    Bridge, BridgeConfig, BridgeError, ChromiumPort, DevToolsPort, McpContent, ScriptedPort,
    cdp_object, cdp_string, chromium_available, free_port, next_temp_dir, process_census,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-180";
/// Task name.
pub const NAME: &str = "DevTools round trip";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 3 validation (scripted) + 1 real-chromium integration.
pub const CASES: [&str; 4] = [
    "evaluate_title_typed",
    "nested_object_mapped",
    "undefined_is_null",
    "real_chromium_title_roundtrip",
];

fn bridge_with_rules() -> Result<Bridge, BridgeError> {
    let mut port = ScriptedPort::new();
    port.on_evaluate("document.title", cdp_string("Wave 29 Test Page"));
    port.on_evaluate(
        "nested",
        cdp_object(serde_json::json!({
            "a": [1, 2, {"b": "x"}],
            "n": null,
            "s": "héllo",
            "deep": {"x": {"y": {"z": 42}}}
        })),
    );
    port.on_evaluate(
        "bigint",
        serde_json::json!({"type": "bigint", "unserializableValue": "9007199254740993n", "description": "9007199254740993n"}),
    );
    Bridge::launch(
        BridgeConfig::single_task("T-180", vec!["example.com".to_string()]),
        Box::new(port),
    )
}

/// V1: evaluate `document.title` → the exact title string, typed.
fn case_evaluate_title_typed() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut bridge = bridge_with_rules().map_err(fixture_err)?;
    match bridge.evaluate("document.title") {
        Ok(McpContent::Text { text, truncated }) => {
            if text != "Wave 29 Test Page" {
                failures.push(format!("title is '{text}', want 'Wave 29 Test Page'"));
            }
            if truncated {
                failures.push("benign title must not be truncated".to_string());
            }
            evidence.push(format!("title round-tripped exactly: '{text}'"));
        }
        Ok(McpContent::Null) => failures.push("title mapped to Null".to_string()),
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
            "title": "Wave 29 Test Page",
            "backend": "scripted-mock",
        }),
    )
}

/// V2: an expression returning a nested object → mapped to MCP content
/// without loss (parse the text back: it must equal the source value).
/// A bigint rides `unserializableValue` per the CDP docs.
fn case_nested_object_mapped() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut bridge = bridge_with_rules().map_err(fixture_err)?;
    let want = serde_json::json!({
        "a": [1, 2, {"b": "x"}],
        "n": null,
        "s": "héllo",
        "deep": {"x": {"y": {"z": 42}}}
    });
    match bridge.evaluate("nested") {
        Ok(McpContent::Text { text, truncated }) => {
            if truncated {
                failures.push("benign nested object must not be truncated".to_string());
            }
            let back: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
                fixture_err(BridgeError::DevTools {
                    detail: format!("mapped text is not JSON: {e}"),
                })
            })?;
            if back != want {
                failures.push(format!("round trip lost data:\n got {back}\nwant {want}"));
            }
            evidence.push("nested object -> text -> parse: byte-identical value".to_string());
        }
        Ok(McpContent::Null) => failures.push("object mapped to Null".to_string()),
        Err(e) => failures.push(format!("evaluate failed: {e}")),
    }
    match bridge.evaluate("bigint") {
        Ok(McpContent::Text { text, truncated }) => {
            if text != "9007199254740993" || truncated {
                failures.push(format!("bigint mapped to '{text}' (truncated={truncated})"));
            }
            evidence.push(format!("bigint via unserializableValue: '{text}'"));
        }
        other => failures.push(format!("bigint wrong outcome: {}", other.is_ok())),
    }
    evidence.push("backend: ScriptedPort (MOCK)".to_string());
    let shutdown = bridge.shutdown();
    if !shutdown.reaped {
        failures.push("bridge not reaped".to_string());
    }
    let lossless = failures.is_empty();
    finish_case(
        CASES[1],
        failures,
        evidence,
        serde_json::json!({
            "round_trip_lossless": lossless,
            "backend": "scripted-mock",
        }),
    )
}

/// `undefined` → typed null, not a crash. An unknown expression (no
/// canned rule) also falls back to `undefined` on the scripted port.
fn case_undefined_is_null() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut bridge = bridge_with_rules().map_err(fixture_err)?;
    for expr in ["undefined", "no.such.rule.here"] {
        match bridge.evaluate(expr) {
            Ok(McpContent::Null) => evidence.push(format!("'{expr}' -> typed Null")),
            Ok(McpContent::Text { text, .. }) => {
                failures.push(format!("'{expr}' mapped to text '{text}', want Null"))
            }
            Err(e) => failures.push(format!("'{expr}' errored: {e}")),
        }
    }
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
            "backend": "scripted-mock",
        }),
    )
}

/// Integration half: real chromium opens a `file://` URL whose title
/// is known; the harness observes the exact title on the debugging
/// port. (`data:` URLs are rejected for top-frame navigation, and
/// full `Runtime.evaluate` needs a websocket; the scripted half above
/// covers its semantics per the CDP docs.)
fn case_real_chromium_title_roundtrip() -> Result<CaseReport, TaskDriverError> {
    if !chromium_available() {
        return finish_case(
            CASES[3],
            Vec::new(),
            vec!["SKIP: no chromium binary on PATH".to_string()],
            serde_json::json!({"backend": "skipped-no-chromium"}),
        );
    }
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let port_num = free_port();
    let data_dir = next_temp_dir("w29-chromium-180");
    let mut browser = spawn_headless_chromium(port_num, &data_dir)?;
    let browser_pid = browser.id();
    let cport = ChromiumPort::new(port_num).map_err(fixture_err)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while cport.targets().is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    // NOTE: data: URLs are rejected for top-frame navigation (the
    // target stays at about:blank), so the round trip uses a file://
    // URL — no network needed, still a real browser navigation.
    let page_path = data_dir.join("title.html");
    std::fs::write(
        &page_path,
        "<html><head><title>Wave 29 Test Page</title></head><body>hi</body></html>",
    )
    .map_err(|e| TaskDriverError::Fixture {
        what: "fixture".to_string(),
        detail: format!("task-180: writing title page failed: {e}"),
    })?;
    let url = format!("file://{}", page_path.display());
    match cport.open_url_and_wait_title(
        &url,
        "Wave 29 Test Page",
        std::time::Duration::from_secs(15),
    ) {
        Ok(target) => {
            evidence.push(format!(
                "real chromium: opened file:// URL, observed title '{}' on target {}",
                target.title, target.raw_id
            ));
            let mut cport = cport;
            let _ = cport.close_target(&target.raw_id);
        }
        Err(e) => failures.push(format!("title round trip failed: {e}")),
    }
    let _ = browser.kill();
    let _ = browser.wait();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let lingering = process_census().into_iter().any(|e| e.pid == browser_pid);
    if lingering {
        failures.push(format!("chromium pid {browser_pid} survived teardown"));
    }
    let _ = std::fs::remove_dir_all(&data_dir);
    finish_case(
        CASES[3],
        failures,
        evidence,
        serde_json::json!({
            "title": "Wave 29 Test Page",
            "census_clean": !lingering,
            "backend": "real-chromium",
        }),
    )
}

/// Spawn a headless chromium with a debugging port for the title
/// round trip. Caller owns teardown (kill, wait, census, rmdir).
fn spawn_headless_chromium(
    port_num: u16,
    data_dir: &std::path::Path,
) -> Result<std::process::Child, TaskDriverError> {
    std::process::Command::new("chromium")
        .arg("--headless=new")
        .arg("--no-sandbox")
        .arg("--disable-gpu")
        .arg(format!("--remote-debugging-port={port_num}"))
        .arg(format!("--user-data-dir={}", data_dir.display()))
        .arg("about:blank")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| TaskDriverError::Fixture {
            what: "chromium".to_string(),
            detail: format!("task-180: chromium spawn failed: {e}"),
        })
}

fn fixture_err(e: BridgeError) -> TaskDriverError {
    TaskDriverError::Fixture {
        what: "bridge".to_string(),
        detail: format!("task-180: {e}"),
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
        "evaluate_title_typed" => case_evaluate_title_typed(),
        "nested_object_mapped" => case_nested_object_mapped(),
        "undefined_is_null" => case_undefined_is_null(),
        "real_chromium_title_roundtrip" => case_real_chromium_title_roundtrip(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-180: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// title round trip itself.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-180".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-180".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
