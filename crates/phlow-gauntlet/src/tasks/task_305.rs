//! task-305: aliases cannot launder delete.
//!
//! Adversarial: a delete is renamed, then submitted under an unknown identity.
//! Canonical deletion and unresolved identity must both require review.

use super::task_304::support::{RiskScorer, decision, require_escalation, safe};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-305";
/// Defense exercised by this task.
pub const NAME: &str = "aliases cannot launder delete";
/// Offline Rust task using a deterministic mock.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the bounded scenario and preserve failure evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match case() {
        Ok(()) => TaskOutcome::Pass {
            evidence: vec![format!("{ID}: {NAME} passed")],
        },
        Err(how) => TaskOutcome::Fail {
            where_: ID.to_owned(),
            how,
            evidence: vec![],
        },
    }
}

fn case() -> Result<(), String> {
    for (tool, effect) in [
        (
            "fs.delete",
            "Canonical effect: permanently delete /data/records.",
        ),
        (
            "tidy.alias",
            "Registered alias of fs.delete: permanently delete /data/records.",
        ),
        (
            "unknown.entry",
            "Unresolved tool identity; effects unavailable.",
        ),
    ] {
        require_escalation(
            &RiskScorer::new(safe()),
            &decision(tool, "observe")?,
            effect,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[ignore = "resume contract: renamed delete under unknown identity must require review (canonical deletion / identity resolution not implemented)"]
    #[test]
    fn aliases_cannot_launder_delete() {
        let result = super::case();
        assert!(result.is_ok(), "{result:?}");
    }
}
