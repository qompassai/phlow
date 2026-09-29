//! task-213: approval scope immutability.
//!
//! Honest scope: The queue stores the proposal scope. Mutating caller-owned inputs must not
//! broaden a pending or approved grant; no action executor integration is claimed.
//! Fixtures use the installed diver-fixed modules, never mocks or live config edits.

use super::task_209::run_probes;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-213";
/// Desired permission invariant.
pub const NAME: &str = "approval scope immutability";
/// Real policy and approval modules in fixed-config headless Neovim.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "approved_path_preserved",
    "approved_tool_preserved",
    "pending_scope_cannot_expand",
    "approved_scope_cannot_expand",
];

const PROBES: [&str; 4] = [
    r#"
local q, id = fixture.pending()
fixture.a.decide(q, id, "approved", "human")
return vim.deep_equal(fixture.a.get(q, id).paths, { "/work/a" })
"#,
    r#"
local q, id = fixture.pending()
fixture.a.decide(q, id, "approved", "human")
return fixture.a.get(q, id).tool == "fs.write"
"#,
    r#"
local r = fixture.req()
local q, id = fixture.pending(r)
r.paths[2] = "/work/b"
return vim.deep_equal(fixture.a.get(q, id).paths, { "/work/a" })
"#,
    r#"
local r = fixture.req()
local q, id = fixture.pending(r)
fixture.a.decide(q, id, "approved", "human")
r.paths[1] = "/work"
return vim.deep_equal(fixture.a.get(q, id).paths, { "/work/a" })
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
    fn approved_path_preserved() {
        let result = probe(Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn approved_tool_preserved() {
        let result = probe(Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn pending_scope_cannot_expand() {
        let result = probe(Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn approved_scope_cannot_expand() {
        let result = probe(Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
