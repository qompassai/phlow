//! task-241: approval actor hardness.
//!
//! Honest scope: Approval.decide is the real human-decision boundary. Missing or model actors must
//! not commit a grant. Actor authentication beyond this local API is not claimed.
//! Fixtures use installed diver-fixed modules without changing either Neovim config.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-241";
/// Desired invariant.
pub const NAME: &str = "approval actor hardness";
/// Runs the actual fixed-config Neovim modules.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "human_approval",
    "human_denial",
    "missing_actor_refused",
    "model_actor_refused",
];

const PROBES: [&str; 4] = [
    r#"
local q, id = pending()
local ok = a.decide(q, id, "approved", "operator")
return ok == true and a.get(q, id).state == "approved"
"#,
    r#"
local q, id = pending()
local ok = a.decide(q, id, "denied", "operator")
return ok == true and a.get(q, id).state == "denied"
"#,
    r#"
local q, id = pending()
local ok = a.decide(q, id, "approved")
return ok ~= true and a.get(q, id).state == "pending"
"#,
    r#"
local q, id = pending()
local ok = a.decide(q, id, "approved", "agent")
return ok ~= true and a.get(q, id).state == "pending"
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
    fn human_approval() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn human_denial() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn missing_actor_refused() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn model_actor_refused() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
