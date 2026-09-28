//! The task contract: what the work must achieve, up front.

use crate::error::CouncilError;

/// Largest number of characters in the contract goal.
pub const GOAL_CHARS_MAX: usize = 512;
/// Largest number of constraint entries.
pub const CONSTRAINTS_MAX: usize = 16;
/// Largest number of acceptance-criteria entries.
pub const ACCEPTANCE_MAX: usize = 16;
/// Largest number of characters in one list entry.
pub const ENTRY_CHARS_MAX: usize = 256;

/// What the candidate work is for: the goal, the hard constraints, and the
/// acceptance criteria. Written before any candidate is proposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskContract {
    /// The goal, in bounded prose.
    pub goal: String,
    /// Hard constraints the work must respect.
    pub constraints: Vec<String>,
    /// What "done" means.
    pub acceptance: Vec<String>,
}

impl TaskContract {
    /// Builds a contract, validating every bound.
    ///
    /// # Contract
    /// - Accepts: non-empty goal within [`GOAL_CHARS_MAX`] chars,
    ///   constraints within [`CONSTRAINTS_MAX`] entries, acceptance within
    ///   [`ACCEPTANCE_MAX`] entries, each entry within
    ///   [`ENTRY_CHARS_MAX`] chars.
    /// - Rejects: empty goals, overlong text, oversized lists — with a
    ///   typed error naming the field.
    pub fn new(
        goal: &str,
        constraints: &[&str],
        acceptance: &[&str],
    ) -> Result<Self, CouncilError> {
        if goal.trim().is_empty() {
            return Err(CouncilError::EmptyGoal);
        }
        check_len("goal", goal, GOAL_CHARS_MAX)?;
        if constraints.len() > CONSTRAINTS_MAX {
            return Err(CouncilError::TooManyItems {
                field: "constraints",
                max: CONSTRAINTS_MAX,
            });
        }
        if acceptance.len() > ACCEPTANCE_MAX {
            return Err(CouncilError::TooManyItems {
                field: "acceptance",
                max: ACCEPTANCE_MAX,
            });
        }
        let mut owned_constraints = Vec::with_capacity(constraints.len());
        for entry in constraints {
            check_len("constraint entry", entry, ENTRY_CHARS_MAX)?;
            owned_constraints.push((*entry).to_string());
        }
        let mut owned_acceptance = Vec::with_capacity(acceptance.len());
        for entry in acceptance {
            check_len("acceptance entry", entry, ENTRY_CHARS_MAX)?;
            owned_acceptance.push((*entry).to_string());
        }
        Ok(Self {
            goal: goal.to_string(),
            constraints: owned_constraints,
            acceptance: owned_acceptance,
        })
    }
}

/// Rejects text longer than `max` characters.
pub(crate) fn check_len(field: &'static str, value: &str, max: usize) -> Result<(), CouncilError> {
    let got = value.chars().count();
    if got > max {
        return Err(CouncilError::TextTooLong { field, max, got });
    }
    Ok(())
}
