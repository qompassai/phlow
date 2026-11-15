//! PH-K2 live wiring: the runtime as the fleet's reference
//! consumer of the run contract.
//!
//! Rung 1 landed the bounded cue primitives in `phlow-checks`
//! (INDEX §2.2); this module is where they fire. At the head of
//! every [`crate::Runtime::run`], the runtime drains its own cue
//! directory: expired `run-this` cues are reaped and listed (never
//! waited on), and each live `run-this` addressed to `phlow` is
//! claimed and answered with a `run-done` in the same directory,
//! addressed back to the sender, carrying the original cue's lane
//! labels.
//!
//! The handler runs *named checks from the operator-approved config
//! only*. The payload shape is `{"checks": ["name", …]}`; a cue
//! naming a check the operator never approved is refused whole —
//! verdict `refused`, the unapproved name in the evidence, and
//! nothing runs, not even the approved names beside it. A cue can
//! therefore never widen the set of commands this host will run: it
//! can only ask for a subset of what the operator already approved,
//! executed by the same pinned [`CheckRunner`] the runtime itself
//! uses.
//!
//! Bounds (all named, all enforced here or in the primitives):
//! at most [`CUES_PER_DRAIN_MAX`] cues per run, at most
//! [`CHECKS_PER_CUE_MAX`] checks per cue, evidence bounded to
//! [`EVIDENCE_CHARS_MAX`] characters, answers live for
//! [`RUN_DONE_TTL_MS`]. A drain failure (including a malformed cue
//! file, which the primitives refuse *by name*) is reported on the
//! run report as `cues_error`; it never fails the run itself.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;

use phlow_checks::CheckRunner;
use phlow_checks::cue::{
    CueError, KIND_RUN_THIS, LaneScope, RunDone, consume_run_this, list_cues, reap_expired_cues,
};
use phlow_checks::{CheckPins, CheckStatus};
use phlow_config::CheckConfig;
use phlow_workspace::Workspace;
use serde_json::Value;

/// This server's name in the cue protocol: cues addressed `to`
/// this name are phlow's to consume, and `run-done` answers are
/// sent `from` it.
pub const PHLOW_SERVER_NAME: &str = "phlow";
/// Most cues consumed at the head of one run. The rest wait for
/// the next run; a flood of cues can never starve the run itself.
pub const CUES_PER_DRAIN_MAX: usize = 8;
/// Most checks one cue may name.
pub const CHECKS_PER_CUE_MAX: usize = 32;
/// Longest check name accepted from a cue payload, in characters.
pub const CHECK_NAME_CHARS_MAX: usize = 64;
/// Evidence bound on the `run-done` payload, in characters.
pub const EVIDENCE_CHARS_MAX: usize = 2_000;
/// How long a `run-done` answer stays claimable, in milliseconds.
pub const RUN_DONE_TTL_MS: u64 = 3_600_000;

/// One consumed cue, as the run report records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumedCue {
    /// The consumed `run-this` cue's id.
    pub cue_id: String,
    /// The server that sent the cue.
    pub from: String,
    /// The verdict answered (`ok`, `failed`, or `refused`).
    pub verdict: String,
    /// The `run-done` cue's id.
    pub done_cue_id: String,
}

/// The outcome of one drain: what was answered, what expired.
#[derive(Debug, Clone, Default)]
pub struct CueDrain {
    /// Cues claimed and answered, oldest first.
    pub consumed: Vec<ConsumedCue>,
    /// Ids of expired cues reaped before consuming — listed,
    /// never waited on (the PH-K2 gate).
    pub expired: Vec<String>,
}

/// Drain `dir`: reap expired cues, then consume up to
/// [`CUES_PER_DRAIN_MAX`] live `run-this` cues addressed to
/// phlow, running each cue's named checks in `workspace`.
///
/// # Errors
/// Returns the primitives' typed [`CueError`] — including
/// `MalformedCue` naming the offending file — when the directory
/// cannot be listed or a cue cannot be claimed or answered.
pub fn drain_cues(
    workspace: &Workspace,
    checks: &BTreeMap<String, CheckConfig>,
    pins: &CheckPins,
    dir: &Path,
    scope: LaneScope,
) -> Result<CueDrain, CueError> {
    let expired = reap_expired_cues(dir, phlow_checks::run_contract::now_unix_ms())?;
    let mut consumed = Vec::new();
    for _ in 0..CUES_PER_DRAIN_MAX {
        // Peek the next consumable cue with the primitive's own
        // selection (oldest live `run-this` addressed to phlow):
        // `consume_run_this` returns only the answer cue, and the
        // report wants the consumed cue's id and sender beside it.
        // The runtime is the single writer of its own runs, so the
        // peeked cue is the one the consume takes.
        let now = phlow_checks::run_contract::now_unix_ms();
        let next = list_cues(dir, scope)?.into_iter().find(|cue| {
            cue.kind == KIND_RUN_THIS && cue.to == PHLOW_SERVER_NAME && !cue.is_expired(now)
        });
        let Some(next) = next else { break };
        // The handler's verdict travels out through this cell: the
        // primitive owns the answer cue, the report wants the
        // verdict beside the cue ids.
        let verdict: RefCell<Option<String>> = RefCell::new(None);
        let outcome = consume_run_this(
            dir,
            PHLOW_SERVER_NAME,
            scope,
            RUN_DONE_TTL_MS,
            |_cue, payload| {
                let (answer, evidence) = run_named_checks(workspace, checks, pins, payload);
                *verdict.borrow_mut() = Some(answer.to_owned());
                RunDone {
                    verdict: answer.to_owned(),
                    evidence,
                }
            },
        )?;
        let Some(done) = outcome else { break };
        consumed.push(ConsumedCue {
            cue_id: next.id.clone(),
            from: next.from.clone(),
            verdict: verdict.borrow().clone().unwrap_or_default(),
            done_cue_id: done.id.clone(),
        });
    }
    Ok(CueDrain { consumed, expired })
}

