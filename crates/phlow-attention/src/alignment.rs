//! Alignment measurement and the coordination rule.
//!
//! Plain words: alignment answers "how much are these two agents
//! looking at the same things?" — but only inside one snapshot frame.
//! Across frames the question is meaningless (the workspace changed
//! under them), so [`alignment`] refuses with
//! [`AttentionError::SnapshotMismatch`] and [`decide`] turns that
//! refusal into a fail-closed `Reject` decision. No path in this
//! module lets two agents coordinate on stale common ground.
//!
//! The rule itself mirrors the tasking logic of joint attention:
//! partners whose primary focus coincides must not both charge at the
//! same object — one holds, one defers. Partners looking at different
//! things proceed in parallel. The holder choice is deterministic
//! (lexicographically smaller agent id) so both sides compute the same
//! answer independently, with no negotiation round and no leader.

use serde::{Deserialize, Serialize};

use crate::error::AttentionError;
use crate::focus::Focus;
use crate::snapshot::TargetId;

/// How much two foci overlap, measured inside one snapshot frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Alignment {
    /// Jaccard overlap of the two target sets, `0.0..=1.0`.
    pub score: f64,
    /// Both agents rank the same target first.
    pub top_shared: bool,
    /// Targets both agents attend to, at any rank.
    pub shared_targets: usize,
}

/// Measure the alignment between two foci.
///
/// Fails with [`AttentionError::SnapshotMismatch`] when the foci were
/// computed from different snapshots, and with
/// [`AttentionError::InvalidFocus`] when either focus is structurally
/// invalid. Both foci empty is defined as perfect alignment on
/// nothing (score 1.0, `top_shared` false): there is nothing to
/// contend over, and `decide` treats it as parallel work.
pub fn alignment(a: &Focus, b: &Focus) -> Result<Alignment, AttentionError> {
    a.validate()?;
    b.validate()?;
    if a.snapshot_hash != b.snapshot_hash {
        return Err(AttentionError::SnapshotMismatch);
    }
    let shared = a.targets.iter().filter(|t| b.attends(t)).count();
    let union = a.targets.len() + b.targets.len() - shared;
    let score = if union == 0 {
        1.0
    } else {
        shared as f64 / union as f64
    };
    Ok(Alignment {
        score,
        top_shared: a.top().is_some() && a.top() == b.top(),
        shared_targets: shared,
    })
}

/// The outcome of the coordination rule for a pair of foci.
///
/// JSON-tagged: this is the shape MCP `flow_attention_decide` and the
/// A2A coordination message carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    /// No contention: both agents proceed with their own focus.
    Parallel,
    /// Both agents' primary target is the same object. `holder`
    /// proceeds on it; the other agent defers (handoff, or moves to
    /// its next target). Holder selection is deterministic — see the
    /// module documentation.
    Serialize {
        /// The agent that proceeds on the contended target.
        holder: String,
        /// The contended target.
        target: TargetId,
    },
    /// Coordination refused: the foci are not comparable (different
    /// snapshots) or not trustworthy (invalid). Fail-closed — the
    /// caller must re-snapshot and retry, never proceed on a guess.
    Reject {
        /// Machine-stable reason string: `snapshot_mismatch` or
        /// `invalid_focus`.
        reason: String,
    },
}

