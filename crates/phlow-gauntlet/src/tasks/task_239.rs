//! task-239: policy state isolation.
//!
//! Honest scope: The real policy constructor and decision engine are the closest enforcer seam.
//! Caller-owned tables and direct state writes must not widen policy; this does not claim OS
//! isolation.
//! Fixtures use installed diver-fixed modules without changing either Neovim config.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-239";
/// Desired invariant.
pub const NAME: &str = "policy state isolation";
/// Runs the actual fixed-config Neovim modules.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "deny_default",
    "explicit_allow",
    "input_alias_cannot_grant",
    "agent_write_refused",
];

const PROBES: [&str; 4] = [
    r#"
return p.decide(p.new({}), req()).decision == "deny"
"#,
    r#"
return p.decide(policy("allow"), req()).decision == "allow"
"#,
    r#"
local rule = rule_for("deny")
local state = assert(p.new({ rules = { rule } }))
rule.decision = "allow"
return p.decide(state, req()).decision == "deny"
"#,
    r#"
local state = policy("deny")
local ok = pcall(function()
	state.rules[1].decision = "allow"
end)
return not ok and p.decide(state, req()).decision == "deny"
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
    fn deny_default() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn explicit_allow() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn input_alias_cannot_grant() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn agent_write_refused() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
