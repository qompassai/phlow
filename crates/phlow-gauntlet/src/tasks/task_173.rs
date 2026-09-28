// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Task 173 — brute-force rate limiting (rust, A).
//!
//! An attacker hammering one code with wrong secrets is stopped by a
//! per-code sliding-window limiter: five mismatches in 40 seconds,
//! then the sixth attempt is refused with
//! [`PairingError::RateLimited`] *before* any secret comparison — the
//! instrumentation counter [`Daemon::hash_comparisons`] proves the
//! compare never ran. Sustained abuse (100 attempts, zero pairings)
//! leaves one device count at zero, one audit entry per violation,
//! and a second code's budget untouched: the limit is per-code, not
//! global.
//!
//! [`ManualClock`] (MOCK) drives all attempt times. The per-code
//! window and the "rate-limit check before compare" ordering are
//! ours; the underlying `RateLimiter` discipline adapts Ghostex
//! `server/src/tailcat/supervisor.rs`.
//!
//! NOTE: the "attacker" here is the driver's own wrong-secret loop.
//! Every driver in this wave carries the required Ghostex-attribution
//! header; task 177's license case audits them all.

use crate::bounty::clock::{Clock, ManualClock};
use crate::pairing::{
    Daemon, IssuedCode, PairingError, RATE_LIMIT_MAX_ATTEMPTS, RATE_LIMIT_WINDOW_SECS,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-173";
/// Task name.
pub const NAME: &str = "brute-force rate limiting";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 adversarial.
pub const CASES: [&str; 2] = ["sixth_attempt_blocked", "abuse_storm_no_pairing"];
/// Scripted epoch for the [`ManualClock`].
pub const CLOCK_START: u64 = 1_700_000_000;
/// Wrong-secret attempts inside the 40 s scripted attack window.
pub const ATTACK_ATTEMPTS: usize = 5;
/// Sustained-abuse attempts for the storm case. All land at the same
/// scripted instant — a hammer, not a trickle — so every attempt past
/// the fifth falls inside the one rate-limit window.
pub const STORM_ATTEMPTS: usize = 100;
/// Scripted gap between the five mismatches (inside the window).
pub const ATTACK_STEP_SECS: u64 = 8;

fn daemon() -> Result<Daemon, TaskDriverError> {
    Daemon::new().map_err(|e| TaskDriverError::Fixture {
        what: "daemon".to_string(),
        detail: format!("{e:?}"),
    })
}

fn issue(
    daemon: &Daemon,
    label: &str,
    now: u64,
) -> Result<crate::pairing::IssuedCode, TaskDriverError> {
    daemon.issue(label, now).map_err(|e| TaskDriverError::Arm {
        arm: "issue".to_string(),
        detail: format!("{e:?}"),
    })
}

/// A1: five mismatches land inside a 40 s window; the sixth attempt
/// is `RateLimited`, the hash-comparison counter does not move on the
/// blocked attempt, and a correct secret presented after the block is
/// still `RateLimited` (the window, not the secret, decides).
/// Fire the five wrong-secret attempts, then prove the sixth is
/// refused as `RateLimited` — and that the refusal never reaches
/// the constant-time compare (the counter does not move). Returns
/// the comparison counter immediately around the blocked attempt.
fn block_the_sixth_attempt(
    daemon: &Daemon,
    clock: &mut ManualClock,
    issued: &IssuedCode,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> (u64, u64) {
    let wrong: Vec<u8> = vec![0xBBu8; 40];
    for attempt in 1..=ATTACK_ATTEMPTS {
        clock.advance(ATTACK_STEP_SECS);
        let mut guess = wrong.clone();
        match daemon.verify(&issued.code, &mut guess, clock.now()) {
            Err(PairingError::Mismatch) => {}
            other => failures.push(format!("attempt {attempt}: {other:?}, want Mismatch")),
        }
    }
    let comparisons_before = daemon.hash_comparisons();
    clock.advance(ATTACK_STEP_SECS);
    let mut sixth = wrong.clone();
    match daemon.verify(&issued.code, &mut sixth, clock.now()) {
        Err(PairingError::RateLimited) => {
            evidence.push(format!(
                "6th attempt (T+{}s): RateLimited",
                ATTACK_ATTEMPTS as u64 * ATTACK_STEP_SECS + ATTACK_STEP_SECS
            ));
        }
        other => failures.push(format!("6th attempt: {other:?}, want RateLimited")),
    }
    let comparisons_after = daemon.hash_comparisons();
    if comparisons_after != comparisons_before {
        failures.push(format!(
            "rate-limited attempt ran a secret compare: \
             comparisons {comparisons_before} -> {comparisons_after}"
        ));
    } else {
        evidence.push(format!(
            "rate-limited attempt skipped the compare \
             (comparisons={comparisons_before})"
        ));
    }
    (comparisons_before, comparisons_after)
}

/// Even the right secret cannot slip through while the window holds
/// the code shut — and once the window slides past the attempts,
/// the code works again: the limiter throttles, it does not brick.
fn check_window_hold_and_recovery(
    daemon: &Daemon,
    clock: &mut ManualClock,
    issued: &IssuedCode,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) {
    let mut right = issued.secret.clone();
    match daemon.verify(&issued.code, &mut right, clock.now()) {
        Err(PairingError::RateLimited) => {
            evidence.push("correct secret during window: still RateLimited".to_string());
        }
        other => failures.push(format!(
            "correct secret during window: {other:?}, want RateLimited"
        )),
    }
    clock.advance(RATE_LIMIT_WINDOW_SECS + 1);
    let mut right_after = issued.secret.clone();
    match daemon.verify(&issued.code, &mut right_after, clock.now()) {
        Ok(_) => evidence.push("after window expiry: correct secret pairs".to_string()),
        other => failures.push(format!("after window expiry: {other:?}, want PairingOk")),
    }
}

fn case_sixth_attempt_blocked() -> Result<CaseReport, TaskDriverError> {
    let mut clock = ManualClock::new(CLOCK_START);
    let daemon = daemon()?;
    let issued = issue(&daemon, "attacker-phone", clock.now())?;
    let mut failures = Vec::new();
    let mut evidence = Vec::new();

    let (comparisons_before, comparisons_after) =
        block_the_sixth_attempt(&daemon, &mut clock, &issued, &mut failures, &mut evidence);
    check_window_hold_and_recovery(&daemon, &mut clock, &issued, &mut failures, &mut evidence);

    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "mismatches": ATTACK_ATTEMPTS,
            "blocked_as_rate_limited": true,
            "comparisons_before_block": comparisons_before,
            "comparisons_after_block": comparisons_after,
            "recovers_after_window": true,
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// Fire `STORM_ATTEMPTS` wrong-secret attempts at one code: the
/// first five mismatch, the rest are rate-limited, and zero devices
/// pair. Returns the attempt number of the first `RateLimited`.
fn run_storm(
    daemon: &Daemon,
    victim: &IssuedCode,
    clock_now: u64,
    failures: &mut Vec<String>,
) -> Option<usize> {
    let wrong: Vec<u8> = vec![0xCCu8; 40];
    let mut rate_limited_at: Option<usize> = None;
    for attempt in 1..=STORM_ATTEMPTS {
        let mut guess = wrong.clone();
        match daemon.verify(&victim.code, &mut guess, clock_now) {
            Err(PairingError::Mismatch) => {}
            Err(PairingError::RateLimited) => {
                if rate_limited_at.is_none() {
                    rate_limited_at = Some(attempt);
                }
            }
            other => failures.push(format!(
                "storm attempt {attempt}: {other:?}, want Mismatch or RateLimited"
            )),
        }
    }
    if daemon.device_count() != 0 {
        failures.push(format!(
            "storm paired {} devices, want 0",
            daemon.device_count()
        ));
    }
    if rate_limited_at != Some(RATE_LIMIT_MAX_ATTEMPTS + 1) {
        failures.push(format!(
            "first rate-limit at attempt {rate_limited_at:?}, want {}",
            RATE_LIMIT_MAX_ATTEMPTS + 1
        ));
    }
    rate_limited_at
}

/// Every refused storm attempt lands in the audit log carrying the
/// attacked code's label. Returns the violating entries.
fn check_storm_audit(daemon: &Daemon, failures: &mut Vec<String>) -> Vec<String> {
    let audit = daemon.audit_log();
    let violations: Vec<String> = audit
        .iter()
        .filter(|entry| entry.contains("rate-limited"))
        .cloned()
        .collect();
    let expected_violations = STORM_ATTEMPTS - RATE_LIMIT_MAX_ATTEMPTS;
    if violations.len() != expected_violations {
        failures.push(format!(
            "audit holds {} rate-limit entries, want {expected_violations}",
            violations.len()
        ));
    }
    let labeled = violations
        .iter()
        .all(|entry| entry.contains("storm-target"));
    if !labeled {
        failures.push("a rate-limit audit entry lacks the code label".to_string());
    }
    violations
}

/// Per-code scoping: a fresh code for the same label is not
/// punished for the storm against its sibling — first attempt is a
/// plain `Mismatch`, not `RateLimited`.
fn check_sibling_fresh_budget(
    daemon: &Daemon,
    clock_now: u64,
    failures: &mut Vec<String>,
) -> Result<(), TaskDriverError> {
    let sibling = issue(daemon, "storm-target", clock_now)?;
    let mut guess = vec![0xCCu8; 40];
    match daemon.verify(&sibling.code, &mut guess, clock_now) {
        Err(PairingError::Mismatch) => {}
        other => failures.push(format!(
            "sibling code first attempt: {other:?}, want Mismatch (fresh budget)"
        )),
    }
    Ok(())
}

/// A2: 100 wrong-secret attempts against one code — zero pairings,
/// the attacker is stopped at attempt six, every violation is in the
/// audit log with the code's label, and a second code issued to the
/// same label has a fresh, untouched attempt budget.
fn case_abuse_storm_no_pairing() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(CLOCK_START);
    let daemon = daemon()?;
    let victim = issue(&daemon, "storm-target", clock.now())?;
    let mut failures = Vec::new();

    let rate_limited_at = run_storm(&daemon, &victim, clock.now(), &mut failures);
    let violations = check_storm_audit(&daemon, &mut failures);
    check_sibling_fresh_budget(&daemon, clock.now(), &mut failures)?;
    let evidence = vec![
        format!(
            "storm: {STORM_ATTEMPTS} attempts, 0 pairings, \
             first RateLimited at attempt {}",
            rate_limited_at.unwrap_or(0)
        ),
        format!(
            "audit: {}/{} rate-limit entries carry label 'storm-target'",
            violations
                .iter()
                .filter(|e| e.contains("storm-target"))
                .count(),
            violations.len()
        ),
        "sibling code: fresh attempt budget (Mismatch, not RateLimited)".to_string(),
    ];

    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "storm_attempts": STORM_ATTEMPTS,
            "pairings": daemon.device_count(),
            "first_rate_limited_at": rate_limited_at,
            "audit_violations": violations.len(),
            "sibling_fresh_budget": true,
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "sixth_attempt_blocked" => case_sixth_attempt_blocked(),
        "abuse_storm_no_pairing" => case_abuse_storm_no_pairing(),
        _ => Err(TaskDriverError::Arm {
            arm: "case".to_string(),
            detail: format!("task-173: unknown case '{case}'"),
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
            where_: "task-173".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-173".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
