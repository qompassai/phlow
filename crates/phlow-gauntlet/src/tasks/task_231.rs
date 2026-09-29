//! task-231: permission identity preservation.
//!
//! Honest scope: The queue must retain exact permission identity: case changes and scope changes
//! are revocation plus grant, never an unchanged permission.
//! Fixtures use installed diver-fixed modules without changing either Neovim config.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-231";
/// Desired invariant.
pub const NAME: &str = "permission identity preservation";
/// Runs the actual fixed-config Neovim modules.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "exact_scope_unchanged",
    "independent_scopes",
    "case_change_is_change",
    "scope_widening_visible",
];

const PROBES: [&str; 4] = [
    r#"
local request = req()
request.permissions_before = { "fs.read:/work/a" }
request.permissions_after = { "fs.read:/work/a" }

local queue = a.new()
local id, err = a.request(queue, "delta-run", request)
assert(id, err)
local record = a.get(queue, id)
return vim.deep_equal(record.permission_delta, { added = {}, removed = {} })
"#,
    r#"
local request = req()
request.permissions_before = {}
request.permissions_after = { "fs.read:/work/a", "fs.read:/work/b" }

local queue = a.new()
local id, err = a.request(queue, "delta-run", request)
assert(id, err)
local record = a.get(queue, id)
return vim.deep_equal(record.permission_delta, { added = { "fs.read:/work/a", "fs.read:/work/b" }, removed = {} })
"#,
    r#"
local request = req()
request.permissions_before = { "read" }
request.permissions_after = { "READ" }

local queue = a.new()
local id, err = a.request(queue, "delta-run", request)
assert(id, err)
local record = a.get(queue, id)
return vim.deep_equal(record.permission_delta, { added = { "READ" }, removed = { "read" } })
"#,
    r#"
local request = req()
request.permissions_before = { "fs.read:/work/a" }
request.permissions_after = { "fs.read:/work" }

local queue = a.new()
local id, err = a.request(queue, "delta-run", request)
assert(id, err)
local record = a.get(queue, id)
return vim.deep_equal(record.permission_delta, { added = { "fs.read:/work" }, removed = { "fs.read:/work/a" } })
"#,
];

/// Run all four cases with bounded subprocess execution and per-case evidence.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    super::task_225::run_probes(ctx, ID, &CASES, &PROBES)
}

#[cfg(test)]
mod tests {
    use super::{CASES, PROBES};

    #[test]
    fn exact_scope_unchanged() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn independent_scopes() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn case_change_is_change() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn scope_widening_visible() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
