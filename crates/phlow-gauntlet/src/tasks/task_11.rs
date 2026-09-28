//! task-11: self-approval rejected (rust).
//!
//! Status: STUB — driver not implemented yet. The stub reports an explicit
//! failure so an unimplemented task can never read as passed.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-11";
/// Human-readable name.
pub const NAME: &str = "self-approval rejected";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Attempt the task.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    TaskOutcome::Fail {
        where_: "stub".to_string(),
        how: "task driver not implemented yet".to_string(),
        evidence: vec!["stub: awaiting wave implementation".to_string()],
    }
}