/// Run one cue's named checks. The verdict is `refused` — with
/// nothing run — when the payload is malformed or names a check
/// outside the operator-approved set; otherwise `ok` when every
/// named check passes and `failed` when any does not.
fn run_named_checks(
    workspace: &Workspace,
    checks: &BTreeMap<String, CheckConfig>,
    pins: &CheckPins,
    payload: &[u8],
) -> (&'static str, String) {
    let names = match parse_check_names(payload) {
        Ok(names) => names,
        Err(reason) => return ("refused", format!("refused: {reason}")),
    };
    for name in &names {
        if !checks.contains_key(name) {
            return (
                "refused",
                format!(
                    "refused: check '{name}' is not in the operator-approved check set; \
                     nothing ran"
                ),
            );
        }
    }
    let runner = CheckRunner::with_pins(workspace, checks.clone(), pins.clone());
    let mut all_ok = true;
    let mut lines: Vec<String> = Vec::new();
    for name in &names {
        let report = runner.run_all(Some(name));
        for check in &report.checks {
            if check.status != CheckStatus::Ok {
                all_ok = false;
            }
            let exit = check
                .returncode
                .map(|code| code.to_string())
                .unwrap_or_else(|| "none".to_owned());
            lines.push(format!(
                "{}: {} (exit {exit}, {} ms)",
                check.name,
                check.status.as_str(),
                check.duration_ms
            ));
            if let Some(error) = &check.error {
                lines.push(format!("  error: {error}"));
            }
        }
    }
    let verdict = if all_ok { "ok" } else { "failed" };
    (verdict, bound_evidence(&lines.join("\n")))
}

/// Parse the `run-this` payload: `{"checks": ["name", …]}`.
/// Duplicates collapse (first occurrence wins); every refusal is
/// a reason string the caller puts in the evidence.
fn parse_check_names(payload: &[u8]) -> Result<Vec<String>, String> {
    let value: Value =
        serde_json::from_slice(payload).map_err(|error| format!("payload is not JSON: {error}"))?;
    let list = value
        .get("checks")
        .and_then(Value::as_array)
        .ok_or_else(|| "payload has no 'checks' array".to_owned())?;
    if list.is_empty() {
        return Err("payload names no checks".to_owned());
    }
    if list.len() > CHECKS_PER_CUE_MAX {
        return Err(format!(
            "payload names {} checks; the bound is {CHECKS_PER_CUE_MAX}",
            list.len()
        ));
    }
    let mut names: Vec<String> = Vec::new();
    for item in list {
        let name = item
            .as_str()
            .ok_or_else(|| "every entry in 'checks' must be a string".to_owned())?;
        if name.is_empty() || name.chars().count() > CHECK_NAME_CHARS_MAX {
            return Err(format!(
                "check names must be 1..={CHECK_NAME_CHARS_MAX} characters"
            ));
        }
        if !names.iter().any(|seen| seen == name) {
            names.push(name.to_owned());
        }
    }
    Ok(names)
}

/// Bound evidence text to [`EVIDENCE_CHARS_MAX`] characters,
/// cutting on a char boundary and saying so when cut.
fn bound_evidence(text: &str) -> String {
    if text.chars().count() <= EVIDENCE_CHARS_MAX {
        return text.to_owned();
    }
    let mut bounded: String = text.chars().take(EVIDENCE_CHARS_MAX).collect();
    bounded.push_str(" [truncated]");
    bounded
}

#[cfg(test)]
mod tests {
    use super::{CHECKS_PER_CUE_MAX, EVIDENCE_CHARS_MAX, bound_evidence, parse_check_names};

    #[test]
    fn payload_parsing_accepts_dedupes_and_bounds() {
        let names = parse_check_names(br#"{"checks":["tests","fmt","tests"]}"#).unwrap();
        assert_eq!(names, vec!["tests", "fmt"]);
        assert!(parse_check_names(b"not json").is_err());
        assert!(parse_check_names(br#"{"checks":[]}"#).is_err());
        assert!(parse_check_names(br#"{"checks":[1]}"#).is_err());
        assert!(parse_check_names(br#"{"other":[]}"#).is_err());
        let many = serde_json::json!({"checks": vec!["c"; CHECKS_PER_CUE_MAX + 1]});
        assert!(parse_check_names(&serde_json::to_vec(&many).unwrap()).is_err());
    }

    #[test]
    fn evidence_is_bounded_on_a_char_boundary() {
        let exact = "x".repeat(EVIDENCE_CHARS_MAX);
        assert_eq!(bound_evidence(&exact), exact);
        let over = "é".repeat(EVIDENCE_CHARS_MAX + 10);
        let bounded = bound_evidence(&over);
        assert!(bounded.ends_with(" [truncated]"));
        assert_eq!(
            bounded.chars().count(),
            EVIDENCE_CHARS_MAX + " [truncated]".chars().count()
        );
    }
}
