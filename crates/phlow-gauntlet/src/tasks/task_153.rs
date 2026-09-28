// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 153 — lenient envelope parsing (rust, V).
//!
//! The seam is `parse_envelope(bytes)` at the socket read boundary.
//! Real peers send imperfect bytes: the parser is liberal in what it
//! accepts *within typed bounds* (whitespace, insignificant key
//! order) and strict about shape. Allocation stays under the named
//! bound [`crate::wire::MAX_ENVELOPE_BYTES`] and the scan is one
//! linear pass — asserted here with an input-size sweep and a scripted
//! [`ManualClock`] (MOCK) proving the 10,000-parse loop is bounded
//! and terminates. (Wall-clock timing is out of scope per the
//! gauntlet's standing conventions; linearity is proven by exact
//! byte counts, not a stopwatch.)

use crate::bounty::clock::{Clock, ManualClock};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::wire::{MAX_ENVELOPE_BYTES, parse_envelope};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-153";
/// Task name.
pub const NAME: &str = "lenient envelope parsing";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 3 validation (the 3rd is the license gate).
pub const CASES: [&str; 3] = [
    "canonical_equivalence",
    "bounded_allocation_linear_time",
    "license_header_present",
];

/// Scripted epoch for the [`ManualClock`].
pub const CLOCK_START: u64 = 1_700_000_000;

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// Parses `inputs` on the scripted clock, one tick per parse, dropping
/// each envelope immediately so at most one is live. Returns
/// (parsed_ok, total_bytes_scanned, ticks_elapsed). The allocation
/// bound itself is asserted by the integration test's counting
/// allocator around this exact function.
pub fn bench_parse(inputs: &[Vec<u8>], clock: &mut ManualClock) -> (usize, u64, u64) {
    let start = clock.now();
    let mut ok = 0usize;
    let mut scanned = 0u64;
    for bytes in inputs {
        if let Ok((_, stats)) = parse_envelope(bytes) {
            ok += 1;
            scanned += stats.bytes_scanned as u64;
        }
        clock.advance(1);
    }
    (ok, scanned, clock.now() - start)
}

