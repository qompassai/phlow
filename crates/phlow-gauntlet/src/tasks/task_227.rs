//! task-227: byte deterministic permission delta.
//!
//! Honest scope: Only the real queue output is serialized; timestamps and random approval IDs are
//! excluded from the delta contract.
//! Fixtures use installed diver-fixed modules without changing either Neovim config.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-227";
/// Desired invariant.
pub const NAME: &str = "byte deterministic permission delta";
/// Runs the actual fixed-config Neovim modules.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "identical_inputs_identical_bytes",
    "sorted_additions",
    "permutation_invariant",
    "duplicates_are_set_members",
];

const PROBES: [&str; 4] = [
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
local x = compute({ "a" }, { "b" })
local y = compute({ "a" }, { "b" })
return type(x) == "table" and vim.json.encode(x) == vim.json.encode(y)
"#,
    r#"
local request = req()
request.permissions_before = {}
request.permissions_after = { "z", "a" }

local queue = a.new()
local id, err = a.request(queue, "delta-run", request)
assert(id, err)
local record = a.get(queue, id)
return vim.deep_equal(record.permission_delta, { added = { "a", "z" }, removed = {} })
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
local x = compute({}, { "b", "a" })
local y = compute({}, { "a", "b" })
return type(x) == "table" and vim.json.encode(x) == vim.json.encode(y)
"#,
    r#"
local request = req()
request.permissions_before = { "a", "a" }
request.permissions_after = { "b", "b" }

local queue = a.new()
local id, err = a.request(queue, "delta-run", request)
assert(id, err)
local record = a.get(queue, id)
return vim.deep_equal(record.permission_delta, { added = { "b" }, removed = { "a" } })
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
    fn identical_inputs_identical_bytes() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn sorted_additions() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn permutation_invariant() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn duplicates_are_set_members() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
