//! task-223: policy load failure denies all.
//!
//! Honest scope: Drives construction failure into policy.decide without inventing a loader
//! fallback. False and sparse malformed configurations must not normalize into permissive
//! policies.
//! Fixtures use the installed diver-fixed modules, never mocks or live config edits.

use super::task_209::run_probes;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-223";
/// Desired permission invariant.
pub const NAME: &str = "policy load failure denies all";
/// Real policy and approval modules in fixed-config headless Neovim.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "missing_policy_denies",
    "invalid_default_denies",
    "false_rules_fail_closed",
    "map_rules_fail_closed",
];

const PROBES: [&str; 4] = [
    r#"
return fixture.p.decide(nil, fixture.req()).decision == "deny"
"#,
    r#"
local result, err = fixture.p.new({ default = "invalid" })
return result == nil and err ~= nil and fixture.p.decide(result, fixture.req()).decision == "deny"
"#,
    r#"
local result = fixture.p.new({ default = "allow", rules = false })
return result == nil and fixture.p.decide(result, fixture.req()).decision == "deny"
"#,
    r#"
local result = fixture.p.new({ default = "allow", rules = { hidden = fixture.rule("deny") } })
return result == nil and fixture.p.decide(result, fixture.req()).decision == "deny"
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
    fn missing_policy_denies() {
        let result = probe(Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn invalid_default_denies() {
        let result = probe(Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn false_rules_fail_closed() {
        let result = probe(Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn map_rules_fail_closed() {
        let result = probe(Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
