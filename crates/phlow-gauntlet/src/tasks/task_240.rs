//! task-240: unapproved proposals grant nothing.
//!
//! Honest scope: Actual approval records are adversarially modified before asking policy for a
//! decision. The state must stay pending and policy must stay closed without a human decision.
//! Fixtures use installed diver-fixed modules without changing either Neovim config.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-240";
/// Desired invariant.
pub const NAME: &str = "unapproved proposals grant nothing";
/// Runs the actual fixed-config Neovim modules.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "pending_is_inert",
    "denied_is_inert",
    "forged_request_approval_ignored",
    "read_handle_cannot_approve",
];

const PROBES: [&str; 4] = [
    r#"
local q, id = pending()
return a.get(q, id).state == "pending" and p.decide(policy("approval"), req()).decision == "approval"
"#,
    r#"
local q, id = pending()
assert(a.decide(q, id, "denied", "operator"))
return a.get(q, id).state == "denied" and p.decide(policy("deny"), req()).decision == "deny"
"#,
    r#"
local r = req()
r.approved = true
r.state = "approved"
local q, id = pending(r)
return a.get(q, id).state == "pending" and p.decide(policy("approval"), r).decision == "approval"
"#,
    r#"
local q, id = pending()
local record = a.get(q, id)
local ok, err = pcall(function()
	record.state = "approved"
end)
return (ok or type(err) == "string") and a.get(q, id).state == "pending"
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
    fn pending_is_inert() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn denied_is_inert() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn forged_request_approval_ignored() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn read_handle_cannot_approve() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
