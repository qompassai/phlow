//! SoL-Pi-inspired efficiency mechanisms for the tool layer.
//!
//! Re-expresses two of the four SoL-Pi mechanisms (NVlabs/SoL-Pi, MIT
//! license) as opt-in, disabled-by-default harness features:
//!
//! - [`action_fusion`]: an edit/write runs its follow-up validation in the
//!   same call, with the validation step bounded and its result attached
//!   to the action receipt.
//! - [`observation_pack`]: large tool results become stable paged handles
//!   instead of raw dumps; pages are retrievable on demand with bounds.
//!
//! These are concept re-expressions, not ports of the upstream TypeScript.
//! Design record and upstream citations live in
//! `crates/phlow-tools/docs/solpi-decisions.md`.
//!
//! # Opt-in contract
//!
//! A missing or default configuration leaves every mechanism disabled —
//! the same rule as upstream ("a missing configuration leaves every
//! mechanism disabled"). [`SolpiToolConfig::disabled`] (the `Default`) is
//! the only way to start; each mechanism requires an explicit
//! [`SolpiToolConfig::enable`] call. Calls made while disabled fail with
//! a typed error and perform no work.

pub mod action_fusion;
pub mod observation_pack;

pub use action_fusion::{
    ActionOutcome, FusionError, FusionPolicy, FusionReceipt, ValidationOutcome, ValidationReport,
    fuse,
};
pub use observation_pack::{
    ObservationHandle, ObservationPage, ObservationProjection, PackError, PackStore,
};

/// The tool-layer SoL-Pi mechanisms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SolpiFeature {
    /// An edit/write runs its follow-up validation in the same call.
    ActionFusion,
    /// Large tool results become stable paged handles.
    ObservationPack,
}

/// Explicit opt-in configuration for the tool-layer mechanisms.
///
/// All mechanisms are disabled by default. There is intentionally no
/// constructor that enables anything: enabling requires [`Self::enable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolpiToolConfig {
    action_fusion: bool,
    observation_pack: bool,
}

impl SolpiToolConfig {
    /// Configuration with every mechanism disabled.
    pub fn disabled() -> Self {
        Self {
            action_fusion: false,
            observation_pack: false,
        }
    }

    /// Explicitly enable one mechanism. This is the only opt-in path.
    pub fn enable(&mut self, feature: SolpiFeature) -> &mut Self {
        match feature {
            SolpiFeature::ActionFusion => self.action_fusion = true,
            SolpiFeature::ObservationPack => self.observation_pack = true,
        }
        self
    }

    /// True only when the mechanism was explicitly enabled.
    pub fn is_enabled(&self, feature: SolpiFeature) -> bool {
        match feature {
            SolpiFeature::ActionFusion => self.action_fusion,
            SolpiFeature::ObservationPack => self.observation_pack,
        }
    }
}

impl Default for SolpiToolConfig {
    /// Default is disabled, matching upstream's missing-config rule.
    fn default() -> Self {
        Self::disabled()
    }
}

#[cfg(test)]
mod tests {
    use super::{SolpiFeature, SolpiToolConfig};

    #[test]
    fn default_config_disables_everything() {
        let config = SolpiToolConfig::default();
        assert!(!config.is_enabled(SolpiFeature::ActionFusion));
        assert!(!config.is_enabled(SolpiFeature::ObservationPack));
    }

    #[test]
    fn enable_is_explicit_and_independent() {
        let mut config = SolpiToolConfig::disabled();
        config.enable(SolpiFeature::ActionFusion);
        assert!(config.is_enabled(SolpiFeature::ActionFusion));
        assert!(!config.is_enabled(SolpiFeature::ObservationPack));
    }
}
