//! The selection gate: the D_sel acceptance rule.
//!
//! A candidate skill is accepted iff its score on the selection split
//! is **strictly greater** than the current skill's; ties are rejected,
//! "including ties caused by evaluation variance" (paper §II.5).
//! [`GateMode::Off`] and [`GateMode::TieAccepts`] exist only as ablation
//! arms (task-102): they are not valid loop configurations.

/// The acceptance rule under test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateMode {
    /// Accept iff candidate D_sel is strictly greater than current.
    /// The paper's rule; ties rejected.
    Strict,
    /// Accept every candidate (ablation: is the gate needed at all?).
    Off,
    /// Accept on greater-or-equal (ablation: does the strict-greater
    /// choice matter against evaluation variance?).
    TieAccepts,
}

/// Decide acceptance. Scores are D_sel fractions in [0, 1]; the
/// comparison is exact (deterministic targets, fixed seeds — no epsilon).
pub fn decide(mode: GateMode, current: f64, candidate: f64) -> bool {
    match mode {
        GateMode::Strict => candidate > current,
        GateMode::Off => true,
        GateMode::TieAccepts => candidate >= current,
    }
}

#[cfg(test)]
mod tests {
    use super::{GateMode, decide};

    /// Validation: the strict-greater truth table, including the
    /// paper's "ties rejected" rule.
    #[test]
    fn strict_truth_table() {
        assert!(decide(GateMode::Strict, 0.5, 0.6));
        assert!(!decide(GateMode::Strict, 0.5, 0.5), "ties rejected");
        assert!(!decide(GateMode::Strict, 0.6, 0.5));
    }

    /// Validation: the ablation arms behave as labeled.
    #[test]
    fn ablation_arms() {
        assert!(decide(GateMode::Off, 0.9, 0.1), "off accepts everything");
        assert!(
            decide(GateMode::TieAccepts, 0.5, 0.5),
            "tie arm accepts ties"
        );
        assert!(!decide(GateMode::TieAccepts, 0.5, 0.4));
    }
}
