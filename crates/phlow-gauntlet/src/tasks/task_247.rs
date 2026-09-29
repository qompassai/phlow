//! task-247: denial audit reconstruction.
//!
//! Honest scope: Policy.decide is the actual denial output seam. It must retain structured tool and
//! scope for ledger ingestion, not merely a prose reason; persistence is not covered.
//! Fixtures use installed diver-fixed modules without changing either Neovim config.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-247";
/// Desired invariant.
pub const NAME: &str = "denial audit reconstruction";
/// Runs the actual fixed-config Neovim modules.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "denial_has_reason",
    "denial_has_risk",
    "denial_has_tool",
    "denial_has_scope",
];

const PROBES: [&str; 4] = [
    r#"
local d = p.decide(nil, req())
return d.decision == "deny" and type(d.reason) == "string" and #d.reason > 0
"#,
    r#"
return p.decide(nil, req()).risk == "local_reversible"
"#,
    r#"
return p.decide(nil, req()).tool == "fs.write"
"#,
    r#"
return vim.deep_equal(p.decide(nil, req()).paths, { "/work/a" })
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
    fn denial_has_reason() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn denial_has_risk() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn denial_has_tool() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn denial_has_scope() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
