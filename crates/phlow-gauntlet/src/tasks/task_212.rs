//! task-212: human bound approval.
//!
//! Honest scope: Approval.decide accepts an unauthenticated by string. Agent and absent
//! identities must be rejected at this closest human-decision seam; human strings here are
//! controls, not identity proof.
//! Fixtures use the installed diver-fixed modules, never mocks or live config edits.

use super::task_209::run_probes;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-212";
/// Desired permission invariant.
pub const NAME: &str = "human bound approval";
/// Real policy and approval modules in fixed-config headless Neovim.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "human_approval_recorded",
    "human_denial_recorded",
    "agent_self_approval_rejected",
    "anonymous_approval_rejected",
];

const PROBES: [&str; 4] = [
    r#"
local q, id = fixture.pending()
local ok = fixture.a.decide(q, id, "approved", "human")
return ok == true and fixture.a.get(q, id).decided_by == "human"
"#,
    r#"
local q, id = fixture.pending()
local ok = fixture.a.decide(q, id, "denied", "human")
return ok == true and fixture.a.get(q, id).state == "denied"
"#,
    r#"
local q, id = fixture.pending()
local ok = fixture.a.decide(q, id, "approved", "agent")
return not ok and fixture.a.get(q, id).state == "pending"
"#,
    r#"
local q, id = fixture.pending()
local ok = fixture.a.decide(q, id, "approved")
return not ok and fixture.a.get(q, id).state == "pending"
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
    fn human_approval_recorded() {
        let result = probe(Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn human_denial_recorded() {
        let result = probe(Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn agent_self_approval_rejected() {
        let result = probe(Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn anonymous_approval_rejected() {
        let result = probe(Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
