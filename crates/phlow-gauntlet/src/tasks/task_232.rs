//! task-232: approval surface delta fidelity.
//!
//! Honest scope: Approval.pending is the actual approval surface data source. It must expose the
//! same computed difference as get, including revocations hidden by a malicious summary.
//! Fixtures use installed diver-fixed modules without changing either Neovim config.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-232";
/// Desired invariant.
pub const NAME: &str = "approval surface delta fidelity";
/// Runs the actual fixed-config Neovim modules.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "pending_exposes_delta",
    "get_and_pending_agree",
    "summary_cannot_hide_revoke",
    "empty_claim_cannot_hide_rename",
];

const PROBES: [&str; 4] = [
    r#"
local r = req()
r.permissions_before = {}
r.permissions_after = { "a" }
local q = a.new()
local id, err = a.request(q, "run", r)
assert(id, err)
local rows = a.pending(q)
return #rows == 1 and vim.deep_equal(rows[1].permission_delta, { added = { "a" }, removed = {} })
"#,
    r#"
local r = req()
r.permissions_before = { "a" }
r.permissions_after = {}
local q = a.new()
local id, err = a.request(q, "run", r)
assert(id, err)
local d = a.get(q, id).permission_delta
return type(d) == "table" and vim.deep_equal(d, a.pending(q)[1].permission_delta)
"#,
    r#"
local request = req()
request.permissions_before = { "admin" }
request.permissions_after = {}
request.summary = "Read-only operation, no change"
local queue = a.new()
local id, err = a.request(queue, "delta-run", request)
assert(id, err)
local record = a.get(queue, id)
return vim.deep_equal(record.permission_delta, { added = {}, removed = { "admin" } })
"#,
    r#"
local request = req()
request.permissions_before = { "a" }
request.permissions_after = { "b" }
request.permission_delta = { added = {}, removed = {} }
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
    fn pending_exposes_delta() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn get_and_pending_agree() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn summary_cannot_hide_revoke() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn empty_claim_cannot_hide_rename() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
