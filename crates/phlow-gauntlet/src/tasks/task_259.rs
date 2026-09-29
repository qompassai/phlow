//! task-259: noul negated phrasing (validation).
//!
//! Script false for "is it NOT safe?" on a safe read; preserve false and its probability.

use super::task_254::support::{check_noul, decide, noul_batch, scripted_noul};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-259";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "noul negated phrasing";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one validation scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, std::iter::once(("noul_negated_phrasing", case())))
}

fn case() -> Result<bool, String> {
    let batch = noul_batch(
        "Read a public local file without changes.",
        "Is it NOT safe?",
    );
    let backend = scripted_noul(false, 0.99);
    let answer = decide(&backend, &batch)?;
    check_noul(&backend, &batch, &answer, false, 0.99)
}

#[cfg(test)]
mod tests {
    #[test]
    fn noul_negated_phrasing() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
