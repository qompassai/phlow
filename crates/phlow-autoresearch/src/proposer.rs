//! The proposer seam. In production the planner specialist fills this
//! trait (emitting change-sets as strict JSON); in tests and in the CLI
//! scenario driver, [`ScriptedProposer`] replays a fixed queue. The
//! split mirrors trainlab's `Sampler` / `ScriptedSampler`: the loop's
//! orchestration never depends on a live model.

use std::collections::VecDeque;

use crate::changeset::ChangeSet;

/// What the proposer may know when choosing the next experiment.
#[derive(Debug, Clone, PartialEq)]
pub struct ProposeContext {
    /// 1-based iteration about to run (counting this run's iterations
    /// only; a resumed ledger does not renumber).
    pub iteration: u32,
    /// The incumbent metric, if a baseline has been established.
    pub incumbent_metric: Option<f64>,
    /// Hash-chain head of the ledger, for proposals that want to cite
    /// the exact state they were made against.
    pub ledger_head_sha256: Option<String>,
}

/// Why a proposal could not be produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposeError {
    /// The proposer has no further experiments. A normal end of run.
    Exhausted,
    /// The proposer emitted something unusable; recorded as
    /// `proposal_invalid` and counted as a failed iteration.
    Invalid(String),
}

/// Source of change-sets for the loop.
pub trait Proposer {
    /// Produce the next change-set, or explain why there is none.
    ///
    /// # Errors
    /// [`ProposeError::Exhausted`] for a clean end of ideas;
    /// [`ProposeError::Invalid`] for a malformed proposal.
    fn propose(&mut self, context: &ProposeContext) -> Result<ChangeSet, ProposeError>;
}

/// One scripted proposal: a change-set to emit, or an invalid-proposal
/// failure to raise.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptedProposal {
    /// Emit this change-set.
    ChangeSet(ChangeSet),
    /// Raise [`ProposeError::Invalid`] with this message.
    Invalid(String),
}

/// A proposer that replays a fixed queue, then reports exhaustion.
#[derive(Debug, Default)]
pub struct ScriptedProposer {
    queue: VecDeque<ScriptedProposal>,
}

impl ScriptedProposer {
    /// A scripted proposer emitting `proposals` in order.
    #[must_use]
    pub fn new(proposals: Vec<ScriptedProposal>) -> Self {
        ScriptedProposer {
            queue: proposals.into(),
        }
    }

    /// Proposals not yet consumed.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.queue.len()
    }
}

impl Proposer for ScriptedProposer {
    fn propose(&mut self, _context: &ProposeContext) -> Result<ChangeSet, ProposeError> {
        match self.queue.pop_front() {
            None => Err(ProposeError::Exhausted),
            Some(ScriptedProposal::ChangeSet(change_set)) => Ok(change_set),
            Some(ScriptedProposal::Invalid(message)) => Err(ProposeError::Invalid(message)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::ChangeKind;

    fn context() -> ProposeContext {
        ProposeContext {
            iteration: 1,
            incumbent_metric: None,
            ledger_head_sha256: None,
        }
    }

    #[test]
    fn scripted_proposer_replays_then_exhausts() {
        let change_set = ChangeSet {
            id: "one".to_string(),
            kind: ChangeKind::TrainlabConfig,
            paths: Vec::new(),
            payload: "{}".to_string(),
            rationale: "first".to_string(),
        };
        let mut proposer = ScriptedProposer::new(vec![
            ScriptedProposal::ChangeSet(change_set.clone()),
            ScriptedProposal::Invalid("bad json".to_string()),
        ]);
        assert_eq!(proposer.propose(&context()), Ok(change_set));
        assert_eq!(
            proposer.propose(&context()),
            Err(ProposeError::Invalid("bad json".to_string()))
        );
        assert_eq!(proposer.propose(&context()), Err(ProposeError::Exhausted));
        assert_eq!(proposer.remaining(), 0);
    }
}