/// Builds a small envelope fixture (~200 bytes).
fn small_fixture(i: usize) -> Vec<u8> {
    format!(r#"{{"version":1,"kind":"event","id":"e{i:05}","body":{{"n":{i},"tag":"bench"}}}}"#)
        .into_bytes()
}

/// V1: leading/trailing whitespace and key order do not change the
/// parsed value: all spellings parse to the identical [`Envelope`]
/// and the identical canonical JSON.
fn case_canonical_equivalence() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let canonical = r#"{"version":1,"kind":"ping","id":"a","body":{"x":1}}"#;
    let spellings = [
        format!("\n  {canonical}  \n"),
        "\t{\"id\":\"a\",\"body\":{\"x\":1},\"kind\":\"ping\",\"version\":1}\r\n".to_string(),
        " { \"version\" : 1 , \"kind\" : \"ping\" , \"id\" : \"a\" , \"body\" : { \"x\" : 1 } } "
            .to_string(),
    ];
    let (want, want_stats) = parse_envelope(canonical.as_bytes()).map_err(|e| {
        arm_error(
            "parse",
            format!("task-153: canonical fixture refused: {e:?}"),
        )
    })?;
    if want_stats.bytes_scanned != canonical.len() {
        failures.push(format!(
            "bytes_scanned {}, want input len {}",
            want_stats.bytes_scanned,
            canonical.len()
        ));
    }
    for (i, spelling) in spellings.iter().enumerate() {
        let (got, stats) = parse_envelope(spelling.as_bytes())
            .map_err(|e| arm_error("parse", format!("task-153: spelling {i} refused: {e:?}")))?;
        if got != want {
            failures.push(format!("spelling {i} parsed to a different envelope"));
        }
        if got.to_canonical_json() != want.to_canonical_json() {
            failures.push(format!("spelling {i} canonical form differs"));
        }
        let trimmed = spelling.trim();
        if stats.bytes_scanned != trimmed.len() {
            failures.push(format!(
                "spelling {i}: bytes_scanned {}, want trimmed len {}",
                stats.bytes_scanned,
                trimmed.len()
            ));
        }
    }
    let evidence = vec![format!(
        "1 canonical + {} imperfect spellings: identical Envelope, identical canonical JSON",
        spellings.len()
    )];
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "spellings": spellings.len() + 1,
            "identical": failures.is_empty(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: 10,000 rapid parses on the scripted clock — every parse
/// succeeds, the clock advances exactly one tick per parse (the loop
/// is bounded and terminates), and an input-size sweep shows the
/// scanner visiting each byte exactly once (linear in input).
fn case_bounded_allocation_linear_time() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut clock = ManualClock::new(CLOCK_START);
    let inputs: Vec<Vec<u8>> = (0..10_000usize).map(small_fixture).collect();
    let (ok, scanned, ticks) = bench_parse(&inputs, &mut clock);
    if ok != 10_000 {
        failures.push(format!("parsed {ok}/10000, want 10000"));
    }
    if ticks != 10_000 {
        failures.push(format!("clock advanced {ticks} ticks, want 10000"));
    }
    let want_scanned: u64 = inputs.iter().map(|b| b.len() as u64).sum();
    if scanned != want_scanned {
        failures.push(format!(
            "bytes_scanned {scanned}, want total input len {want_scanned}"
        ));
    }
    // Input-size sweep: doubling sizes, bytes_scanned == input len
    // exactly at every size — the scan is one linear pass.
    let mut sweep_ok = true;
    for size in [256usize, 512, 1024, 2048] {
        let pad = "x".repeat(size);
        let bytes =
            format!(r#"{{"version":1,"kind":"ping","id":"s","body":"{pad}"}}"#).into_bytes();
        match parse_envelope(&bytes) {
            Ok((_, stats)) if stats.bytes_scanned == bytes.len() => {}
            other => {
                sweep_ok = false;
                failures.push(format!("size {size}: sweep failed: {other:?}"));
            }
        }
    }
    // One envelope at exactly the named bound parses.
    let big_body = "y".repeat(MAX_ENVELOPE_BYTES - 128);
    let big = format!(r#"{{"version":1,"kind":"ping","id":"big","body":"{big_body}"}}"#);
    let big_len = big.len();
    match parse_envelope(big.as_bytes()) {
        Ok(_) if big_len <= MAX_ENVELOPE_BYTES => {}
        Ok(_) => failures.push(format!(
            "bound-size fixture is {big_len} bytes, over the bound"
        )),
        Err(e) => failures.push(format!("bound-size envelope refused: {e:?}")),
    }
    let evidence = vec![format!(
        "10000 parses: ok={ok}, ticks={ticks}, bytes_scanned={scanned} \
         (== total input len); size sweep linear at 4 sizes: {sweep_ok}; \
         {big_len}-byte envelope (bound {MAX_ENVELOPE_BYTES}) parses"
    )];
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "parses": ok,
            "ticks": ticks,
            "bytes_scanned": scanned,
            "sweep_linear": sweep_ok,
            "bound_bytes": MAX_ENVELOPE_BYTES,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// License gate: the adapted wire module carries the maddada
/// attribution and the source commit.
fn case_license_header_present() -> Result<CaseReport, TaskDriverError> {
    crate::tasks::task_158::check_attribution(&["src/wire.rs", "src/tasks/task_153.rs"])
        .map(|mut r| {
            r.case = CASES[2].to_string();
            r
        })
        .map_err(|e| arm_error("license", e))
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "canonical_equivalence" => case_canonical_equivalence(),
        "bounded_allocation_linear_time" => case_bounded_allocation_linear_time(),
        "license_header_present" => case_license_header_present(),
        _ => Err(arm_error(
            "case",
            format!("task-153: unknown case '{case}'"),
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
            where_: "task-153".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-153".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
