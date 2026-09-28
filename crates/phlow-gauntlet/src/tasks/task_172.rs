// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Task 172 — constant-time secret compare (rust, V).
//!
//! The presented secret is never compared with an early-exit loop:
//! it is hashed, then the two 32-byte digests are compared by a
//! constant-time routine that visits all 32 bytes on match AND on
//! mismatch, while the caller-supplied secret buffer is zeroized on
//! every return path. The driver measures real accept/reject loop
//! times (a sane epsilon bounds their difference), asserts the
//! 32-byte visit count on both outcomes, and scans the store and the
//! audit log for any plaintext secret.
//!
//! Adapts the hash-compared secret check from Ghostex
//! `server/src/remote_access/pairing_code.rs`; the visit-count
//! instrumentation, the timing bound, and the zeroization contract
//! are ours.

use crate::bounty::clock::{Clock, ManualClock};
use crate::pairing::{Daemon, PairingError, ct_compare, sha256_bytes, sha256_hex};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-172";
/// Task name.
pub const NAME: &str = "constant-time secret compare";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation.
pub const CASES: [&str; 2] = ["timing_side_channel_bounded", "secret_zeroized_everywhere"];
/// Scripted epoch for the [`ManualClock`].
pub const CLOCK_START: u64 = 1_700_000_000;
/// Verify iterations per timing arm.
pub const TIMING_ITERS: usize = 2_000;
/// Accept/reject loop times must differ by less than this factor.
/// A correct constant-time compare lands near 1.0; an early-exit
/// byte loop would show several x on a 32-byte digest.
pub const TIMING_FACTOR_BOUND: f64 = 2.0;

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

/// Time one closure in nanoseconds per iteration, averaged over
/// `TIMING_ITERS` runs. The index lets each iteration use its own
/// pre-issued code.
fn time_per_iter<F>(mut work: F) -> u128
where
    F: FnMut(usize),
{
    let started = std::time::Instant::now();
    for i in 0..TIMING_ITERS {
        work(i);
    }
    started.elapsed().as_nanos() / TIMING_ITERS as u128
}

/// V1: real accept and reject loops. Correct secret accepts; a
/// 32-byte wrong secret mismatches; the two per-iteration times stay
/// within `TIMING_FACTOR_BOUND`. The instrumented
/// [`ct_compare`] is also asserted to visit exactly 32 bytes on both
/// the match and the mismatch paths.
///
/// Each iteration verifies a freshly issued code: a correct verify
/// consumes its code, and repeated wrong secrets against one code
/// trip the rate limiter — reusing codes would measure the
/// `Consumed`/`RateLimited` paths instead of the compare.
/// Issue fresh codes for every timing iteration (each code is
/// single-use, so one code per iteration) and time accept vs
/// reject per iteration in nanoseconds.
fn time_accept_reject(daemon: &Daemon, clock_now: u64) -> Result<(u128, u128), TaskDriverError> {
    let mut accept_codes = Vec::with_capacity(TIMING_ITERS);
    let mut reject_codes = Vec::with_capacity(TIMING_ITERS);
    for i in 0..TIMING_ITERS {
        accept_codes.push(issue(daemon, &format!("timing-accept-{i}"), clock_now)?);
        reject_codes.push(issue(daemon, &format!("timing-reject-{i}"), clock_now)?);
    }
    let wrong: Vec<u8> = vec![0xA5; 32];
    let accept_ns = time_per_iter(|i| {
        let issued = &accept_codes[i];
        let mut secret = issued.secret.clone();
        let _ = daemon.verify(&issued.code, &mut secret, clock_now);
    });
    let reject_ns = time_per_iter(|i| {
        let issued = &reject_codes[i];
        let mut secret = wrong.clone();
        let _ = daemon.verify(&issued.code, &mut secret, clock_now);
    });
    Ok((accept_ns, reject_ns))
}

