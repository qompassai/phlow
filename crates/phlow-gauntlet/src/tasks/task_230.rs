//! task-230: bounded permission delta.
//!
//! Honest scope: The queue admission seam must support 4096 unique permissions within one second
//! and reject malformed sets. Outer Neovim execution has a 15-second deadline.
//! Fixtures use installed diver-fixed modules without changing either Neovim config.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-230";
/// Desired invariant.
pub const NAME: &str = "bounded permission delta";
/// Runs the actual fixed-config Neovim modules.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "single_permission",
    "large_valid_set",
    "non_string_member_rejected",
    "sparse_set_rejected",
];

const PROBES: [&str; 4] = [
    r#"
local request = req()
request.permissions_before = {}
request.permissions_after = { "x" }

local queue = a.new()
local id, err = a.request(queue, "delta-run", request)
assert(id, err)
local record = a.get(queue, id)
return vim.deep_equal(record.permission_delta, { added = { "x" }, removed = {} })
"#,
    r#"
local request = req()
request.permissions_before = {}
request.permissions_after = {}
local PERMISSIONS_MAX = 4096
for index = 1, PERMISSIONS_MAX do
	request.permissions_after[index] = string.format("permission-%04d", index)
end
local q = a.new()
local started = vim.uv.hrtime()
local id, err = a.request(q, "run", request)
assert(id, err)
local d = a.get(q, id).permission_delta
return type(d) == "table"
	and #d.added == PERMISSIONS_MAX
	and #d.removed == 0
	and vim.deep_equal(d.added, request.permissions_after)
	and vim.uv.hrtime() - started < 1000000000
"#,
    r#"
local r = req()
r.permissions_before = {}
r.permissions_after = { false }
local id, err = a.request(a.new(), "run", r)
return id == nil and type(err) == "string"
"#,
    r#"
local r = req()
r.permissions_before = {}
r.permissions_after = { [2] = "root" }
local id, err = a.request(a.new(), "run", r)
return id == nil and type(err) == "string"
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
    fn single_permission() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn large_valid_set() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn non_string_member_rejected() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn sparse_set_rejected() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
