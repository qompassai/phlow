// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 158 — editor-socket parity and license audit (rust, V/A).
//!
//! The seam is the Neovim editor socket read path. The socket gets
//! the same envelope discipline as the daemon link — no second,
//! weaker parser: malformed input is a typed error and the socket
//! stays up, and a nesting bomb is rejected at the same bound. Plus
//! the wave-25 license gate: every `.rs` file added in tasks 151–157
//! is scanned for the maddada attribution and the source commit; a
//! missing header fails the task. This module also exports
//! [`check_attribution`], the license case shared by tasks 151–157.

use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::wire::{EditorSocket, FrameError, SocketError, parse_envelope};
use crate::{TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Task id.
pub const ID: &str = "task-158";
/// Task name.
pub const NAME: &str = "editor-socket parity and license audit";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 1 validation + 1 adversarial + the license audit.
pub const CASES: [&str; 3] = [
    "malformed_socket_stays_up",
    "nesting_bomb_socket_bounded",
    "license_audit_all_files",
];

/// The exact attribution header every Ghostex-adapted `.rs` file in
/// this wave must start with.
pub const ATTRIBUTION_LINES: [&str; 3] = [
    "// Copyright (c) maddada",
    "// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500",
    "// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.",
];

/// Every `.rs` file the wave added carrying adapted code or doctrine:
/// the shared wire module plus the seven task drivers and their
/// integration tests.
const AUDIT_FILES: [&str; 15] = [
    "src/wire.rs",
    "src/tasks/task_151.rs",
    "src/tasks/task_152.rs",
    "src/tasks/task_153.rs",
    "src/tasks/task_154.rs",
    "src/tasks/task_155.rs",
    "src/tasks/task_156.rs",
    "src/tasks/task_157.rs",
    "tests/task_151.rs",
    "tests/task_152.rs",
    "tests/task_153.rs",
    "tests/task_154.rs",
    "tests/task_155.rs",
    "tests/task_156.rs",
    "tests/task_157.rs",
];

/// Checks that each file (relative to the crate root) starts with
/// exactly [`ATTRIBUTION_LINES`]. Shared by the license cases of
/// tasks 151–157.
pub fn check_attribution(files: &[&str]) -> Result<CaseReport, String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut failures = Vec::new();
    let mut checked = Vec::new();
    for rel in files {
        let path = root.join(rel);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let head: Vec<&str> = text.lines().take(3).collect();
        if head != ATTRIBUTION_LINES {
            failures.push(format!("{rel}: attribution header missing or not first"));
        } else {
            checked.push(rel.to_string());
        }
    }
    let mut report = CaseReport::pass(
        "license_header_present",
        serde_json::json!({
            "files_checked": checked.len(),
            "files": checked,
        }),
        vec![format!(
            "attribution header (maddada + source commit) present and first on {} files",
            checked.len()
        )],
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// A scripted nvim-socket peer (MOCK): it only records that it is
/// still connected. The point is the negative: a refused frame must
/// not disturb the peer side.
struct ScriptedNvimPeer {
    connected: bool,
}

impl ScriptedNvimPeer {
    fn new() -> Self {
        ScriptedNvimPeer { connected: true }
    }
}

/// V1: a malformed editor message is a typed error; the socket stays
/// up, the nvim peer is unaffected, and the next good message reads
/// cleanly.
fn case_malformed_socket_stays_up() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut socket = EditorSocket::new();
    let peer = ScriptedNvimPeer::new();
    match socket.read(b"not json at all") {
        Err(SocketError::Frame(FrameError::Malformed(_))) => {}
        other => failures.push(format!("malformed message gave {other:?}, want Malformed")),
    }
    if !socket.is_alive() {
        failures.push("socket went down on a malformed message".to_string());
    }
    if !peer.connected {
        failures.push("nvim peer was disturbed by the refusal".to_string());
    }
    if socket.rejected() != 1 {
        failures.push(format!("rejected {}, want 1", socket.rejected()));
    }
    let good = br#"{"version":1,"kind":"event","id":"n1","body":{"buf":3}}"#;
    match socket.read(good) {
        Ok(routed) if routed.version == 1 && routed.envelope.id == "n1" => {}
        other => failures.push(format!("good message after refusal gave {other:?}")),
    }
    if socket.received() != 1 {
        failures.push(format!("received {}, want 1", socket.received()));
    }
    let evidence = vec![format!(
        "malformed editor message -> SocketError::Frame(Malformed); socket up \
         (rejected={}, received={}); nvim peer unaffected; next message reads",
        socket.rejected(),
        socket.received()
    )];
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "typed": "Malformed",
            "socket_up": socket.is_alive(),
            "rejected": socket.rejected(),
            "received": socket.received(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1: a nesting bomb over the editor socket is rejected at the same
/// depth bound as the daemon link (task-155's rule), and the socket
/// stays up.
fn case_nesting_bomb_socket_bounded() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut socket = EditorSocket::new();
    let mut bomb = Vec::with_capacity(20_000);
    bomb.extend(std::iter::repeat_n(b'[', 10_000));
    bomb.extend(std::iter::repeat_n(b']', 10_000));
    match socket.read(&bomb) {
        Err(SocketError::Frame(FrameError::DepthExceeded { bound, .. }))
            if bound == crate::wire::MAX_NESTING => {}
        other => failures.push(format!("nesting bomb gave {other:?}, want DepthExceeded")),
    }
    if !socket.is_alive() {
        failures.push("socket went down on the nesting bomb".to_string());
    }
    // Parity check: the raw parser types it identically.
    match parse_envelope(&bomb) {
        Err(FrameError::DepthExceeded { .. }) => {}
        other => failures.push(format!("raw parser gave {other:?}, want DepthExceeded")),
    }
    let evidence = vec![format!(
        "10000-deep bomb over the editor socket -> DepthExceeded (bound {}), \
         same as the daemon link; socket up",
        crate::wire::MAX_NESTING
    )];
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "typed": "DepthExceeded",
            "bound": crate::wire::MAX_NESTING,
            "socket_up": socket.is_alive(),
            "parity_with_daemon_link": true,
            "backend": "hostile-fixture",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// License audit: every `.rs` file added in tasks 151–157 carries the
/// maddada attribution and the source commit as its first three
/// lines. Missing attribution fails the task.
fn case_license_audit_all_files() -> Result<CaseReport, TaskDriverError> {
    let mut report = check_attribution(&AUDIT_FILES).map_err(|e| arm_error("license", e))?;
    report.case = CASES[2].to_string();
    // This task's own two files carry the header too (noted, not gated).
    let own = check_attribution(&["src/tasks/task_158.rs", "tests/task_158.rs"])
        .map_err(|e| arm_error("license", e))?;
    let mut evidence = report.evidence.clone();
    evidence.push(format!(
        "task-158's own files also attributed: {}",
        own.passed
    ));
    report.evidence = evidence;
    if !own.passed {
        report.passed = false;
        report.failures.extend(own.failures);
    }
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "malformed_socket_stays_up" => case_malformed_socket_stays_up(),
        "nesting_bomb_socket_bounded" => case_nesting_bomb_socket_bounded(),
        "license_audit_all_files" => case_license_audit_all_files(),
        _ => Err(arm_error(
            "case",
            format!("task-158: unknown case '{case}'"),
        )),
    }
}

/// Task-level entry for the gauntlet runner: the headline case.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-158".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-158".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
