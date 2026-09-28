// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 155 — hostile frame rejection (rust, A).
//!
//! The seam is `parse_envelope(bytes)` under attacker-controlled
//! input: the parser is a trust boundary. Truncation, nesting bombs,
//! and invalid UTF-8 produce typed errors — never panics, never
//! unbounded allocation, never a hang. The driver feeds hostile byte
//! fixtures (MOCK) through the read path with the panic trap armed
//! ([`crate::wire::arm_panic_trap`]); the allocation bounds are
//! asserted by the integration test's counting allocator around the
//! driver's [`feed_hostile`] entry point.

use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::wire::{
    EditorSocket, FrameError, MAX_FRAME_BYTES, SocketError, arm_panic_trap, panic_trapped,
    parse_envelope,
};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-155";
/// Task name.
pub const NAME: &str = "hostile frame rejection";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 5 adversarial (the 5th is the license gate).
pub const CASES: [&str; 5] = [
    "nesting_bomb_depth_bound",
    "oversize_rejected_pre_buffering",
    "truncated_and_encoding_typed",
    "zero_panics_connection_survives",
    "license_header_present",
];

/// A 10,000-deep nested JSON bomb: `[[[[...]]]]`.
pub fn nesting_bomb(depth: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(2 * depth);
    v.extend(std::iter::repeat_n(b'[', depth));
    v.extend(std::iter::repeat_n(b']', depth));
    v
}

/// A 100 MB frame of `A`s with an invalid UTF-8 first byte: proves the
/// size check runs before UTF-8 validation and before any buffering.
pub fn oversize_frame() -> Vec<u8> {
    let mut v = vec![b'A'; 100 * 1024 * 1024];
    v[0] = 0xFF;
    v
}

/// A frame cut off mid-value.
pub fn truncated_frame() -> Vec<u8> {
    br#"{"version":1,"kind":"ping","id":"t","body":{"x":[1,2"#.to_vec()
}

