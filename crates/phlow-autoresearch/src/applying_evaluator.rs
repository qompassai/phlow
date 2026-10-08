//! The applying evaluator: composition of the loop's apply step
//! with evidence evaluation.
//!
//! [`crate::research_loop::run_loop`] validates a change-set, then
//! hands it to an [`Evaluator`]. Until this module, no evaluator
//! applied anything — the scripted evaluator replays outcomes and
//! [`crate::receipt_evaluator::ReceiptEvaluator`] verifies evidence
//! files. The live path composes the two real steps in order:
//!
//! 1. [`crate::applier::apply_change_set`] applies the change-set
//!    inside the experiment worktree (fail closed, nothing written
//!    on error);
//! 2. the inner evaluator measures the result.
//!
//! An apply failure is an [`EvalError`] (`evaluation_failed`), so
//! the loop records the iteration as a crash with the apply detail
//! in the ledger — it can never become a score. Gate violations are
//! still caught earlier, by the loop's own validation pass, before
//! this evaluator is invoked; the applier's re-validation here is
//! defense in depth, and its class is preserved in the message.

use std::path::PathBuf;

use crate::applier::apply_change_set;
use crate::changeset::ChangeSet;
use crate::clock::Clock;
use crate::evaluator::{EvalError, Evaluation, Evaluator};

/// An [`Evaluator`] that applies each change-set in the experiment
/// worktree before delegating measurement to an inner evaluator.
pub struct ApplyingEvaluator {
    worktree_root: PathBuf,
    ledger_dir: PathBuf,
    inner: Box<dyn Evaluator>,
}

impl std::fmt::Debug for ApplyingEvaluator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The inner evaluator is a trait object with no Debug bound;
        // name its presence, not its state.
        f.debug_struct("ApplyingEvaluator")
            .field("worktree_root", &self.worktree_root)
            .field("ledger_dir", &self.ledger_dir)
            .field("inner", &"<dyn Evaluator>")
            .finish()
    }
}

impl ApplyingEvaluator {
    /// Compose `inner` behind the applier for `worktree_root`.
    /// `ledger_dir` is passed to containment validation so a
    /// change-set can never write into the loop's own ledger.
    #[must_use]
    pub fn new(worktree_root: PathBuf, ledger_dir: PathBuf, inner: Box<dyn Evaluator>) -> Self {
        ApplyingEvaluator {
            worktree_root,
            ledger_dir,
            inner,
        }
    }
}

impl Evaluator for ApplyingEvaluator {
    fn evaluate(
        &mut self,
        change_set: &ChangeSet,
        clock: &dyn Clock,
    ) -> Result<Evaluation, EvalError> {
        apply_change_set(change_set, &self.worktree_root, &self.ledger_dir).map_err(|err| {
            EvalError::failed(format!(
                "apply failed [{}]: {}",
                err.class.name(),
                err.message
            ))
        })?;
        self.inner.evaluate(change_set, clock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::ChangeKind;
    use crate::clock::ManualClock;
    use crate::evaluator::{ScriptedEvaluator, ScriptedOutcome};
    use crate::testsupport::{TestDir, test_dir};

    fn setup() -> (TestDir, PathBuf, PathBuf) {
        let root = test_dir("applying");
        let worktree = root.path().join("worktree");
        let ledger = root.path().join("ledger");
        std::fs::create_dir_all(worktree.join("src")).expect("mkdir worktree");
        std::fs::create_dir_all(&ledger).expect("mkdir ledger");
        std::fs::write(worktree.join("src/lib.rs"), "alpha\nbeta\n").expect("write lib");
        (root, worktree, ledger)
    }

    fn patch_cs(payload: &str) -> ChangeSet {
        ChangeSet {
            id: "p".to_string(),
            kind: ChangeKind::FilePatch,
            paths: vec!["src/lib.rs".to_string()],
            payload: payload.to_string(),
            rationale: "applying evaluator test".to_string(),
        }
    }

    const GOOD_DIFF: &str = "--- a/src/lib.rs\n+++ b/src/lib.rs\n\
        @@ -1,2 +1,2 @@\n alpha\n-beta\n+BETA\n";
    const BAD_DIFF: &str = "--- a/src/lib.rs\n+++ b/src/lib.rs\n\
        @@ -1,2 +1,2 @@\n alpha\n-nope\n+BETA\n";

    #[test]
    fn valid_patch_is_applied_before_inner_evaluation() {
        let (_root, worktree, ledger) = setup();
        let inner = ScriptedEvaluator::new(vec![ScriptedOutcome::Evaluation(
            Evaluation::metric_only(0.5, "inner"),
        )]);
        let mut evaluator = ApplyingEvaluator::new(worktree.clone(), ledger, Box::new(inner));
        let evaluation = evaluator
            .evaluate(&patch_cs(GOOD_DIFF), &ManualClock::new())
            .expect("applied and evaluated");
        assert_eq!(evaluation.metric, 0.5);
        assert_eq!(
            std::fs::read_to_string(worktree.join("src/lib.rs")).expect("read lib"),
            "alpha\nBETA\n"
        );
    }

    #[test]
    fn failed_apply_never_reaches_the_inner_evaluator() {
        let (_root, worktree, ledger) = setup();
        // One scripted outcome. If the failed apply reached the inner
        // evaluator, the outcome would be consumed by it (or the error
        // would be the script's own); neither may happen.
        let inner = ScriptedEvaluator::new(vec![ScriptedOutcome::Evaluation(
            Evaluation::metric_only(0.5, "inner"),
        )]);
        let mut evaluator = ApplyingEvaluator::new(worktree.clone(), ledger, Box::new(inner));
        let err = evaluator
            .evaluate(&patch_cs(BAD_DIFF), &ManualClock::new())
            .expect_err("bad apply must fail");
        assert!(err.message.contains("apply failed"), "{}", err.message);
        assert_eq!(
            std::fs::read_to_string(worktree.join("src/lib.rs")).expect("read lib"),
            "alpha\nbeta\n"
        );
        // The scripted outcome is still unconsumed: a valid patch
        // applies and evaluates against it.
        let evaluation = evaluator
            .evaluate(&patch_cs(GOOD_DIFF), &ManualClock::new())
            .expect("valid patch evaluates");
        assert_eq!(evaluation.metric, 0.5);
        assert_eq!(
            std::fs::read_to_string(worktree.join("src/lib.rs")).expect("read lib"),
            "alpha\nBETA\n"
        );
    }

    #[test]
    fn trainlab_config_passes_through_to_inner() {
        let (_root, worktree, ledger) = setup();
        let inner = ScriptedEvaluator::new(vec![ScriptedOutcome::Evaluation(
            Evaluation::metric_only(0.75, "inner"),
        )]);
        let mut evaluator = ApplyingEvaluator::new(worktree, ledger, Box::new(inner));
        let cs = ChangeSet {
            id: "cfg".to_string(),
            kind: ChangeKind::TrainlabConfig,
            paths: Vec::new(),
            payload: "{}".to_string(),
            rationale: "config".to_string(),
        };
        let evaluation = evaluator
            .evaluate(&cs, &ManualClock::new())
            .expect("config evaluates");
        assert_eq!(evaluation.metric, 0.75);
    }
}
