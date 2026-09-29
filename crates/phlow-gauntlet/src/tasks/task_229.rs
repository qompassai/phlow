//! task-229: permission delta snapshots.
//!
//! Honest scope: Approval records must own their before/after inputs and return immutable snapshots
//! of the computed delta. Calls drive Approval.request/get directly.
//! Fixtures use installed diver-fixed modules without changing either Neovim config.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-229";
/// Desired invariant.
pub const NAME: &str = "permission delta snapshots";
/// Runs the actual fixed-config Neovim modules.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "input_retained",
    "two_independent_requests",
    "caller_mutation_isolated",
    "reader_mutation_isolated",
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
local function compute(before, after, summary)
	local request = req()
	request.permissions_before = before
	request.permissions_after = after
	request.summary = summary
	local queue = a.new()
	local id, err = a.request(queue, "run", request)
	assert(id, err)
	return a.get(queue, id).permission_delta
end
local x = compute({}, { "a" })
local y = compute({}, { "b" })
return vim.deep_equal(x, { added = { "a" }, removed = {} }) and vim.deep_equal(y, { added = { "b" }, removed = {} })
"#,
    r#"
local request = req()
request.permissions_before = {}
request.permissions_after = { "a" }
local q = a.new()
local id, err = a.request(q, "run", request)
assert(id, err)
request.permissions_after[1] = "root"
return vim.deep_equal(a.get(q, id).permission_delta, { added = { "a" }, removed = {} })
"#,
    r#"
local request = req()
request.permissions_before = {}
request.permissions_after = { "a" }
local q = a.new()
local id, err = a.request(q, "run", request)
assert(id, err)
local record = a.get(q, id)
if type(record.permission_delta) ~= "table" then
	return false
end
record.permission_delta.added[1] = "root"
return vim.deep_equal(a.get(q, id).permission_delta, { added = { "a" }, removed = {} })
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
    fn input_retained() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn two_independent_requests() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn caller_mutation_isolated() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn reader_mutation_isolated() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
