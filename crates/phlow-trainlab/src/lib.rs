#![forbid(unsafe_code)]

//! Executable-reward post-training experiment harness for phlow.
//!
//! # Status: EXPERIMENTAL
//!
//! This crate ports the *measurement* half of the from-scratch
//! GLM-5.3-Flash training course (Vuk Rosić / freeCodeCamp, companion repo
//! `vukrosic/glm-5.3-flash-from-scratch`) into phlow: frozen task splits,
//! executable rewards, group-relative (RLOO) statistics, pass@k
//! evaluation, select-on-dev / open-confirm-once discipline, and run
//! receipts. It enables **no weight updates**: phlow's specialists are
//! GGUF models served by Ollama and expose no gradient path, so the
//! runner records the advantages an update *would* have used and stops
//! there. A trainer backend is the named follow-up that would close the
//! loop.
//!
//! # Hard constraints
//!
//! - **Generated code is executed only through an explicitly
//!   acknowledged executor.** [`executor::ExecutorConfig`] requires
//!   `acknowledge_code_execution == true`; phlow is not an OS sandbox,
//!   and this crate never pretends otherwise. Execution happens in a
//!   fresh temporary directory under a deadline with bounded output.
//! - **Splits are frozen and disjoint.** [`task::frozen_tasks`]
//!   generates deterministic tasks from per-split seeds with unseen
//!   function names per split; a confirmation split can be opened only
//!   once, against a previously recorded selection
//!   ([`gate::ConfirmationGate`]).
//! - **Fail closed.** Unknown splits/families, exhausted budgets,
//!   oversized completions, missing harness results, and tampered
//!   receipts are typed errors or explicit `Invalid` evaluations —
//!   never inferred success.
//! - **Loopback by default.** The Ollama sampler refuses non-loopback
//!   base URLs unless remote access was explicitly opted into.
//!
//! # Layout
//!
//! - [`task`]: synthetic coding-task families and frozen split
//!   generation (mirrors `glm53_flash/tasks.py`).
//! - [`executor`]: bounded subprocess execution of a completion
//!   against a task's hidden cases.
//! - [`reward`]: evaluation classification and reward semantics
//!   (binary `1.0 / 0.0 / penalty`, or case-fraction).
//! - [`group`]: leave-one-out (RLOO) advantages and the unbiased
//!   pass@k estimator.
//! - [`sampler`]: the completion-sampler contract, a scripted sampler
//!   for tests, and a loopback Ollama sampler.
//! - [`gate`]: the selection/confirmation ledger.
//! - [`receipt`]: the immutable per-run JSON receipt.
//! - [`runner`]: group scheduling, the run loop, and pass@k reports.

pub mod error;
pub mod executor;
pub mod gate;
pub mod group;
pub mod receipt;
pub mod reward;
pub mod rng;
pub mod runner;
pub mod sampler;
pub mod task;

pub use error::TrainlabError;
pub use executor::{Executor, ExecutorConfig};
pub use gate::ConfirmationGate;
pub use group::{leave_one_out_advantages, pass_at_k};
pub use receipt::RunReceipt;
pub use reward::{EvalStatus, Evaluation, RewardConfig, RewardMode};
pub use runner::{PasskReport, RunConfig, evaluate_passk, run_experiment};
pub use sampler::{OllamaSampler, Sampler, ScriptedSampler};
pub use task::{CodingTask, Split, frozen_tasks};
