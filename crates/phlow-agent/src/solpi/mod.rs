//! SoL-Pi-inspired efficiency mechanisms for the agent layer.
//!
//! Re-expresses SoL-Pi mechanisms (NVlabs/SoL-Pi, MIT license) as
//! harness features. The reducer and the online compaction are
//! opt-in, disabled by default; the context API is the baseline
//! (see the configuration contract below):
//!
//! - [`reducer`]: evidence-preserving reduction — long diagnostic text
//!   becomes a compact receipt only when every retained quotation matches
//!   the source byte-for-byte; any mismatch leaves the original unchanged.
//! - [`context_compact`]: online context compaction — completed steps
//!   become compaction candidates under explicit economic and
//!   window-pressure checks with named thresholds.
//! - [`context_api`]: the model-facing context API (Manuscript
//!   Phase 1) — the stateful half built on `context_compact`'s
//!   decisions: a pinned task and tally, archive-backed collapse and
//!   retrieval, a true-count usage indicator, and per-turn tool-result
//!   budgeting. Compaction archives; it never discards. Adopted as
//!   the harness baseline on 2026-11-27 (Owner ruling): the
//!   protective core is default behaviour, not an opt-in.
//!
//! These are concept re-expressions, not ports of the upstream TypeScript.
//! Design record and upstream citations live in
//! `crates/phlow-agent/docs/solpi-decisions.md`.
//!
//! Deliberate scope cut: upstream's reducer may send log content to a
//! configured reducer model. This port makes no model or network calls —
//! the reduction proposal is caller-supplied and all verification is local,
//! so no log content can leave the process through this module.
//!
//! # Configuration contract
//!
//! A missing or default configuration is the baseline:
//! [`SolpiFeature::ContextApi`] — the Manuscript protective core —
//! is enabled, and the two opt-in mechanisms are disabled. Enabling
//! an opt-in mechanism requires an explicit
//! [`SolpiAgentConfig::enable`] call; turning the baseline off
//! requires an explicit [`SolpiAgentConfig::disable`] call.
//! [`SolpiAgentConfig::disabled`] remains the all-off constructor
//! for callers that need no mechanisms at all.

pub mod context_api;
pub mod context_compact;
pub mod reducer;

pub use context_api::{
    Admission, ArchiveEntry, ArchivedRange, CollapseReceipt, ContextApiError, ContextEvent,
    ContextLedger, EventKind, HarnessCompactError, TallyAuthor, TallyEntry, UsageIndicator,
    admit_tool_results,
};
pub use context_compact::{
    CompactStep, CompactionPlan, CompactionPolicy, PolicyError, evaluate_compaction,
};
pub use reducer::{
    CompactReceipt, EvidenceReducer, ReducerError, ReductionOutcome, ReductionProposal,
};

/// The agent-layer SoL-Pi mechanisms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SolpiFeature {
    /// Compact receipts only on byte-for-byte quote verification.
    EvidenceReducer,
    /// Completed steps become compaction candidates under cost checks.
    OnlineCompact,
    /// The model-facing context API: pinned task and tally,
    /// archive-backed collapse and retrieval. Baseline: enabled in
    /// the default configuration (Manuscript adoption, 2026-11-27).
    ContextApi,
}

/// Configuration for the agent-layer mechanisms.
///
/// The two original mechanisms are opt-in: disabled unless enabled
/// through [`Self::enable`]. The context API is the baseline: the
/// default configuration enables it, and only an explicit
/// [`Self::disable`] call turns it off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolpiAgentConfig {
    evidence_reducer: bool,
    online_compact: bool,
    context_api: bool,
}

impl SolpiAgentConfig {
    /// Configuration with every mechanism disabled.
    pub fn disabled() -> Self {
        Self {
            evidence_reducer: false,
            online_compact: false,
            context_api: false,
        }
    }

