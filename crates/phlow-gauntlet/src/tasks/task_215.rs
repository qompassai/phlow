//! task-215: no permissive policy parsing.
//!
//! Honest scope: No parser mode is currently declared. Proposed bypass flags must be rejected
//! as unsupported rather than ignored; malformed scope elements must also fail admission.
//! Fixtures use the installed diver-fixed modules, never mocks or live config edits.

use super::task_209::run_probes;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-215";
/// Desired permission invariant.
pub const NAME: &str = "no permissive policy parsing";
/// Real policy and approval modules in fixed-config headless Neovim.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "known_default_accepted",
    "known_scope_accepted",
    "legacy_bypass_flag_rejected",
    "permissive_malformed_scope_rejected",
];

const PROBES: [&str; 4] = [
    r#"
return fixture.p.new({ default = "deny" }) ~= nil
"#,
    r#"
return fixture.policy("allow") ~= nil
"#,
    r#"
return fixture.p.new({ legacy = true, default = "allow" }) == nil
"#,
    r#"
return fixture.p.new({
	permissive = true,
	rules = { { risk = "local_reversible", decision = "allow", paths = { 42 } } },
}) == nil
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
    fn known_default_accepted() {
        let result = probe(Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn known_scope_accepted() {
        let result = probe(Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn legacy_bypass_flag_rejected() {
        let result = probe(Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn permissive_malformed_scope_rejected() {
        let result = probe(Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
