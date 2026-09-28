//! SoL-Pi-inspired efficiency mechanisms for the agent layer.
//!
//! Re-expresses two of the four SoL-Pi mechanisms (NVlabs/SoL-Pi, MIT
//! license) as opt-in, disabled-by-default harness features:
//!
//! - [`reducer`]: evidence-preserving reduction — long diagnostic text
//!   becomes a compact receipt only when every retained quotation matches
//!   the source byte-for-byte; any mismatch leaves the original unchanged.
//! - [`context_compact`]: online context compaction — completed steps
//!   become compaction candidates under explicit economic and
//!   window-pressure checks with named thresholds.
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
//! # Opt-in contract
//!
//! A missing or default configuration leaves every mechanism disabled.
//! [`SolpiAgentConfig::disabled`] (the `Default`) is the only way to
//! start; each mechanism requires an explicit
//! [`SolpiAgentConfig::enable`] call.

pub mod context_compact;
pub mod reducer;

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
}

/// Explicit opt-in configuration for the agent-layer mechanisms.
///
/// All mechanisms are disabled by default. There is intentionally no
/// constructor that enables anything: enabling requires [`Self::enable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolpiAgentConfig {
    evidence_reducer: bool,
    online_compact: bool,
}

impl SolpiAgentConfig {
    /// Configuration with every mechanism disabled.
    pub fn disabled() -> Self {
        Self {
            evidence_reducer: false,
            online_compact: false,
        }
    }

    /// Explicitly enable one mechanism. This is the only opt-in path.
    pub fn enable(&mut self, feature: SolpiFeature) -> &mut Self {
        match feature {
            SolpiFeature::EvidenceReducer => self.evidence_reducer = true,
            SolpiFeature::OnlineCompact => self.online_compact = true,
        }
        self
    }

    /// True only when the mechanism was explicitly enabled.
    pub fn is_enabled(&self, feature: SolpiFeature) -> bool {
        match feature {
            SolpiFeature::EvidenceReducer => self.evidence_reducer,
            SolpiFeature::OnlineCompact => self.online_compact,
        }
    }
}

impl Default for SolpiAgentConfig {
    /// Default is disabled, matching the missing-config rule.
    fn default() -> Self {
        Self::disabled()
    }
}

#[cfg(test)]
mod tests {
    use super::{SolpiAgentConfig, SolpiFeature};

    #[test]
    fn default_config_disables_everything() {
        let config = SolpiAgentConfig::default();
        assert!(!config.is_enabled(SolpiFeature::EvidenceReducer));
        assert!(!config.is_enabled(SolpiFeature::OnlineCompact));
    }

    #[test]
    fn enable_is_explicit_and_independent() {
        let mut config = SolpiAgentConfig::disabled();
        config.enable(SolpiFeature::OnlineCompact);
        assert!(config.is_enabled(SolpiFeature::OnlineCompact));
        assert!(!config.is_enabled(SolpiFeature::EvidenceReducer));
    }
}
