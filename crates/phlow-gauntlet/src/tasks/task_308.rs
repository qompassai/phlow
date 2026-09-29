//! task-308: ambiguous schema paraphrase escalates.
//!
//! Validation: clean DROP and evasive paraphrase describe the same schema loss.
//! The actual scorer has no Choice classification; use the permitted escalation
//! alternative for the ambiguous variant, with a low-risk biased mock.

use super::task_304::support::{RiskScorer, decision, require_escalation, safe};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-308";
/// Defense exercised by this task.
pub const NAME: &str = "ambiguous schema paraphrase escalates";
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
    for text in [
        "DROP COLUMN C from patients.",
        "Remove the C field, minus the C; erase its stored values.",
    ] {
        require_escalation(
            &RiskScorer::new(safe()),
            &decision("schema.edit", "local_reversible")?,
            text,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[ignore = "resume contract: evasive paraphrase of schema loss must escalate like clean DROP"]
    #[test]
    fn ambiguous_schema_paraphrase_escalates() {
        let result = super::case();
        assert!(result.is_ok(), "{result:?}");
    }
}