    /// The baseline configuration: the Manuscript protective core
    /// (context API) enabled, the opt-in mechanisms disabled. This
    /// is the `Default`.
    pub fn baseline() -> Self {
        Self {
            evidence_reducer: false,
            online_compact: false,
            context_api: true,
        }
    }

    /// Explicitly enable one mechanism: the opt-in path for the
    /// reducer and the online compaction, and the way back after a
    /// [`Self::disable`] call.
    pub fn enable(&mut self, feature: SolpiFeature) -> &mut Self {
        match feature {
            SolpiFeature::EvidenceReducer => self.evidence_reducer = true,
            SolpiFeature::OnlineCompact => self.online_compact = true,
            SolpiFeature::ContextApi => self.context_api = true,
        }
        self
    }

    /// Explicitly disable one mechanism. This is the only opt-out
    /// path: the baseline mechanism runs until this call removes it.
    pub fn disable(&mut self, feature: SolpiFeature) -> &mut Self {
        match feature {
            SolpiFeature::EvidenceReducer => self.evidence_reducer = false,
            SolpiFeature::OnlineCompact => self.online_compact = false,
            SolpiFeature::ContextApi => self.context_api = false,
        }
        self
    }

    /// True when the mechanism is in force: an opt-in mechanism that
    /// was explicitly enabled, or the baseline mechanism unless it
    /// was explicitly disabled.
    pub fn is_enabled(&self, feature: SolpiFeature) -> bool {
        match feature {
            SolpiFeature::EvidenceReducer => self.evidence_reducer,
            SolpiFeature::OnlineCompact => self.online_compact,
            SolpiFeature::ContextApi => self.context_api,
        }
    }
}

impl Default for SolpiAgentConfig {
    /// Default is the baseline, matching the missing-config rule:
    /// no configuration means the protective core runs.
    fn default() -> Self {
        Self::baseline()
    }
}

#[cfg(test)]
mod tests {
    use super::{SolpiAgentConfig, SolpiFeature};

    #[test]
    fn default_config_is_the_baseline() {
        let config = SolpiAgentConfig::default();
        assert!(config.is_enabled(SolpiFeature::ContextApi));
        assert!(!config.is_enabled(SolpiFeature::EvidenceReducer));
        assert!(!config.is_enabled(SolpiFeature::OnlineCompact));
        assert_eq!(config, SolpiAgentConfig::baseline());
    }

    #[test]
    fn disabled_config_disables_everything() {
        let config = SolpiAgentConfig::disabled();
        assert!(!config.is_enabled(SolpiFeature::EvidenceReducer));
        assert!(!config.is_enabled(SolpiFeature::OnlineCompact));
        assert!(!config.is_enabled(SolpiFeature::ContextApi));
    }

    #[test]
    fn disable_is_the_explicit_baseline_opt_out() {
        let mut config = SolpiAgentConfig::default();
        config.disable(SolpiFeature::ContextApi);
        assert!(!config.is_enabled(SolpiFeature::ContextApi));
        config.enable(SolpiFeature::ContextApi);
        assert!(config.is_enabled(SolpiFeature::ContextApi));
    }

    #[test]
    fn enable_is_explicit_and_independent() {
        let mut config = SolpiAgentConfig::disabled();
        config.enable(SolpiFeature::OnlineCompact);
        assert!(config.is_enabled(SolpiFeature::OnlineCompact));
        assert!(!config.is_enabled(SolpiFeature::EvidenceReducer));
        assert!(!config.is_enabled(SolpiFeature::ContextApi));
    }

    #[test]
    fn context_api_enable_is_independent() {
        let mut config = SolpiAgentConfig::disabled();
        config.enable(SolpiFeature::ContextApi);
        assert!(config.is_enabled(SolpiFeature::ContextApi));
        assert!(!config.is_enabled(SolpiFeature::OnlineCompact));
        assert!(!config.is_enabled(SolpiFeature::EvidenceReducer));
    }
}
