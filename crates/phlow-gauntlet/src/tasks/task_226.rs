//! task-226: permission delta set edges.
//!
//! Honest scope: The real approval queue must calculate set differences, not compare cardinality or
//! summaries. No test-local delta implementation is used.
//! Fixtures use installed diver-fixed modules without changing either Neovim config.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-226";
/// Desired invariant.
pub const NAME: &str = "permission delta set edges";
/// Runs the actual fixed-config Neovim modules.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "add_only",
    "remove_only",
    "rename_remove_add",
    "equal_size_disjoint",
];

const PROBES: [&str; 4] = [
    r#"
local request = req()
request.permissions_before = { "a" }
request.permissions_after = { "a", "b" }

local queue = a.new()
local id, err = a.request(queue, "delta-run", request)
assert(id, err)
local record = a.get(queue, id)
return vim.deep_equal(record.permission_delta, { added = { "b" }, removed = {} })
"#,
    r#"
local request = req()
request.permissions_before = { "a", "b" }
request.permissions_after = { "b" }

local queue = a.new()
local id, err = a.request(queue, "delta-run", request)
assert(id, err)
local record = a.get(queue, id)
return vim.deep_equal(record.permission_delta, { added = {}, removed = { "a" } })
"#,
    r#"
local request = req()
request.permissions_before = { "old" }
request.permissions_after = { "new" }

local queue = a.new()
local id, err = a.request(queue, "delta-run", request)
assert(id, err)
local record = a.get(queue, id)
return vim.deep_equal(record.permission_delta, { added = { "new" }, removed = { "old" } })
"#,
    r#"
local request = req()
request.permissions_before = { "a", "b" }
request.permissions_after = { "c", "d" }

local queue = a.new()
local id, err = a.request(queue, "delta-run", request)
assert(id, err)
local record = a.get(queue, id)
return vim.deep_equal(record.permission_delta, { added = { "c", "d" }, removed = { "a", "b" } })
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
    fn add_only() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn remove_only() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn rename_remove_add() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn equal_size_disjoint() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
