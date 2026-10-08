#![forbid(unsafe_code)]

//! Bounded autoresearch loop orchestration for phlow.
//!
//! # Status: EXPERIMENTAL
//!
//! This crate ports the discipline of Karpathy's autoresearch — one
//! mutable artifact, one metric, a fixed budget, a complete experiment
//! log, keep/discard by measurement — onto phlow's existing evidence
//! surfaces, and inverts its autonomy: the loop is bounded, sandboxed,
//! and can only propose. Promotion of anything it finds stays behind
//! phlow-experiment's human-signed gate.
//!
//! # Hard constraints
//!
//! - **No autonomous self-modification or self-approval.** The loop
//!   applies typed [`changeset::ChangeSet`]s inside one experiment
//!   worktree, measures them, and records verdicts. It contains no
//!   promotion code and no path to trainlab's confirmation gate: the
//!   confirmation split is opened by humans, against a recorded
//!   selection, never by this crate.
//! - **Bounded everything.** Iteration cap, per-experiment and total
//!   wall-clock budgets, and a consecutive-failure budget are explicit
//!   constants in [`research_loop`], validated before a run and
//!   enforced during it. A second gate violation halts a run outright.
//! - **Fail closed.** The [`ledger::Ledger`] hash chain is verified in
//!   full before the first iteration; corrupt evidence, malformed
//!   receipt hashes, and non-finite metrics are failures, never scores.
//! - **Containment.** Change-set paths are validated by canonical
//!   ancestry inside the experiment worktree — never by string prefix —
//!   and the fixed harness (trainlab, phlow-experiment), the build
//!   manifests, and the ledger itself are forbidden surfaces.
//!
//! # Layout
//!
//! - [`changeset`]: the typed, bounded change-set and its validation.
//! - [`proposer`]: the proposer seam (planner specialist in
//!   production; scripted in tests).
//! - [`evaluator`]: the evaluator seam (trainlab on the frozen dev
//!   split in production; scripted in tests).
//! - [`ledger`]: the hash-chained, append-only experiment ledger.
//! - [`research_loop`]: the state machine, budgets, and halt logic.
//! - [`clock`]: monotonic clocks (system for production, manual for
//!   deterministic tests).
//! - [`error`]: the typed error and the failure taxonomy.

pub mod changeset;
pub mod clock;
pub mod error;
pub mod evaluator;
pub mod ledger;
pub mod proposer;
pub mod research_loop;

#[cfg(test)]
pub(crate) mod testsupport;

pub use changeset::{ChangeKind, ChangeSet, validate_change_set};
pub use clock::{Clock, ManualClock, SystemClock};
pub use error::{AutoresearchError, FailureClass};
pub use evaluator::{EvalError, Evaluation, Evaluator, ScriptedEvaluator, ScriptedOutcome};
pub use ledger::{Decision, EntryDraft, EntryKind, Ledger, LedgerEntry};
pub use proposer::{ProposeContext, ProposeError, Proposer, ScriptedProposal, ScriptedProposer};
pub use research_loop::{HaltReason, LoopConfig, LoopReport, run_loop};
