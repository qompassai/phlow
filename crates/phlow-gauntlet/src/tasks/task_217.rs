//! task-217: conflicting policy rules deny.
//!
//! Honest scope: Real policy.decide currently uses first match. A conflict must deny
//! regardless of order; construction-time rejection is also acceptable.
//! Fixtures use the installed diver-fixed modules, never mocks or live config edits.

use super::task_209::run_probes;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-217";
/// Desired permission invariant.
pub const NAME: &str = "conflicting policy rules deny";
/// Real policy and approval modules in fixed-config headless Neovim.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "single_allow_works",
    "single_deny_works",
    "allow_then_deny_denies",
    "deny_then_allow_denies",
];

const PROBES: [&str; 4] = [
    r#"
return fixture.p.decide(fixture.policy("allow"), fixture.req()).decision == "allow"
"#,
    r#"
return fixture.p.decide(fixture.policy("deny"), fixture.req()).decision == "deny"
"#,
    r#"
local result = fixture.p.new({ rules = { fixture.rule("allow"), fixture.rule("deny") } })
return fixture.p.decide(result, fixture.req()).decision == "deny"
"#,
    r#"
local result = fixture.p.new({ rules = { fixture.rule("deny"), fixture.rule("allow") } })
return fixture.p.decide(result, fixture.req()).decision == "deny"
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
    fn single_allow_works() {
        let result = probe(Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn single_deny_works() {
        let result = probe(Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn allow_then_deny_denies() {
        let result = probe(Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn deny_then_allow_denies() {
        let result = probe(Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
