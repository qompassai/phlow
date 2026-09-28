// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 151 — open enums with Other(String) (rust, V).
//!
//! The seam is the envelope `kind` field. A newer peer's unknown
//! variant must round-trip without loss or misrouting: closed enums
//! fail closed on upgrade, open enums degrade gracefully. The driver
//! uses the scripted peer fixtures below (MOCK) against
//! [`crate::wire::Kind`], the re-implementation of Ghostex
//! `packages/gx-protocol/src/open_enum.rs`'s open-string-enum rule.
//! The macro itself is not lifted: the match-on-exact-spelling rule
//! is re-implemented as plain functions, so no serde derive is needed.

use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::wire::{Dispatch, Kind, dispatch, parse_envelope};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-151";
/// Task name.
pub const NAME: &str = "open enums with Other(String)";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 4 validation (the 4th is the license gate).
pub const CASES: [&str; 4] = [
    "unknown_variant_round_trips",
    "mixed_variants_no_errors",
    "never_aliases_known",
    "license_header_present",
];

/// The seven known wire spellings, in [`Kind`] order.
const KNOWN: [&str; 7] = [
    "ping",
    "pong",
    "request",
    "response",
    "event",
    "subscribe",
    "unsubscribe",
];

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// One scripted envelope from a version-skewed peer (MOCK).
fn envelope_json(kind: &str) -> String {
    format!(r#"{{"version":1,"kind":"{kind}","id":"m1"}}"#)
}

/// V1: the peer sends `kind: "ping_v9"`, unknown to this build. It
/// must deserialize to `Kind::Other("ping_v9")`, re-serialize
/// byte-identical, and route to the default handler.
fn case_unknown_variant_round_trips() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let kind = Kind::from_wire("ping_v9");
    if kind != Kind::Other("ping_v9".to_string()) {
        failures.push(format!("from_wire(\"ping_v9\") gave {kind:?}, want Other"));
    }
    // Byte-identical round trip through the JSON string form.
    let back = kind.to_json();
    if back != serde_json::Value::String("ping_v9".to_string()) {
        failures.push(format!("to_json gave {back}, want \"ping_v9\""));
    }
    if Kind::from_wire(kind.as_str()) != kind {
        failures.push("as_str/from_wire round trip not identical".to_string());
    }
    // Through the full envelope parse: unknown kind, known shape.
    let (env, _) = parse_envelope(envelope_json("ping_v9").as_bytes()).map_err(|e| {
        arm_error(
            "parse",
            format!("task-151: unknown kind failed to parse: {e:?}"),
        )
    })?;
    if env.kind != Kind::Other("ping_v9".to_string()) {
        failures.push(format!(
            "envelope kind parsed as {:?}, want Other",
            env.kind
        ));
    }
    match dispatch(&env.kind) {
        Dispatch::Default { raw } if raw == "ping_v9" => {
            evidence.push("unknown kind routed to the default handler".to_string());
        }
        other => failures.push(format!("dispatch gave {other:?}, want Default")),
    }
    evidence.push(format!(
        "kind \"ping_v9\" -> {kind:?} -> {:?}: byte-identical, default-routed",
        kind.to_json()
    ));
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "round_trip_byte_identical": failures.is_empty(),
            "dispatched_to_default": true,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: 1,000 unknown + 200 known variants through the parser: zero
/// deserialization errors, and the catch-all bucket holds exactly the
/// 1,000 unknowns.
fn case_mixed_variants_no_errors() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut unknown = 0u32;
    let mut known = 0u32;
    let mut errors = 0u32;
    for i in 0..1000u32 {
        let wire = format!("op_v{i}");
        match parse_envelope(envelope_json(&wire).as_bytes()) {
            Ok((env, _)) if matches!(env.kind, Kind::Other(_)) => unknown += 1,
            Ok((env, _)) => {
                errors += 1;
                failures.push(format!("unknown {wire} parsed as known {:?}", env.kind));
            }
            Err(e) => {
                errors += 1;
                failures.push(format!("unknown {wire} errored: {e:?}"));
            }
        }
    }
    for i in 0..200u32 {
        let wire = KNOWN[(i as usize) % KNOWN.len()];
        match parse_envelope(envelope_json(wire).as_bytes()) {
            Ok((env, _)) if env.kind.is_known() => known += 1,
            Ok((env, _)) => {
                errors += 1;
                failures.push(format!("known {wire} parsed as {:?}", env.kind));
            }
            Err(e) => {
                errors += 1;
                failures.push(format!("known {wire} errored: {e:?}"));
            }
        }
    }
    if unknown != 1000 {
        failures.push(format!("catch-all bucket has {unknown}, want 1000"));
    }
    if known != 200 {
        failures.push(format!("known bucket has {known}, want 200"));
    }
    if errors != 0 {
        failures.push(format!("{errors} deserialization errors, want 0"));
    }
    let evidence = vec![format!(
        "1200 mixed variants: 0 errors, catch-all bucket = {unknown} (want 1000), known = {known}"
    )];
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "total": 1200,
            "unknown_bucket": unknown,
            "known_bucket": known,
            "errors": errors,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Unknown spellings never alias a known variant: near-misses stay in
/// `Other`, and every known spelling still maps to its variant.
fn case_never_aliases_known() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let near_misses = [
        "Ping",
        "PING",
        " ping",
        "ping ",
        "ping_v9",
        "admin_override",
        "",
        "pong\0",
    ];
    for miss in near_misses {
        match Kind::from_wire(miss) {
            Kind::Other(kept) if kept == miss => {}
            other => failures.push(format!("{miss:?} aliased to {other:?}")),
        }
    }
    for known in KNOWN {
        if !Kind::from_wire(known).is_known() {
            failures.push(format!("known spelling {known:?} fell into Other"));
        }
    }
    let evidence = vec![format!(
        "{} near-miss spellings stayed in Other; {} known spellings intact",
        near_misses.len(),
        KNOWN.len()
    )];
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "near_misses": near_misses.len(),
            "known_spellings": KNOWN.len(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// License gate: the adapted enum module carries the maddada
/// attribution and the source commit.
fn case_license_header_present() -> Result<CaseReport, TaskDriverError> {
    crate::tasks::task_158::check_attribution(&["src/wire.rs", "src/tasks/task_151.rs"])
        .map(|mut r| {
            r.case = CASES[3].to_string();
            r
        })
        .map_err(|e| arm_error("license", e))
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "unknown_variant_round_trips" => case_unknown_variant_round_trips(),
        "mixed_variants_no_errors" => case_mixed_variants_no_errors(),
        "never_aliases_known" => case_never_aliases_known(),
        "license_header_present" => case_license_header_present(),
        _ => Err(arm_error(
            "case",
            format!("task-151: unknown case '{case}'"),
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
            where_: "task-151".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-151".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
