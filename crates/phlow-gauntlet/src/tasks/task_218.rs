//! task-218: wildcard scope escalation.
//!
//! Honest scope: The path and endpoint allowlists in policy.decide are the closest effective-
//! grant seam. Every requested resource must fit the grant; these do not claim glob syntax is
//! supported.
//! Fixtures use the installed diver-fixed modules, never mocks or live config edits.

use super::task_209::run_probes;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-218";
/// Desired permission invariant.
pub const NAME: &str = "wildcard scope escalation";
/// Real policy and approval modules in fixed-config headless Neovim.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "exact_path_allowed",
    "unmatched_path_denied",
    "mixed_paths_denied",
    "mixed_endpoints_denied",
];

const PROBES: [&str; 4] = [
    r#"
return fixture.p.decide(fixture.policy("allow"), fixture.req()).decision == "allow"
"#,
    r#"
local r = fixture.req()
r.paths = { "/work/b" }
return fixture.p.decide(fixture.policy("allow"), r).decision == "deny"
"#,
    r#"
local r = fixture.req()
r.paths = { "/work/a", "/work/b" }
return fixture.p.decide(fixture.policy("allow"), r).decision == "deny"
"#,
    r#"
local r = fixture.req()
r.endpoints = { "https://allowed.invalid", "https://other.invalid" }
local grant = fixture.rule("allow")
grant.endpoints = { "https://allowed.invalid" }
return fixture.p.decide(fixture.p.new({ rules = { grant } }), r).decision == "deny"
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
    fn exact_path_allowed() {
        let result = probe(Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn unmatched_path_denied() {
        let result = probe(Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn mixed_paths_denied() {
        let result = probe(Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn mixed_endpoints_denied() {
        let result = probe(Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
