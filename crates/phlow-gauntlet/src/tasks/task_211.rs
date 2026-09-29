//! task-211: review token integrity.
//!
//! Honest scope: Approval IDs are the closest token seam. Tests cover unknown IDs, decision
//! replay and copied queue state; they do not establish cryptographic entropy or action-token
//! consumption.
//! Fixtures use the installed diver-fixed modules, never mocks or live config edits.

use super::task_209::run_probes;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-211";
/// Desired permission invariant.
pub const NAME: &str = "review token integrity";
/// Real policy and approval modules in fixed-config headless Neovim.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "human_decision_recorded",
    "distinct_requests_have_distinct_ids",
    "forged_id_rejected",
    "copied_consumed_token_rejected",
];

const PROBES: [&str; 4] = [
    r#"
local q, id = fixture.pending()
local ok = fixture.a.decide(q, id, "approved", "human")
return ok == true and fixture.a.get(q, id).state == "approved"
"#,
    r#"
local q, id = fixture.pending()
local other = fixture.a.request(q, "run", fixture.req())
return other ~= nil and id ~= other
"#,
    r#"
local q = fixture.a.new()
local ok = fixture.a.decide(q, "forged-review-id", "approved", "human")
return not ok and #fixture.a.fixture.pending(q) == 0
"#,
    r#"
local q, id = fixture.pending()
local copied = vim.deepcopy(q)
local ok = fixture.a.decide(q, id, "approved", "human")
local replay = fixture.a.decide(q, id, "approved", "human")
local fork = fixture.a.decide(copied, id, "approved", "human")
return ok == true and not replay and not fork
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
    fn human_decision_recorded() {
        let result = probe(Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn distinct_requests_have_distinct_ids() {
        let result = probe(Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn forged_id_rejected() {
        let result = probe(Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn copied_consumed_token_rejected() {
        let result = probe(Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
