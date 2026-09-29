//! task-208: credential invalidation and stale context copies.
//!
//! Drives `ConversationContext::clear`, `messages`, and `Clone`.
//!
//! Honest scope: clear is conversation reset, NOT credential revocation. There
//! is no broker revocation API. The prerequisite tested here is that a snapshot
//! or clone retained by the agent cannot carry raw credential values beyond a
//! reset; admission-time redaction is one valid solution. This does not assert
//! that clear magically mutates owned snapshots, nor prove immediate revocation
//! of outstanding opaque handles. Add that integration once handles exist.

use super::task_201::{CANARY, case_index, report, run_cases};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use phlow_agent::ConversationContext;

/// Stable task identifier.
pub const ID: &str = "task-208";
/// Human-readable invariant.
pub const NAME: &str = "credential invalidation and stale context copies";
/// Offline Rust seams; no network, subprocess, or Neovim required.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation controls, then two adversarial credential probes.
pub const CASES: [&str; 4] = [
    "clear_removes_current_messages",
    "fresh_context_usable_after_clear",
    "old_snapshot_cannot_reveal_credential",
    "old_clone_cannot_reveal_credential",
];

/// Run one bounded in-memory probe; unknown names return a driver error.
/// Fixtures contain at most two calls/messages and less than 1 KiB of input.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    let index = case_index(case, &CASES)?;
    let mut context = ConversationContext::new(2);
    let input = if index < 2 {
        "ordinary task".to_owned()
    } else {
        format!("api_key={CANARY}")
    };
    context.add_message("tool", &input);
    let snapshot = context.messages();
    let retained = context.clone();
    context.clear();
    let (passed, output_bytes) = match index {
        0 => (context.is_empty() && context.messages().is_empty(), 0),
        1 => {
            context.add_message("user", "fresh task");
            let fresh = context.messages();
            (
                fresh.len() == 1 && fresh[0].content == "fresh task",
                fresh.iter().map(|message| message.content.len()).sum(),
            )
        }
        2 => (
            context.is_empty()
                && snapshot
                    .iter()
                    .all(|message| !message.content.contains(CANARY)),
            snapshot.iter().map(|message| message.content.len()).sum(),
        ),
        _ => {
            let stale = retained.messages();
            (
                context.is_empty()
                    && stale
                        .iter()
                        .all(|message| !message.content.contains(CANARY)),
                stale.iter().map(|message| message.content.len()).sum(),
            )
        }
    };
    Ok(report(
        CASES[index],
        passed,
        output_bytes,
        "ConversationContext reset and retained copies",
    ))
}

/// Run all four cases, retaining only boolean/count evidence, never payloads.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, run_case)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(index: usize) {
        let result = run_case(CASES[index]).expect("bounded fixture must execute");
        assert!(result.passed, "{}", result.failures.join("; "));
    }

    #[test]
    fn clear_removes_current_messages() {
        check(0);
    }

    #[test]
    fn fresh_context_usable_after_clear() {
        check(1);
    }

    #[test]
    fn old_snapshot_cannot_reveal_credential() {
        check(2);
    }

    #[test]
    fn old_clone_cannot_reveal_credential() {
        check(3);
    }
}
