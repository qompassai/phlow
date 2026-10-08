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
//! - [`ollama_proposer`]: the live proposer binding — the planner
//!   specialist over the Ollama-shaped HTTP API, loopback only, with
//!   a fakeable transport so tests never need a live daemon.
//! - [`receipt_evaluator`]: the live evaluator adapter — derives the
//!   dev-split metric from real trainlab receipts and verifies
//!   trainer receipts (hash chain, adapter movement, parity guard)
//!   instead of running anything itself.
//! - [`applier`]: the `FilePatch` applier — the loop's apply step,
//!   restricted unified-diff subset, compute-before-write.
//! - [`applying_evaluator`]: composition of the applier with an
//!   inner evaluator (apply first, then measure).
//! - [`live_evaluator`]: the fully live evaluator — runs trainlab on
//!   the frozen dev split for real and scores the receipt through
//!   [`receipt_evaluator`].
//! - [`ledger`]: the hash-chained, append-only experiment ledger.
//! - [`research_loop`]: the state machine, budgets, and halt logic.
//! - [`clock`]: monotonic clocks (system for production, manual for
//!   deterministic tests).
//! - [`error`]: the typed error and the failure taxonomy.

pub mod applier;
pub mod applying_evaluator;
pub mod changeset;
pub mod clock;
pub mod error;
pub mod evaluator;
pub mod ledger;
pub mod live_evaluator;
pub mod ollama_proposer;
pub mod proposer;
pub mod receipt_evaluator;
pub mod research_loop;

#[cfg(test)]
pub(crate) mod testsupport;

pub use applier::apply_change_set;
pub use applying_evaluator::ApplyingEvaluator;
pub use changeset::{ChangeKind, ChangeSet, validate_change_set};
pub use clock::{Clock, ManualClock, SystemClock};
pub use error::{AutoresearchError, FailureClass};
pub use evaluator::{EvalError, Evaluation, Evaluator, ScriptedEvaluator, ScriptedOutcome};
pub use ledger::{Decision, EntryDraft, EntryKind, Ledger, LedgerEntry};
pub use live_evaluator::{BaseConfig, LiveEvaluator};
pub use ollama_proposer::{ChatTransport, HttpTransport, OllamaProposer, ProposerConfig};
pub use proposer::{ProposeContext, ProposeError, Proposer, ScriptedProposal, ScriptedProposer};
pub use receipt_evaluator::{EvaluationBundle, ReceiptEvaluator, TrainerEvidence};
pub use research_loop::{HaltReason, LoopConfig, LoopReport, run_loop};
