//! task-224: partial approvals cannot combine authority.
//!
//! Honest scope: No multi-approver grant combiner exists. Policy.decide is the closest
//! effective scope seam: separate rules cannot authorize a compound request unless one covers
//! its entire scope.
//! Fixtures use the installed diver-fixed modules, never mocks or live config edits.

use super::task_209::run_probes;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-224";
/// Desired permission invariant.
pub const NAME: &str = "partial approvals cannot combine authority";
/// Real policy and approval modules in fixed-config headless Neovim.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "first_scope_allowed",
    "second_scope_allowed",
    "union_of_scopes_denied",
    "union_with_unapproved_scope_denied",
];

const PROBES: [&str; 4] = [
    r#"
return fixture.p.decide(fixture.split_policy(), fixture.req()).decision == "allow"
"#,
    r#"
local r = fixture.req()
r.paths = { "/work/b" }
return fixture.p.decide(fixture.split_policy(), r).decision == "allow"
"#,
    r#"
local r = fixture.req()
r.paths = { "/work/a", "/work/b" }
return fixture.p.decide(fixture.split_policy(), r).decision == "deny"
"#,
    r#"
local r = fixture.req()
r.paths = { "/work/a", "/work/b", "/work/c" }
return fixture.p.decide(fixture.split_policy(), r).decision == "deny"
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
    fn first_scope_allowed() {
        let result = probe(Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn second_scope_allowed() {
        let result = probe(Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn union_of_scopes_denied() {
        let result = probe(Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn union_with_unapproved_scope_denied() {
        let result = probe(Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
