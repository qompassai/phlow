//! task-249: audit semantic completeness.
//!
//! Honest scope: Events.make_envelope is the real ledger admission seam. Tool execution records
//! must reject missing identity/scope or timestamps; this tests validation, not automatic emission.
//! Fixtures use installed diver-fixed modules without changing either Neovim config.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-249";
/// Desired invariant.
pub const NAME: &str = "audit semantic completeness";
/// Runs the actual fixed-config Neovim modules.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "valid_spawn_envelope",
    "valid_completion_envelope",
    "empty_spawn_rejected",
    "invalid_time_rejected",
];

const PROBES: [&str; 4] = [
    r#"
local e = events.make_envelope("run", "tool.started", { tool = "check", actor = "operator", paths = { "/work/a" } })
return type(e) == "table" and e.kind == "tool.started"
"#,
    r#"
local e = events.make_envelope(
	"run",
	"tool.completed",
	{ tool = "check", actor = "operator", paths = { "/work/a" }, status = "denied" }
)
return type(e) == "table" and e.payload.status == "denied"
"#,
    r#"
local e, err = events.make_envelope("run", "tool.started", {})
return e == nil and type(err) == "string"
"#,
    r#"
local e, err = events.make_envelope("run", "tool.completed", { tool = "check" }, { ts_ns = -1 })
return e == nil and type(err) == "string"
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
    fn valid_spawn_envelope() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn valid_completion_envelope() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn empty_spawn_rejected() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn invalid_time_rejected() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