/// Feeds every hostile fixture through one socket and returns
/// (rejected, alive_after). Used by the panic case and by the
/// integration test's allocation measurement.
pub fn feed_hostile(socket: &mut EditorSocket) -> (u64, bool) {
    let fixtures: Vec<Vec<u8>> = vec![
        nesting_bomb(10_000),
        oversize_frame(),
        truncated_frame(),
        vec![0xFF, 0xFE, b'{', b'}'],
        Vec::new(),
    ];
    let mut rejected = 0u64;
    for bytes in &fixtures {
        if socket.read(bytes).is_err() {
            rejected += 1;
        }
    }
    (rejected, socket.is_alive())
}

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// A1: 10,000-deep nesting is rejected at the depth bound with a typed
/// error — no stack overflow (the parse returns), no hang.
fn case_nesting_bomb_depth_bound() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let bomb = nesting_bomb(10_000);
    match parse_envelope(&bomb) {
        Err(FrameError::DepthExceeded { depth, bound }) => {
            if bound != crate::wire::MAX_NESTING {
                failures.push(format!("bound {bound}, want {}", crate::wire::MAX_NESTING));
            }
            if depth <= crate::wire::MAX_NESTING {
                failures.push(format!("reported depth {depth} within bound"));
            }
        }
        other => failures.push(format!("nesting bomb gave {other:?}, want DepthExceeded")),
    }
    // A shallow-but-deep envelope (at the bound) still parses: the
    // bound rejects bombs, not legitimate depth.
    let mut ok_deep = br#"{"version":1,"kind":"ping","id":"d","body":"#.to_vec();
    ok_deep.extend(nesting_bomb(32));
    ok_deep.extend(br#"}"#.iter());
    if parse_envelope(&ok_deep).is_err() {
        failures.push("32-deep nesting rejected, want accepted".to_string());
    }
    let evidence = vec![format!(
        "10000-deep bomb -> DepthExceeded (bound {}); 32-deep parses",
        crate::wire::MAX_NESTING
    )];
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "bomb_depth": 10000,
            "bound": crate::wire::MAX_NESTING,
            "typed": "DepthExceeded",
            "backend": "hostile-fixture",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2: a 100 MB frame on the 1 MB bound is rejected pre-buffering.
/// The fixture's first byte is invalid UTF-8: `TooLarge` (not
/// `Encoding`) proves the size check runs first.
fn case_oversize_rejected_pre_buffering() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let frame = oversize_frame();
    match parse_envelope(&frame) {
        Err(FrameError::TooLarge { got, bound }) => {
            if got != 100 * 1024 * 1024 {
                failures.push(format!("got {got}, want 104857600"));
            }
            if bound != MAX_FRAME_BYTES {
                failures.push(format!("bound {bound}, want {MAX_FRAME_BYTES}"));
            }
        }
        other => failures.push(format!(
            "100MB frame gave {other:?}, want TooLarge (size check before UTF-8)"
        )),
    }
    let evidence = vec![format!(
        "100MB frame (invalid UTF-8 first byte) -> TooLarge on the {MAX_FRAME_BYTES}-byte bound, \
         before UTF-8 validation or buffering"
    )];
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "frame_bytes": frame.len(),
            "bound": MAX_FRAME_BYTES,
            "typed": "TooLarge",
            "backend": "hostile-fixture",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2 continued: truncated frames and invalid UTF-8 are typed, and an
/// empty frame is `Malformed` (complete but shapeless), not
/// `Truncated`.
fn case_truncated_and_encoding_typed() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    match parse_envelope(&truncated_frame()) {
        Err(FrameError::Truncated) => {}
        other => failures.push(format!("cut-off frame gave {other:?}, want Truncated")),
    }
    match parse_envelope(&[0xFF, 0xFE, b'{', b'}']) {
        Err(FrameError::Encoding) => {}
        other => failures.push(format!("invalid UTF-8 gave {other:?}, want Encoding")),
    }
    match parse_envelope(b"") {
        Err(FrameError::Malformed(_)) => {}
        other => failures.push(format!("empty frame gave {other:?}, want Malformed")),
    }
    let evidence = vec![
        "cut-off frame -> FrameError::Truncated".to_string(),
        "invalid UTF-8 -> FrameError::Encoding".to_string(),
        "empty frame -> FrameError::Malformed (not Truncated)".to_string(),
    ];
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "truncated_typed": true,
            "encoding_typed": true,
            "empty_malformed": true,
            "backend": "hostile-fixture",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Zero panics across all hostile inputs, run under the panic trap;
/// the connection survives every rejection and stays usable.
fn case_zero_panics_connection_survives() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    arm_panic_trap();
    let mut socket = EditorSocket::new();
    let (rejected, alive) = feed_hostile(&mut socket);
    if panic_trapped() {
        failures.push("a panic fired under hostile input".to_string());
    }
    if rejected != 5 {
        failures.push(format!("rejected {rejected}/5 hostile fixtures"));
    }
    if !alive || !socket.is_alive() {
        failures.push("connection did not survive the hostile fixtures".to_string());
    }
    // Still usable afterwards: a good envelope reads cleanly.
    let good = br#"{"version":1,"kind":"ping","id":"after"}"#;
    match socket.read(good) {
        Ok(routed) if routed.version == 1 => {}
        other => failures.push(format!("good envelope after attack gave {other:?}")),
    }
    if !matches!(
        socket.read(&nesting_bomb(10_000)),
        Err(SocketError::Frame(FrameError::DepthExceeded { .. }))
    ) {
        failures.push("socket read path did not type the nesting bomb".to_string());
    }
    let evidence = vec![format!(
        "5 hostile fixtures: 0 panics (trap armed), rejected={rejected}, \
         connection up and usable afterwards"
    )];
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "fixtures": 5,
            "panics": 0,
            "rejected": rejected,
            "connection_up": socket.is_alive(),
            "backend": "hostile-fixture",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// License gate: the adapted wire module carries the maddada
/// attribution and the source commit.
fn case_license_header_present() -> Result<CaseReport, TaskDriverError> {
    crate::tasks::task_158::check_attribution(&["src/wire.rs", "src/tasks/task_155.rs"])
        .map(|mut r| {
            r.case = CASES[4].to_string();
            r
        })
        .map_err(|e| arm_error("license", e))
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "nesting_bomb_depth_bound" => case_nesting_bomb_depth_bound(),
        "oversize_rejected_pre_buffering" => case_oversize_rejected_pre_buffering(),
        "truncated_and_encoding_typed" => case_truncated_and_encoding_typed(),
        "zero_panics_connection_survives" => case_zero_panics_connection_survives(),
        "license_header_present" => case_license_header_present(),
        _ => Err(arm_error(
            "case",
            format!("task-155: unknown case '{case}'"),
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
            where_: "task-155".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-155".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