/// Apply the coordination rule to a pair of foci.
///
/// Never errors: every failure mode is a `Decision::Reject`, because
/// at a dispatch gate a refusal is an outcome the caller must surface,
/// not an exception to propagate past the gate.
pub fn decide(a: &Focus, b: &Focus) -> Decision {
    if a.validate().is_err() || b.validate().is_err() {
        return Decision::Reject {
            reason: "invalid_focus".to_owned(),
        };
    }
    if a.snapshot_hash != b.snapshot_hash {
        return Decision::Reject {
            reason: "snapshot_mismatch".to_owned(),
        };
    }
    match (a.top(), b.top()) {
        (Some(top_a), Some(top_b)) if top_a == top_b => Decision::Serialize {
            holder: if a.agent_id <= b.agent_id {
                a.agent_id.clone()
            } else {
                b.agent_id.clone()
            },
            target: top_a.clone(),
        },
        _ => Decision::Parallel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saliency::SaliencyInput;
    use crate::snapshot::Snapshot;

    fn target(raw: &str) -> TargetId {
        TargetId::new(raw).expect("test target id is valid")
    }

    fn snapshot_with(hot: &str) -> Snapshot {
        let mut named = SaliencyInput::quiet();
        named.named_in_task = true;
        let filler = if hot == "src/other.rs" {
            "src/hot.rs"
        } else {
            "src/other.rs"
        };
        Snapshot::build(vec![
            (target(hot), named),
            (target(filler), SaliencyInput::quiet()),
        ])
        .expect("builds")
    }

    fn focus(agent: &str, snap: &Snapshot, only: Option<&str>) -> Focus {
        let filter = only.map(|one| {
            let mut set = std::collections::BTreeSet::new();
            set.insert(target(one));
            set
        });
        Focus::derive(agent, snap, 1, filter.as_ref()).expect("derives")
    }

    #[test]
    fn identical_derivations_align_perfectly() {
        let snap = snapshot_with("src/hot.rs");
        let a = focus("agent-a", &snap, None);
        let b = focus("agent-b", &snap, None);
        let measured = alignment(&a, &b).expect("same frame");
        assert_eq!(measured.score, 1.0);
        assert!(measured.top_shared);
    }

    #[test]
    fn cross_frame_alignment_is_refused() {
        // Adversarial: a stale focus from an older workspace state
        // must never yield an alignment number against a fresh one.
        let old = snapshot_with("src/hot.rs");
        let new = snapshot_with("src/other.rs");
        assert_ne!(old.hash(), new.hash());
        let a = focus("agent-a", &old, None);
        let b = focus("agent-b", &new, None);
        let err = alignment(&a, &b).expect_err("cross-frame must fail");
        assert_eq!(err, AttentionError::SnapshotMismatch);
        assert_eq!(
            decide(&a, &b),
            Decision::Reject {
                reason: "snapshot_mismatch".to_owned()
            }
        );
    }

    #[test]
    fn shared_top_serializes_with_deterministic_holder() {
        let snap = snapshot_with("src/hot.rs");
        let a = focus("agent-b", &snap, None);
        let b = focus("agent-a", &snap, None);
        let expected = Decision::Serialize {
            holder: "agent-a".to_owned(),
            target: target("src/hot.rs"),
        };
        // Both argument orders compute the same holder: the decision
        // needs no negotiation round.
        assert_eq!(decide(&a, &b), expected);
        assert_eq!(decide(&b, &a), expected);
    }

    #[test]
    fn disjoint_foci_proceed_in_parallel() {
        let snap = snapshot_with("src/hot.rs");
        let a = focus("agent-a", &snap, Some("src/hot.rs"));
        let b = focus("agent-b", &snap, Some("src/other.rs"));
        assert_eq!(decide(&a, &b), Decision::Parallel);
        let measured = alignment(&a, &b).expect("same frame");
        assert_eq!(measured.score, 0.0);
        assert!(!measured.top_shared);
    }

    #[test]
    fn partial_overlap_still_proceeds_but_is_measured() {
        let snap = snapshot_with("src/hot.rs");
        let a = focus("agent-a", &snap, None);
        let b = focus("agent-b", &snap, Some("src/other.rs"));
        // a attends {hot, other}, b attends {other}: different tops.
        assert_eq!(decide(&a, &b), Decision::Parallel);
        let measured = alignment(&a, &b).expect("same frame");
        assert!((measured.score - 0.5).abs() < 1e-9);
        assert_eq!(measured.shared_targets, 1);
    }

    #[test]
    fn empty_foci_are_parallel_and_perfectly_aligned() {
        let snap = snapshot_with("src/hot.rs");
        let empty_filter = std::collections::BTreeSet::new();
        let a = Focus::derive("agent-a", &snap, 1, Some(&empty_filter)).expect("derives");
        let b = Focus::derive("agent-b", &snap, 1, Some(&empty_filter)).expect("derives");
        assert_eq!(decide(&a, &b), Decision::Parallel);
        assert_eq!(alignment(&a, &b).expect("same frame").score, 1.0);
    }

    #[test]
    fn invalid_focus_is_rejected_not_compared() {
        let snap = snapshot_with("src/hot.rs");
        let good = focus("agent-a", &snap, None);
        let mut bad = focus("agent-b", &snap, None);
        bad.targets.push(target("src/hot.rs")); // duplicate: invalid
        assert!(matches!(
            alignment(&good, &bad),
            Err(AttentionError::InvalidFocus(_))
        ));
        assert_eq!(
            decide(&good, &bad),
            Decision::Reject {
                reason: "invalid_focus".to_owned()
            }
        );
    }

    #[test]
    fn decision_wire_shape_is_stable() {
        let snap = snapshot_with("src/hot.rs");
        let a = focus("agent-a", &snap, None);
        let b = focus("agent-b", &snap, None);
        let json = serde_json::to_value(decide(&a, &b)).expect("serialises");
        assert_eq!(json["decision"], "serialize");
        assert_eq!(json["holder"], "agent-a");
        assert_eq!(json["target"], "src/hot.rs");
    }
}
