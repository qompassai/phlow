//! SkillOpt gauntlet waves: ablation + robustness experiments for the
//! paper's self-improving skill loop (arXiv 2605.23904v2, tasks 101–115).
//!
//! **Experimental-only.** Nothing here touches production skills, diver
//! config, or deployed state: the loop optimizes experiment-state
//! [`doc::SkillDoc`]s inside a [`harness::Sandbox`], accepted edits are
//! staged for operator approval, and export requires an
//! [`harness::ApprovalRecord`] binding the exact exported bytes.
//!
//! Module map:
//! - [`rng`]: deterministic seeded RNG streams.
//! - [`doc`]: skill documents, edit ops, char budgets, the protected
//!   slow-update section, experiment/untrusted provenance.
//! - [`target`]: scripted task targets (F-order, F-bind, F-ledger) with
//!   fixed seeded splits.
//! - [`optimizer`]: the optimizer seam; the scripted mock (offline
//!   control) and the real Ollama-backed [`optimizer::ModelOptimizer`].
//! - [`gate`]: the D_sel acceptance gate.
//! - [`learner`]: the loop: rollout → reflect → propose → gate → apply,
//!   epoch-end slow/meta updates.
//! - [`harness`]: the safety boundary (sandbox, approval-gated export).

pub mod doc;
pub mod driver;
pub mod gate;
pub mod harness;
pub mod learner;
pub mod optimizer;
pub mod rng;
pub mod target;
