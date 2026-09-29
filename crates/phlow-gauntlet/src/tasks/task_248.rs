//! task-248: append only audit ownership.
//!
//! Honest scope: The real events sink is used with real tool.started/completed events. Mutation
//! attacks target aliases returned by append and events; no fake ledger is involved.
//! Fixtures use installed diver-fixed modules without changing either Neovim config.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-248";
/// Desired invariant.
pub const NAME: &str = "append only audit ownership";
/// Runs the actual fixed-config Neovim modules.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "ordered_events",
    "envelope_has_time_source",
    "append_input_cannot_rewrite",
    "reader_cannot_erase_scope",
];

const PROBES: [&str; 4] = [
    r#"
local s = events.new_sink()
assert(s:append("run", "tool.started", { tool = "check" }))
assert(s:append("run", "tool.completed", { tool = "check" }))
local rows = s:events("run")
return #rows == 2 and rows[1].seq < rows[2].seq
"#,
    r#"
local s = events.new_sink()
local e = assert(s:append("run", "tool.started", { tool = "check" }))
return e.run_id == "run" and e.ts_ns > 0 and e.source == "harness"
"#,
    r#"
local s = events.new_sink()
local payload = { tool = "check" }
assert(s:append("run", "tool.started", payload))
payload.tool = "other"
return s:events()[1].payload.tool == "check"
"#,
    r#"
local s = events.new_sink()
assert(s:append("run", "tool.started", { paths = { "/work/a" } }))
s:events()[1].payload.paths[1] = "/outside"
return s:events()[1].payload.paths[1] == "/work/a"
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
    fn ordered_events() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn envelope_has_time_source() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn append_input_cannot_rewrite() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn reader_cannot_erase_scope() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
