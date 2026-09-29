//! task-216: policy version pinning.
//!
//! Honest scope: Policy.new currently has no version contract. Version 1 is the desired
//! initial schema, not an existing supported API; acceptance must preserve it and reject
//! missing or unknown versions.
//! Fixtures use the installed diver-fixed modules, never mocks or live config edits.

use super::task_209::run_probes;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-216";
/// Desired permission invariant.
pub const NAME: &str = "policy version pinning";
/// Real policy and approval modules in fixed-config headless Neovim.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "version_one_accepted",
    "version_one_retained",
    "future_version_rejected",
    "missing_version_rejected",
];

const PROBES: [&str; 4] = [
    r#"
local result = fixture.p.new({ version = 1, default = "deny" })
return result ~= nil
"#,
    r#"
local result = fixture.p.new({ version = 1, default = "deny" })
return result ~= nil and result.version == 1
"#,
    r#"
return fixture.p.new({ version = 999, default = "allow" }) == nil
"#,
    r#"
return fixture.p.new({ default = "allow" }) == nil
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
    fn version_one_accepted() {
        let result = probe(Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn version_one_retained() {
        let result = probe(Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn future_version_rejected() {
        let result = probe(Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn missing_version_rejected() {
        let result = probe(Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
