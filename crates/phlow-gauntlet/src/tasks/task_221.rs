//! task-221: denial proposal round trip.
//!
//! Honest scope: Feeds actual policy.decide output to approval.request. The desired denial
//! record must be sufficient for exact proposal creation without fishing for extra
//! permissions; no invented denial fixture is used.
//! Fixtures use the installed diver-fixed modules, never mocks or live config edits.

use super::task_209::run_probes;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-221";
/// Desired permission invariant.
pub const NAME: &str = "denial proposal round trip";
/// Real policy and approval modules in fixed-config headless Neovim.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "request_scope_survives_queue",
    "request_risk_survives_queue",
    "denial_is_sufficient_for_proposal",
    "denial_round_trip_preserves_only_scope",
];

const PROBES: [&str; 4] = [
    r#"
local q, id = fixture.pending()
return vim.deep_equal(fixture.a.get(q, id).paths, fixture.req().paths)
"#,
    r#"
local q, id = fixture.pending()
return fixture.a.get(q, id).risk == fixture.req().risk
"#,
    r#"
local d = fixture.p.decide(nil, fixture.req())
local q = fixture.a.new()
local id = fixture.a.request(q, "run", d)
return id ~= nil and fixture.a.get(q, id).tool == fixture.req().tool
"#,
    r#"
local d = fixture.p.decide(nil, fixture.req())
local q = fixture.a.new()
local id = fixture.a.request(q, "run", d)
if id == nil then
	return false
end
local proposal = fixture.a.get(q, id)
return vim.deep_equal(proposal.paths, fixture.req().paths) and proposal.endpoints == nil and proposal.argv == nil
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
    fn request_scope_survives_queue() {
        let result = probe(Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn request_risk_survives_queue() {
        let result = probe(Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn denial_is_sufficient_for_proposal() {
        let result = probe(Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn denial_round_trip_preserves_only_scope() {
        let result = probe(Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
