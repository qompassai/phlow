//! task-250: spawn audit reconstruction.
//!
//! Honest scope: Runtime.check is the actual spawn report seam. Reports already carry argv,
//! workspace and status; desired ledger extensions must add actor and absolute time plus pinned-
//! versus-resolved identities for substitution decisions. This does not claim durable ledger
//! emission.

use super::task_233::{Fixture, ok};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-250";
/// Desired invariant.
pub const NAME: &str = "spawn audit reconstruction";
/// Executes real Rust runtime seams.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "spawn_retains_argv",
    "denial_retains_scope",
    "spawn_has_actor_and_absolute_time",
    "substitution_has_both_identities",
];

/// Execute all cases, retaining assertion failures separately from fixture errors.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, CASES.iter().enumerate().map(|(i, c)| (*c, case(i))))
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => {
            let f = Fixture::new()?;
            let mut r = f.runtime(&["/usr/bin/true"], 1000, true)?;
            let v = r.check(Some("probe"));
            Ok(v["checks"][0]["cmd"] == serde_json::json!(["/usr/bin/true"]))
        }
        1 => {
            let f = Fixture::new()?;
            let mut r = f.runtime_trusted(&["/usr/bin/true"], 1000, true, false)?;
            let v = r.check(Some("probe"));
            Ok(!ok(&v) && v["checks"][0]["workspace"].as_str() == f.path.to_str())
        }
        2 => {
            let f = Fixture::new()?;
            let mut r = f.runtime(&["/usr/bin/true"], 1000, true)?;
            let v = r.check(Some("probe"));
            Ok(v["checks"][0]["actor"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
                && v["checks"][0]["started_at"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty()))
        }
        3 => {
            let f = Fixture::new()?;
            f.executable(
                "tool",
                "#!/bin/sh
exit 0
",
            )?;
            let mut r = f.runtime(&["./tool"], 1000, true)?;
            f.executable(
                "tool",
                "#!/bin/sh
exit 1
",
            )?;
            let v = r.check(Some("probe"));
            let c = &v["checks"][0];
            Ok(c["status"] == "error"
                && c["pinned_sha256"].is_string()
                && c["observed_sha256"].is_string()
                && c["pinned_sha256"] != c["observed_sha256"])
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn spawn_retains_argv() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn denial_retains_scope() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn spawn_has_actor_and_absolute_time() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn substitution_has_both_identities() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
