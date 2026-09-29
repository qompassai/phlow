//! task-214: unknown policy fields fail closed.
//!
//! Honest scope: Drives policy.new and validate_rule directly. Successful construction must
//! not silently discard unsupported policy or rule keys.
//! Fixtures use the installed diver-fixed modules, never mocks or live config edits.

use super::task_209::run_probes;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-214";
/// Desired permission invariant.
pub const NAME: &str = "unknown policy fields fail closed";
/// Real policy and approval modules in fixed-config headless Neovim.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "empty_policy_denies",
    "known_rule_accepted",
    "unknown_top_level_rejected",
    "unknown_rule_field_rejected",
];

const PROBES: [&str; 4] = [
    r#"
return fixture.p.decide(fixture.p.new({}), fixture.req()).decision == "deny"
"#,
    r#"
local result = fixture.policy("allow")
return result ~= nil and fixture.p.decide(result, fixture.req()).decision == "allow"
"#,
    r#"
return fixture.p.new({ default = "allow", unknown_permission = true }) == nil
"#,
    r#"
return fixture.p.new({ rules = { { risk = "local_reversible", decision = "allow", typo_paths = { "/work/a" } } } })
	== nil
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
    fn empty_policy_denies() {
        let result = probe(Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn known_rule_accepted() {
        let result = probe(Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn unknown_top_level_rejected() {
        let result = probe(Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn unknown_rule_field_rejected() {
        let result = probe(Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
