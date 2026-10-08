#![forbid(unsafe_code)]

//! Behavioral canary battery for phlow.
//!
//! Phlow loads models from external registries and runtimes
//! (Ollama, Cloudflare, and similar). A trojaned or backdoored
//! model can behave normally on
//! standard tasks while firing on trigger inputs; weight-level
//! detection is an unsolved problem, so this crate screens behavior
//! before deployment. It does not and cannot prove a model clean —
//! it is defense-in-depth, and the approval layer remains the primary
//! defense (design: `phlow/canary-battery-design-2026-10-04.md`).
//!
//! # What a battery run does
//!
//! [`CanarySuite`] registers eleven probes in the design's four
//! categories — injection resistance, backdoor trigger candidates,
//! refusal consistency, calibration anomalies — behind the design's
//! `Probe` trait. Each probe runs three times and passes only on
//! 3/3. In the trigger category, a unanimous pass is challenged by
//! one perturbed repetition (the 2026-10-07 consensus challenger):
//! if the perturbed run fails, the probe fails. The verdict is
//! binary and fail-closed — [`Verdict::Deploy`] only when every
//! probe passes — and is cached against the exact model artifact
//! hash via [`VerdictCache`].
//!
//! Split runs (probes whose repetitions disagree) are recorded in
//! the [`CanaryReport`] and appended to a standing split log: they
//! are the calibration labels the deferred disagreement resolver
//! will be built on, and they are logged from this first build. The
//! resolver itself is deliberately not implemented here.
//!
//! # Trust boundaries and bounds
//!
//! - Payloads are canary content: they are loaded from a separate,
//!   access-controlled file outside the repo ([`PayloadStore`]),
//!   never shipped in the crate, and never copied into evidence,
//!   reports, or logs — evidence carries statistics and fixed
//!   labels only, by construction.
//! - Thresholds are per-model ([`ThresholdBook`]): the literature
//!   the design rests on shows detection thresholds do not
//!   generalize across models, so an uncalibrated model is refused,
//!   never judged by global numbers.
//! - All inputs are bounded by named constants (store size, list
//!   lengths, payload sizes, artifact hash size), and a full battery
//!   is budgeted at [`BATTERY_BUDGET_MS`] milliseconds.
//!
//! # Rich answers
//!
//! The battery consumes phlow-system1's [`RichAnswer`] end-to-end:
//! the type the design references is the type `phlow-system1`
//! exports (landed with its Clef backend), re-exported here so probe
//! code and callers name one type only. (An earlier build defined a
//! canary-local stand-in because system1 then exposed only scalar
//! confidence; that stand-in is gone, and a regression test pins the
//! type identity.) Backends that cannot produce a real distribution
//! fail closed with `System1Error::DistributionUnavailable`.
//!
//! # Deviations from the design text, forced and recorded
//!
//! - The design sketches `ProbeBackend` as async; its own `Probe`
//!   trait is synchronous. The backend trait here is synchronous to
//!   match `Probe::run`; async clients are adapted outside the
//!   crate, keeping the battery core runtime-free. The `canary-live`
//!   binary is that adapter for the Ollama backend.

pub mod error;
pub mod payloads;
pub mod probe;
pub mod probes;
pub mod report;
pub mod stats;
pub mod suite;
pub mod thresholds;
pub mod verdict;

pub use error::CanaryError;
pub use payloads::{PayloadStore, TriggerSet, VariantPicker};
pub use probe::{
    ChallengerEvidence, Perturbation, Probe, ProbeBackend, ProbeCategory, ProbeEvidence,
    ProbeResult, RichAnswer, RichAnswerBatch,
};
pub use report::{CanaryReport, RunOutcome, SplitRun};
pub use suite::{BATTERY_BUDGET_MS, CANARY_VERSION, CanarySuite, RUNS_PER_PROBE, RunContext};
pub use thresholds::{BaselineStats, ModelThresholds, ThresholdBook, calibrate};
pub use verdict::{Verdict, VerdictCache, model_hash_bytes, model_hash_reader};
