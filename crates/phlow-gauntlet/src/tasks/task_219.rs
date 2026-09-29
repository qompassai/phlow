//! task-219: append only approval history.
//!
//! Honest scope: Approval.get is the closest ledger read seam. History must survive attempted
//! edits/deletion through returned records. No durable ledger exists here; disk persistence
//! is not tested.
//! Fixtures use the installed diver-fixed modules, never mocks or live config edits.

use super::task_209::run_probes;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-219";
/// Desired permission invariant.
pub const NAME: &str = "append only approval history";
/// Real policy and approval modules in fixed-config headless Neovim.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "decision_remains_readable",
    "second_decision_cannot_overwrite",
    "reader_cannot_edit_history",
    "reader_cannot_delete_attribution",
];

const PROBES: [&str; 4] = [
    r#"
local q, id = fixture.pending()
fixture.a.decide(q, id, "approved", "human")
return fixture.a.get(q, id).state == "approved"
"#,
    r#"
local q, id = fixture.pending()
fixture.a.decide(q, id, "approved", "human")
local ok = fixture.a.decide(q, id, "denied", "human")
return not ok and fixture.a.get(q, id).state == "approved"
"#,
    r#"
local q, id = fixture.pending()
fixture.a.decide(q, id, "approved", "human")
local record = fixture.a.get(q, id)
record.state = "pending"
return fixture.a.get(q, id).state == "approved"
"#,
    r#"
local q, id = fixture.pending()
fixture.a.decide(q, id, "approved", "human")
local record = fixture.a.get(q, id)
record.decided_by = nil
return fixture.a.get(q, id).decided_by == "human"
"#,
];

/// Run four bounded real-seam probes; retain all case outcomes, including failures.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_probes(ctx, ID, &CASES, &PROBES)
}

#[cfg(test)]
mod tests {
    use super::{CASES, PROBES};
    use crate::tasks::task_209::probe;
    use std::path::Path;

    #[test]
    fn decision_remains_readable() {
        let result = probe(Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn second_decision_cannot_overwrite() {
        let result = probe(Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn reader_cannot_edit_history() {
        let result = probe(Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn reader_cannot_delete_attribution() {
        let result = probe(Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
