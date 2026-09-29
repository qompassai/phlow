//! task-210: minimal permission proposals.
//!
//! Honest scope: Approval.request is the proposal admission seam. No denial-bound proposal
//! API exists; these probes require admission to reject unsupported extra authority instead
//! of dropping it.
//! Fixtures use the installed diver-fixed modules, never mocks or live config edits.

use super::task_209::run_probes;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-210";
/// Desired permission invariant.
pub const NAME: &str = "minimal permission proposals";
/// Real policy and approval modules in fixed-config headless Neovim.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "single_path_retained",
    "single_tool_retained",
    "extra_permissions_rejected",
    "extra_tools_rejected",
];

const PROBES: [&str; 4] = [
    r#"
local q, id = fixture.pending()
return vim.deep_equal(fixture.a.get(q, id).paths, { "/work/a" })
"#,
    r#"
local q, id = fixture.pending()
return fixture.a.get(q, id).tool == "fs.write"
"#,
    r#"
local r = fixture.req()
r.permissions = { "fs.write", "process.spawn" }
local id = fixture.a.request(fixture.a.new(), "run", r)
return id == nil
"#,
    r#"
local r = fixture.req()
r.tools = { "fs.write", "fs.delete" }
local id = fixture.a.request(fixture.a.new(), "run", r)
return id == nil
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
    fn single_path_retained() {
        let result = probe(Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn single_tool_retained() {
        let result = probe(Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn extra_permissions_rejected() {
        let result = probe(Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn extra_tools_rejected() {
        let result = probe(Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
