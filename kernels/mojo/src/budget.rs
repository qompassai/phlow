//! Launch budgets and the launch planner.
//!
//! Validation says a launch *could* run; the budget says it *may* run now.
//! [`LaunchPlanner`] tracks committed launches and payload bytes against a
//! [`LaunchBudget`]. Planning is validate -> prepare -> commit -> observe: a
//! rejected plan leaves the counters exactly as they were.

use crate::error::KernelError;
use crate::launch::ValidatedLaunch;

/// Maximum launches one planner commits.
pub const LAUNCHES_MAX: u32 = 1024;
/// Maximum aggregate payload bytes one planner commits (1 GiB).
pub const BUDGET_PAYLOAD_BYTES_MAX: u64 = 1024 * 1024 * 1024;

/// The work budget a sequence of launches must fit in.
///
/// `launches_max` bounds fan-out per planning session; `payload_bytes_max`
/// bounds aggregate host->device traffic. Both are named so the operator can
/// reason about the worst case before any launch is planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchBudget {
    /// Maximum launches the planner will commit.
    pub launches_max: u32,
    /// Maximum aggregate payload bytes the planner will commit.
    pub payload_bytes_max: u64,
}

impl LaunchBudget {
    /// Build a budget, rejecting zero budgets (a planner that can plan
    /// nothing is a configuration error, not a degenerate budget).
    ///
    /// # Errors
    ///
    /// [`KernelError::LaunchesExceeded`] if `launches_max` is 0;
    /// [`KernelError::BudgetBytesExceeded`] if `payload_bytes_max` is 0.
    /// (Both reuse the "budget exhausted" variants with `used: 0`.)
    pub fn new(launches_max: u32, payload_bytes_max: u64) -> Result<Self, KernelError> {
        if launches_max == 0 {
            return Err(KernelError::LaunchesExceeded { used: 0, max: 0 });
        }
        if payload_bytes_max == 0 {
            return Err(KernelError::BudgetBytesExceeded {
                requested: 0,
                max: 0,
            });
        }
        if launches_max > LAUNCHES_MAX {
            return Err(KernelError::LaunchesExceeded {
                used: launches_max,
                max: LAUNCHES_MAX,
            });
        }
        if payload_bytes_max > BUDGET_PAYLOAD_BYTES_MAX {
            return Err(KernelError::BudgetBytesExceeded {
                requested: payload_bytes_max,
                max: BUDGET_PAYLOAD_BYTES_MAX,
            });
        }
        Ok(Self {
            launches_max,
            payload_bytes_max,
        })
    }
}

/// A launch the planner committed, ready for an executor.
///
/// Carries the validated launch plus its sequence number in this planner.
/// The sequence number lets an executor order receipts and lets tests assert
/// exactly-once planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    launch: ValidatedLaunch,
    sequence: u64,
}

impl LaunchPlan {
    /// The validated launch this plan commits.
    #[must_use]
    pub fn launch(&self) -> &ValidatedLaunch {
        &self.launch
    }

    /// This plan's sequence number (0-based, per planner).
    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
}

/// Plans validated launches against a [`LaunchBudget`].
///
/// Owns the commit counters. `plan` checks the budget first and only then
/// increments; a rejected plan changes nothing.
#[derive(Debug)]
pub struct LaunchPlanner {
    budget: LaunchBudget,
    launches_used: u32,
    payload_bytes_used: u64,
    sequence: u64,
}

impl LaunchPlanner {
    /// A planner with a fresh (zero-used) budget.
    #[must_use]
    pub fn new(budget: LaunchBudget) -> Self {
        Self {
            budget,
            launches_used: 0,
            payload_bytes_used: 0,
            sequence: 0,
        }
    }

    /// Commit a validated launch to a plan.
    ///
    /// # Errors
    ///
    /// [`KernelError::LaunchesExceeded`] or
    /// [`KernelError::BudgetBytesExceeded`] when the budget is exhausted.
    /// Counters are unchanged on rejection.
    pub fn plan(&mut self, launch: ValidatedLaunch) -> Result<LaunchPlan, KernelError> {
        if self.launches_used >= self.budget.launches_max {
            return Err(KernelError::LaunchesExceeded {
                used: self.launches_used,
                max: self.budget.launches_max,
            });
        }
        let payload_after = self
            .payload_bytes_used
            .checked_add(u64::from(launch.payload_bytes()))
            .ok_or(KernelError::ArithmeticOverflow {
                what: "planner payload total",
            })?;
        if payload_after > self.budget.payload_bytes_max {
            return Err(KernelError::BudgetBytesExceeded {
                requested: payload_after,
                max: self.budget.payload_bytes_max,
            });
        }
        // Commit point: every check passed, publish the new counters once.
        self.launches_used += 1;
        self.payload_bytes_used = payload_after;
        let plan = LaunchPlan {
            launch,
            sequence: self.sequence,
        };
        self.sequence += 1;
        Ok(plan)
    }

    /// Launches committed so far.
    #[must_use]
    pub fn launches_used(&self) -> u32 {
        self.launches_used
    }

    /// Payload bytes committed so far.
    #[must_use]
    pub fn payload_bytes_used(&self) -> u64 {
        self.payload_bytes_used
    }

    /// The budget this planner enforces.
    #[must_use]
    pub fn budget(&self) -> LaunchBudget {
        self.budget
    }
}
