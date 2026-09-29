//! task-220: proposal expiry enforced at decision.
//!
//! Honest scope: Uses real monotonic time and sweep_expired. Decide must enforce TTL even
//! without a sweep; the wait is bounded to 10 ms, with a 1 ms proposal TTL.
//! Fixtures use the installed diver-fixed modules, never mocks or live config edits.

use super::task_209::run_probes;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-220";
/// Desired permission invariant.
pub const NAME: &str = "proposal expiry enforced at decision";
/// Real policy and approval modules in fixed-config headless Neovim.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "before_deadline_stays_pending",
    "deadline_sweep_expires",
    "swept_expired_cannot_approve",
    "unswept_expired_cannot_approve",
];

const PROBES: [&str; 4] = [
    r#"
local q, id = fixture.pending()
local count = fixture.a.sweep_expired(q, fixture.a.get(q, id).created_ns)
return count == 0 and fixture.a.get(q, id).state == "pending"
"#,
    r#"
local q, id = fixture.pending()
local count = fixture.a.sweep_expired(q, fixture.a.get(q, id).deadline_ns)
return count == 1 and fixture.a.get(q, id).state == "expired"
"#,
    r#"
local q, id = fixture.pending()
fixture.a.sweep_expired(q, fixture.a.get(q, id).deadline_ns)
return not fixture.a.decide(q, id, "approved", "human")
"#,
    r#"
local q = fixture.a.new()
local id = fixture.a.request(q, "run", fixture.req(), { timeout_ms = 1 })
assert(id ~= nil)
vim.wait(10)
local ok = fixture.a.decide(q, id, "approved", "human")
return not ok and fixture.a.get(q, id).state ~= "approved"
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
    fn before_deadline_stays_pending() {
        let result = probe(Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn deadline_sweep_expires() {
        let result = probe(Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn swept_expired_cannot_approve() {
        let result = probe(Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn unswept_expired_cannot_approve() {
        let result = probe(Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}
