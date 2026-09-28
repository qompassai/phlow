// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 157 — version-skew games (rust, A).
//!
//! The seam is version dispatch after task-154. Version numbers are
//! attacker-controlled: downgrade replays and absurd versions must
//! fail closed. The driver runs a hostile version-skewed peer (MOCK)
//! against [`crate::wire::Session`], the per-session version state.
//! The mental model is TUF-style version monotonicity: the negotiated
//! version is a high-water mark that never moves down.

use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::wire::{Session, VersionError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-157";
/// Task name.
pub const NAME: &str = "version-skew games";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 4 adversarial (the 4th is the license gate).
pub const CASES: [&str; 4] = [
    "downgrade_rejected_session_intact",
    "absurd_version_rejected_pre_dispatch",
    "monotonic_tracking",
    "license_header_present",
];

/// The spec's absurd version: 2^32 - 1.
const ABSURD_VERSION: u64 = 4_294_967_295;

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// A1: the peer negotiates v3, then sends v1 envelopes (a downgrade
/// to a weaker parser) → `VersionError::Downgrade`; the session stays
/// at v3 and keeps working.
fn case_downgrade_rejected_session_intact() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut session = Session::negotiate(3).map_err(|e| {
        arm_error(
            "negotiate",
            format!("task-157: v3 negotiation refused: {e:?}"),
        )
    })?;
    session
        .accept(3)
        .map_err(|e| arm_error("accept", format!("task-157: v3 accept refused: {e:?}")))?;
    match session.accept(1) {
        Err(VersionError::Downgrade { got: 1, session: 3 }) => {}
        other => failures.push(format!("v1 after v3 gave {other:?}, want Downgrade")),
    }
    if session.negotiated() != 3 {
        failures.push(format!(
            "session moved to v{}, want it kept at v3",
            session.negotiated()
        ));
    }
    if session.accepted() != 1 {
        failures.push(format!(
            "accepted {}, want 1 (the downgrade must not count)",
            session.accepted()
        ));
    }
    // The session is still usable at v3 afterwards.
    if let Err(e) = session.accept(3) {
        failures.push(format!("session unusable after downgrade: {e:?}"));
    }
    let evidence = vec![format!(
        "v3 session + v1 envelope -> Downgrade{{got:1, session:3}}; \
         session kept at v{}, accepted={}",
        session.negotiated(),
        session.accepted()
    )];
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "downgrade_typed": "Downgrade",
            "session_version": session.negotiated(),
            "accepted": session.accepted(),
            "backend": "hostile-fixture",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2: version 4,294,967,295 is rejected as unsupported *before* any
/// parser dispatch — the bound check is an if-chain, never
/// `parsers[version]`, so there is nothing to index or overflow.
/// Version 0 is `TooOld`. Neither touches session state.
fn case_absurd_version_rejected_pre_dispatch() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut session = Session::negotiate(3).map_err(|e| {
        arm_error(
            "negotiate",
            format!("task-157: v3 negotiation refused: {e:?}"),
        )
    })?;
    match session.accept(ABSURD_VERSION) {
        Err(VersionError::Unsupported { got, max: 3 }) if got == ABSURD_VERSION => {}
        other => failures.push(format!(
            "version {ABSURD_VERSION} gave {other:?}, want Unsupported (pre-dispatch)"
        )),
    }
    match session.accept(0) {
        Err(VersionError::TooOld { got: 0, min: 1 }) => {}
        other => failures.push(format!("version 0 gave {other:?}, want TooOld")),
    }
    if session.negotiated() != 3 || session.accepted() != 0 {
        failures.push(format!(
            "session state touched by rejected versions: v{}, accepted={}",
            session.negotiated(),
            session.accepted()
        ));
    }
    let evidence = vec![format!(
        "version {ABSURD_VERSION} -> Unsupported{{max:3}} pre-dispatch; \
         version 0 -> TooOld; session untouched (v3, accepted=0)"
    )];
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "absurd_version": ABSURD_VERSION,
            "absurd_typed": "Unsupported",
            "zero_typed": "TooOld",
            "session_untouched": true,
            "backend": "hostile-fixture",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Monotonic per-session version tracking: the mark only moves up.
/// After 1→2→3, a replayed 2 is a downgrade and the mark stays 3.
fn case_monotonic_tracking() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut session = Session::negotiate(1).map_err(|e| {
        arm_error(
            "negotiate",
            format!("task-157: v1 negotiation refused: {e:?}"),
        )
    })?;
    for v in [1u64, 2, 3] {
        if let Err(e) = session.accept(v) {
            failures.push(format!("monotonic accept({v}) refused: {e:?}"));
        }
    }
    if session.negotiated() != 3 {
        failures.push(format!("mark {}, want 3", session.negotiated()));
    }
    if session.accepted() != 3 {
        failures.push(format!("accepted {}, want 3", session.accepted()));
    }
    match session.accept(2) {
        Err(VersionError::Downgrade { got: 2, session: 3 }) => {}
        other => failures.push(format!("replayed v2 gave {other:?}, want Downgrade")),
    }
    if session.negotiated() != 3 {
        failures.push("mark moved down after downgrade replay".to_string());
    }
    let evidence = vec![format!(
        "accept 1,2,3 -> mark v{}, accepted={}; replayed v2 -> Downgrade, mark stays v3",
        session.negotiated(),
        session.accepted()
    )];
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "mark": session.negotiated(),
            "accepted": session.accepted(),
            "downgrade_rejected": true,
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
    crate::tasks::task_158::check_attribution(&["src/wire.rs", "src/tasks/task_157.rs"])
        .map(|mut r| {
            r.case = CASES[3].to_string();
            r
        })
        .map_err(|e| arm_error("license", e))
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "downgrade_rejected_session_intact" => case_downgrade_rejected_session_intact(),
        "absurd_version_rejected_pre_dispatch" => case_absurd_version_rejected_pre_dispatch(),
        "monotonic_tracking" => case_monotonic_tracking(),
        "license_header_present" => case_license_header_present(),
        _ => Err(arm_error(
            "case",
            format!("task-157: unknown case '{case}'"),
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
            where_: "task-157".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-157".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
