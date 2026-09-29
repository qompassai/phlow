//! task-222: emergency approval revocation.
//!
//! Honest scope: No revoke API exists. The closest existing transition is decide(denied)
//! after approval; it must remove authority immediately while preserving human attribution.
//! Executor integration remains unproven.
//! Fixtures use the installed diver-fixed modules, never mocks or live config edits.

use super::task_209::run_probes;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-222";
/// Desired permission invariant.
pub const NAME: &str = "emergency approval revocation";
/// Real policy and approval modules in fixed-config headless Neovim.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "pending_can_be_denied",
    "denied_cannot_be_reapproved",
    "approved_can_be_revoked",
    "revocation_visible_on_next_read",
];

const PROBES: [&str; 4] = [
    r#"
local q, id = fixture.pending()
fixture.a.decide(q, id, "denied", "human")
return fixture.a.get(q, id).state == "denied"
"#,
    r#"
local q, id = fixture.pending()
fixture.a.decide(q, id, "denied", "human")
return not fixture.a.decide(q, id, "approved", "human")
"#,
    r#"
local q, id = fixture.pending()
fixture.a.decide(q, id, "approved", "human")
fixture.a.decide(q, id, "denied", "human")
return fixture.a.get(q, id).state ~= "approved"
"#,
    r#"
local q, id = fixture.pending()
fixture.a.decide(q, id, "approved", "human")
local snapshot = vim.deepcopy(fixture.a.get(q, id))
fixture.a.decide(q, id, "denied", "human")
return snapshot.state == "approved" and fixture.a.get(q, id).state == "denied"
"#,
];

/// Run four bounded real-seam probes; retain all case outcomes, including failures.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_probes(ctx, ID, &CASES, &PROBES)
}

#[cfg(test)]
mod tests {
    use super::{CASES, PROBES};
    use crate::tasks::task_209::probe;
    use std::path::Path;

    #[test]
    fn pending_can_be_denied() {
        let result = probe(Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn denied_cannot_be_reapproved() {
        let result = probe(Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn approved_can_be_revoked() {
        let result = probe(Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn revocation_visible_on_next_read() {
        let result = probe(Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
