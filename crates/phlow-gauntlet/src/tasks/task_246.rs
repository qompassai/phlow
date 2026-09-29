//! task-246: approval audit completeness.
//!
//! Honest scope: The real queue record is the closest approval ledger; it must retain actor,
//! monotonic decision time and immutable scope. No persistent ledger is fabricated.
//! Fixtures use installed diver-fixed modules without changing either Neovim config.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-246";
/// Desired invariant.
pub const NAME: &str = "approval audit completeness";
/// Runs the actual fixed-config Neovim modules.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "request_has_identity_time",
    "decision_retains_actor",
    "decision_has_timestamp",
    "scope_cannot_be_rewritten",
];

const PROBES: [&str; 4] = [
    r#"
local q, id = pending()
local r = a.get(q, id)
return r.id == id and r.run_id == "run" and type(r.created_ns) == "number" and r.created_ns > 0
"#,
    r#"
local q, id = pending()
assert(a.decide(q, id, "approved", "operator"))
return a.get(q, id).decided_by == "operator"
"#,
    r#"
local q, id = pending()
assert(a.decide(q, id, "denied", "operator"))
local r = a.get(q, id)
return type(r.decided_ns) == "number" and r.decided_ns >= r.created_ns
"#,
    r#"
local request = req()
local q, id = pending(request)
request.paths[1] = "/work/other"
return vim.deep_equal(a.get(q, id).paths, { "/work/a" })
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
    fn request_has_identity_time() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn decision_retains_actor() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn decision_has_timestamp() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn scope_cannot_be_rewritten() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
