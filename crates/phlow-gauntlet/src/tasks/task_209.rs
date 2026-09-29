//! task-209: structured denial records.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. `decide` is the denial seam; a
//! denial must retain the denied tool and exact scope as typed data. No event emission is claimed.
//! This file also hosts the shared JSON fixtures for the rescoped approval tasks.

use phlow_approval::{ApprovalQueue, DEFAULT_TTL, Policy, Record, Request, Risk, Verdict, decide};
use serde_json::{Value, json};

use super::task_233::{collect_outcome, err};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-209";
/// Desired permission invariant.
pub const NAME: &str = "structured denial records";
/// Drives the phlow-approval policy seam directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "default_denial_is_typed",
    "explicit_denial_has_reason",
    "denial_retains_tool",
    "denial_retains_exact_paths",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    let scope = req()?.scope().clone();
    match index {
        0 => {
            let empty = Policy::from_json(&json!({ "version": 1 })).map_err(err)?;
            let d = decide(Some(&empty), &scope);
            Ok(d.verdict == Verdict::Deny && d.scope.risk() == Risk::LocalReversible)
        }
        1 => {
            let d = decide(Some(&policy("deny")?), &scope);
            Ok(d.verdict == Verdict::Deny && !d.reason.is_empty())
        }
        2 => {
            let d = decide(None, &scope);
            Ok(d.verdict == Verdict::Deny && d.scope.tool() == "fs.write")
        }
        3 => Ok(decide(None, &scope).scope.paths() == ["/work/a"]),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

// Shared fixtures: JSON inputs and queue construction only. Every admission, decision and read
// calls the real phlow-approval API.

/// Run id used by every fixture request.
pub(super) const RUN: &str = "run";
/// The two configured operators; neither is a reserved model identity.
pub(super) const HUMAN: &str = "human";
pub(super) const OPERATOR: &str = "operator";

/// The baseline request: one tool, one risk class, one path.
pub(super) fn req_json() -> Value {
    json!({ "tool": "fs.write", "risk": "local_reversible", "paths": ["/work/a"] })
}

/// The baseline request with `extra`'s fields set or replaced.
pub(super) fn req_with(extra: Value) -> Result<Value, String> {
    let mut value = req_json();
    let fields = extra.as_object().ok_or("fixture extra must be an object")?;
    for (key, item) in fields {
        value[key] = item.clone();
    }
    Ok(value)
}

pub(super) fn req() -> Result<Request, String> {
    Request::from_json(&req_json()).map_err(err)
}

/// A rule granting or denying exactly the baseline scope.
pub(super) fn rule(decision: &str) -> Value {
    json!({
        "risk": "local_reversible",
        "decision": decision,
        "tools": ["fs.write"],
        "paths": ["/work/a"],
    })
}

pub(super) fn policy_of(rules: Value) -> Result<Policy, String> {
    Policy::from_json(&json!({ "version": 1, "default": "deny", "rules": rules })).map_err(err)
}

pub(super) fn policy(decision: &str) -> Result<Policy, String> {
    policy_of(json!([rule(decision)]))
}

/// Two allow rules for `/work/a` and `/work/b` separately.
pub(super) fn split_policy() -> Result<Policy, String> {
    let mut second = rule("allow");
    second["paths"] = json!(["/work/b"]);
    policy_of(json!([rule("allow"), second]))
}

/// Verdict for an untrusted request document under `policy`.
pub(super) fn verdict_for(policy: Option<&Policy>, request: &Value) -> Result<Verdict, String> {
    let request = Request::from_json(request).map_err(err)?;
    Ok(decide(policy, request.scope()).verdict)
}

pub(super) fn queue() -> Result<ApprovalQueue, String> {
    ApprovalQueue::new(&[HUMAN, OPERATOR]).map_err(err)
}

/// Admit `input` into a fresh queue; return the queue and the pending ID.
pub(super) fn pending_from(input: &Value) -> Result<(ApprovalQueue, String), String> {
    let mut queue = queue()?;
    let request = Request::from_json(input).map_err(err)?;
    let id = queue.request(RUN, request, DEFAULT_TTL).map_err(err)?;
    Ok((queue, id))
}

pub(super) fn pending() -> Result<(ApprovalQueue, String), String> {
    pending_from(&req_json())
}

pub(super) fn record(queue: &ApprovalQueue, id: &str) -> Result<Record, String> {
    queue.get(id).ok_or_else(|| format!("record {id} missing"))
}

/// Collect four case outcomes with the shared evidence/failure reporting.
pub(super) fn run_cases(
    id: &str,
    cases: &[&'static str; 4],
    case: fn(usize) -> Result<bool, String>,
) -> TaskOutcome {
    collect_outcome(
        id,
        cases.iter().enumerate().map(|(i, name)| (*name, case(i))),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_denial_is_typed() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn explicit_denial_has_reason() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn denial_retains_tool() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn denial_retains_exact_paths() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