/// Sanity: the timed arms actually did the work the timing claims —
/// a correct secret pairs, a wrong one mismatches.
fn sanity_timed_arms(
    daemon: &Daemon,
    clock_now: u64,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> Result<(), TaskDriverError> {
    let accept_probe = issue(daemon, "timing-accept-probe", clock_now)?;
    let reject_probe = issue(daemon, "timing-reject-probe", clock_now)?;
    let mut accept_buf = accept_probe.secret.clone();
    let probe_ok = daemon.verify(&accept_probe.code, &mut accept_buf, clock_now);
    let mut reject_buf = vec![0xA5; 32];
    let probe_reject = daemon.verify(&reject_probe.code, &mut reject_buf, clock_now);
    match (&probe_ok, &probe_reject) {
        (Ok(_), Err(PairingError::Mismatch)) => {
            evidence.push("sanity: correct accepts, wrong mismatches".to_string());
        }
        other => failures.push(format!("sanity arms wrong: {other:?}, want (Ok, Mismatch)")),
    }
    Ok(())
}

/// Visit-count proof on the primitive itself: `ct_compare` visits
/// all 32 bytes on match and on mismatch.
fn prove_constant_time_primitive(
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> (u64, u64) {
    let digest = sha256_bytes(b"timing-probe");
    let mut other = digest;
    other[0] ^= 0x01;
    let (match_ok, match_visited) = ct_compare(&digest, &digest);
    let (mismatch_ok, mismatch_visited) = ct_compare(&digest, &other);
    if !match_ok || match_visited != 32 {
        failures.push(format!(
            "ct_compare match: ok={match_ok} visited={match_visited}, want ok=true visited=32"
        ));
    }
    if mismatch_ok || mismatch_visited != 32 {
        failures.push(format!(
            "ct_compare mismatch: ok={mismatch_ok} visited={mismatch_visited}, \
             want ok=false visited=32"
        ));
    }
    evidence.push(format!(
        "ct_compare: match visited={match_visited}, mismatch visited={mismatch_visited}"
    ));
    (match_visited, mismatch_visited)
}

fn case_timing_side_channel_bounded() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(CLOCK_START);
    let daemon = daemon()?;
    let mut failures = Vec::new();
    let mut evidence = Vec::new();

    let (accept_ns, reject_ns) = time_accept_reject(&daemon, clock.now())?;
    sanity_timed_arms(&daemon, clock.now(), &mut failures, &mut evidence)?;
    let factor = if accept_ns == 0 || reject_ns == 0 {
        f64::INFINITY
    } else {
        (accept_ns.max(reject_ns) as f64) / (accept_ns.min(reject_ns) as f64)
    };
    if factor > TIMING_FACTOR_BOUND {
        failures.push(format!(
            "accept/reject factor {factor:.2} exceeds bound {TIMING_FACTOR_BOUND}"
        ));
    }
    evidence.push(format!(
        "timing: accept={accept_ns}ns/iter reject={reject_ns}ns/iter \
         factor={factor:.2} bound={TIMING_FACTOR_BOUND}"
    ));
    let (match_visited, mismatch_visited) =
        prove_constant_time_primitive(&mut failures, &mut evidence);

    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "accept_ns_per_iter": accept_ns,
            "reject_ns_per_iter": reject_ns,
            "factor": factor,
            "factor_bound": TIMING_FACTOR_BOUND,
            "ct_match_visited": match_visited,
            "ct_mismatch_visited": mismatch_visited,
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// V2: the caller's secret buffer is all-zero after verify on the
/// accept path and on the mismatch path, and neither the store dump
/// nor the audit log contains the plaintext secret.
fn case_secret_zeroized_everywhere() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(CLOCK_START);
    let daemon = daemon()?;
    let mut failures = Vec::new();
    let mut evidence = Vec::new();

    let accept = issue(&daemon, "zeroize-accept", clock.now())?;
    let reject = issue(&daemon, "zeroize-reject", clock.now())?;
    let mut accept_buf = accept.secret.clone();
    let _ = daemon.verify(&accept.code, &mut accept_buf, clock.now());
    let accept_zero = accept_buf.iter().all(|b| *b == 0);
    if !accept_zero {
        failures.push("accept path left secret bytes in the caller buffer".to_string());
    }
    let mut reject_buf = vec![0x5Au8; 40];
    let _ = daemon.verify(&reject.code, &mut reject_buf, clock.now());
    let reject_zero = reject_buf.iter().all(|b| *b == 0);
    if !reject_zero {
        failures.push("mismatch path left secret bytes in the caller buffer".to_string());
    }
    evidence.push(format!(
        "zeroization: accept buffer all-zero={accept_zero}, \
         mismatch buffer all-zero={reject_zero}"
    ));

    // Store + audit scan: the secret must appear nowhere, while the
    // hash appears (positive control, same as task 170).
    let mut haystack = daemon.store_dump();
    for entry in daemon.audit_log() {
        haystack.extend_from_slice(entry.as_bytes());
        haystack.push(b'\n');
    }
    let secret_present = haystack
        .windows(accept.secret.len())
        .any(|window| window == accept.secret.as_slice());
    if secret_present {
        failures.push("secret plaintext found in store dump or audit log".to_string());
    }
    let hash_hex = sha256_hex(&accept.secret);
    let hash_present = String::from_utf8_lossy(&haystack).contains(&hash_hex);
    if !hash_present {
        failures.push("secret hash missing from store dump (positive control)".to_string());
    }
    evidence.push(format!(
        "scan: secret_present={secret_present} hash_present={hash_present} \
         bytes_scanned={}",
        haystack.len()
    ));

    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "accept_buffer_zero": accept_zero,
            "mismatch_buffer_zero": reject_zero,
            "secret_in_store_or_audit": secret_present,
            "hash_in_store": hash_present,
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
        "timing_side_channel_bounded" => case_timing_side_channel_bounded(),
        "secret_zeroized_everywhere" => case_secret_zeroized_everywhere(),
        _ => Err(TaskDriverError::Arm {
            arm: "case".to_string(),
            detail: format!("task-172: unknown case '{case}'"),
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
            where_: "task-172".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-172".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
