// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 156 — variant-confusion refusal (rust, A).
//!
//! The seam is open-enum dispatch after task-151. `Other(String)`
//! must be a dead end for dispatch, not a backdoor: an attacker
//! crafting a variant name that *looks* privileged must not reach a
//! privileged handler. The driver feeds hostile kind spellings (MOCK
//! hostile peer) through [`crate::wire::dispatch`]. The privileged
//! arm in this model is [`crate::wire::Dispatch::Subscribe`] — it
//! mutates daemon state — and only the exact `"subscribe"` spelling
//! reaches it.

use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::wire::{Dispatch, Kind, dispatch};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-156";
/// Task name.
pub const NAME: &str = "variant-confusion refusal";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 4 adversarial (the 4th is the license gate).
pub const CASES: [&str; 4] = [
    "confusable_variants_defaulted",
    "other_never_normalized",
    "zero_privileged_dispatches",
    "license_header_present",
];

/// Hostile kind spellings: a fake-privileged name, a case variant of
/// a known kind, whitespace smuggling, an embedded NUL, and Unicode
/// confusables (Cyrillic і/е).
const HOSTILE: [&str; 7] = [
    "admin_override",
    "Ping",
    "PING",
    " subscribe",
    "subscribe ",
    "pi\0ng",
    "pіng",
];

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// A1: every hostile spelling lands in `Other(..)` and routes to the
/// default handler — none reaches a known dispatch arm.
fn case_confusable_variants_defaulted() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut defaulted = 0u32;
    for spelling in HOSTILE {
        let kind = Kind::from_wire(spelling);
        if !matches!(kind, Kind::Other(_)) {
            failures.push(format!("{spelling:?} escaped Other: {kind:?}"));
            continue;
        }
        match dispatch(&kind) {
            Dispatch::Default { raw } if raw == spelling => defaulted += 1,
            other => failures.push(format!("{spelling:?} dispatched to {other:?}")),
        }
    }
    if defaulted as usize != HOSTILE.len() {
        failures.push(format!("defaulted {defaulted}, want {}", HOSTILE.len()));
    }
    let evidence = vec![format!(
        "{} hostile spellings (admin_override, case variants, NUL, confusables): \
         all Other(..) -> default handler",
        HOSTILE.len()
    )];
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "hostile_spellings": HOSTILE.len(),
            "defaulted": defaulted,
            "backend": "hostile-fixture",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2: the `Other` payload is captured verbatim — never case-folded,
/// trimmed, or Unicode-normalized before dispatch. A normalizing
/// dispatch would turn `"Ping"` into `Ping`; this one must not.
fn case_other_never_normalized() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    for spelling in HOSTILE {
        let kind = Kind::from_wire(spelling);
        match &kind {
            Kind::Other(kept) => {
                if kept != spelling {
                    failures.push(format!(
                        "{spelling:?} was normalized to {kept:?} before dispatch"
                    ));
                }
                if kind.as_str() != spelling {
                    failures.push(format!("{spelling:?} as_str changed the payload"));
                }
            }
            other => failures.push(format!("{spelling:?} not captured: {other:?}")),
        }
    }
    // And the exact known spelling still dispatches to its arm.
    if dispatch(&Kind::from_wire("ping")) != Dispatch::Ping {
        failures.push("\"ping\" did not dispatch to Ping".to_string());
    }
    let evidence = vec![format!(
        "{} hostile payloads byte-identical through from_wire/as_str/dispatch; \
         \"ping\" still reaches the Ping arm",
        HOSTILE.len()
    )];
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "verbatim": failures.is_empty(),
            "backend": "hostile-fixture",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Zero privileged dispatches from unknown variants: across every
/// hostile spelling the `Subscribe` arm (the privileged one) fires
/// zero times, while the exact `"subscribe"` spelling still reaches
/// it exactly once — the arm exists, attackers just cannot touch it.
fn case_zero_privileged_dispatches() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut privileged = 0u32;
    let mut default = 0u32;
    for spelling in HOSTILE {
        match dispatch(&Kind::from_wire(spelling)) {
            Dispatch::Subscribe => privileged += 1,
            Dispatch::Default { .. } => default += 1,
            _ => failures.push(format!("{spelling:?} reached a non-default known arm")),
        }
    }
    if privileged != 0 {
        failures.push(format!(
            "{privileged} privileged dispatches from hostile variants"
        ));
    }
    if default as usize != HOSTILE.len() {
        failures.push(format!("default {default}, want {}", HOSTILE.len()));
    }
    // Control: the privileged arm is reachable by its exact spelling.
    match dispatch(&Kind::from_wire("subscribe")) {
        Dispatch::Subscribe => {}
        other => failures.push(format!("exact \"subscribe\" gave {other:?}")),
    }
    let evidence = vec![format!(
        "privileged (Subscribe) dispatches from {} hostile variants: {privileged}; \
         exact \"subscribe\" still reaches it",
        HOSTILE.len()
    )];
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "privileged_dispatches": privileged,
            "default_dispatches": default,
            "control_reaches_subscribe": true,
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
    crate::tasks::task_158::check_attribution(&["src/wire.rs", "src/tasks/task_156.rs"])
        .map(|mut r| {
            r.case = CASES[3].to_string();
            r
        })
        .map_err(|e| arm_error("license", e))
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "confusable_variants_defaulted" => case_confusable_variants_defaulted(),
        "other_never_normalized" => case_other_never_normalized(),
        "zero_privileged_dispatches" => case_zero_privileged_dispatches(),
        "license_header_present" => case_license_header_present(),
        _ => Err(arm_error(
            "case",
            format!("task-156: unknown case '{case}'"),
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
            where_: "task-156".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-156".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
