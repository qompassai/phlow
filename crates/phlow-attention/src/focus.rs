//! A focus: one agent's advertised attention, derived from a snapshot.
//!
//! Plain words: a focus is the analog of the partner attention map in
//! the paper, made explicit. It says "computed from snapshot H, these
//! are the targets I rank highest, in order". The ordering is never
//! agent-chosen — it is the snapshot's own ranking, optionally filtered
//! to the targets the agent actually intends to work on. That is what
//! makes two strangers' foci comparable without any shared convention:
//! neither side got to invent the order.
//!
//! The JSON shape of this struct is the wire format on both MCP and
//! A2A transports.

use serde::{Deserialize, Serialize};

use crate::error::AttentionError;
use crate::snapshot::{Snapshot, TargetId};
use crate::{AGENT_ID_MAX_BYTES, FOCUS_TARGETS_MAX, TARGET_ID_MAX_BYTES};

/// Validate a raw agent identifier at a trust boundary.
pub fn validate_agent_id(raw: &str) -> Result<(), AttentionError> {
    if raw.is_empty() || raw.len() > AGENT_ID_MAX_BYTES || raw.chars().any(char::is_control) {
        return Err(AttentionError::InvalidAgentId);
    }
    Ok(())
}

/// One agent's advertised focus within one snapshot frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Focus {
    /// Who is attending. Identity is asserted by the transport at the
    /// board; this field is the claim the board checks it against.
    pub agent_id: String,
    /// Hash of the snapshot this focus was computed from.
    pub snapshot_hash: String,
    /// Per-agent monotone sequence. A focus that does not advance the
    /// sequence is stale or replayed and must be refused.
    pub seq: u64,
    /// Attended targets, most salient first, at most
    /// [`FOCUS_TARGETS_MAX`].
    pub targets: Vec<TargetId>,
}

impl Focus {
    /// Derive a focus from a snapshot: the snapshot's own top targets.
    ///
    /// `filter` restricts the ranking to a subset (the targets the
    /// agent intends to work on) without ever reordering it.
    pub fn derive(
        agent_id: &str,
        snapshot: &Snapshot,
        seq: u64,
        filter: Option<&std::collections::BTreeSet<TargetId>>,
    ) -> Result<Focus, AttentionError> {
        validate_agent_id(agent_id)?;
        let mut targets = Vec::new();
        for (id, _) in snapshot.ranked() {
            if targets.len() >= FOCUS_TARGETS_MAX {
                break;
            }
            if let Some(allowed) = filter
                && !allowed.contains(id)
            {
                continue;
            }
            targets.push(id.clone());
        }
        Ok(Focus {
            agent_id: agent_id.to_owned(),
            snapshot_hash: snapshot.hash().to_owned(),
            seq,
            targets,
        })
    }

    /// Structural validation for a focus that arrived over a
    /// transport (i.e. was not derived locally). The board and the
    /// alignment/decide entry points run this before trusting it.
    pub fn validate(&self) -> Result<(), AttentionError> {
        validate_agent_id(&self.agent_id)
            .map_err(|_| AttentionError::InvalidFocus("agent id".to_owned()))?;
        if self.targets.len() > FOCUS_TARGETS_MAX {
            return Err(AttentionError::TooManyTargets {
                count: self.targets.len(),
                max: FOCUS_TARGETS_MAX,
            });
        }
        for target in &self.targets {
            // Re-run the identifier rules: a deserialised TargetId is
            // only as trustworthy as its constructor, and serde does
            // not call it.
            let raw = target.as_str();
            if raw.is_empty()
                || raw.len() > TARGET_ID_MAX_BYTES
                || raw.chars().any(char::is_control)
            {
                return Err(AttentionError::InvalidFocus(format!(
                    "target id {raw:?} breaks the identifier rules"
                )));
            }
        }
        let mut sorted = self.targets.clone();
        sorted.sort();
        sorted.dedup();
        if sorted.len() != self.targets.len() {
            return Err(AttentionError::InvalidFocus(
                "duplicate targets in one focus".to_owned(),
            ));
        }
        Ok(())
    }

    /// The agent's primary target of attention, if any.
    pub fn top(&self) -> Option<&TargetId> {
        self.targets.first()
    }

    /// Whether this focus attends to `target` at any rank.
    pub fn attends(&self, target: &TargetId) -> bool {
        self.targets.contains(target)
    }
}

// TargetId needs serde for the Focus wire shape. The deserialiser
// funnels through the same byte bounds as the constructor; full
// re-validation happens in Focus::validate at the trust boundary.
impl Serialize for TargetId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TargetId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<TargetId, D::Error> {
        let raw = String::deserialize(deserializer)?;
        if raw.len() > TARGET_ID_MAX_BYTES {
            return Err(serde::de::Error::custom("target id over the byte bound"));
        }
        // Construct without the control-character rule here: rejecting
        // at Focus::validate keeps one validation site for semantics,
        // while the length bound above keeps allocations honest.
        Ok(TargetId::from_wire(raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saliency::SaliencyInput;

    fn target(raw: &str) -> TargetId {
        TargetId::new(raw).expect("test target id is valid")
    }

    fn snapshot() -> Snapshot {
        let mut named = SaliencyInput::quiet();
        named.named_in_task = true;
        let mut central = SaliencyInput::quiet();
        central.graph_centrality = 0.9;
        Snapshot::build(vec![
            (target("src/hot.rs"), named),
            (target("src/hub.rs"), central),
            (target("src/cold.rs"), SaliencyInput::quiet()),
        ])
        .expect("builds")
    }

    #[test]
    fn derive_takes_the_snapshot_ranking_verbatim() {
        let snap = snapshot();
        let focus = Focus::derive("agent-a", &snap, 1, None).expect("derives");
        let order: Vec<&str> = focus.targets.iter().map(TargetId::as_str).collect();
        assert_eq!(order, vec!["src/hot.rs", "src/hub.rs", "src/cold.rs"]);
        assert_eq!(focus.snapshot_hash, snap.hash());
        assert_eq!(focus.top(), Some(&target("src/hot.rs")));
    }

    #[test]
    fn derive_filter_restricts_without_reordering() {
        let snap = snapshot();
        let allowed: std::collections::BTreeSet<TargetId> =
            [target("src/cold.rs"), target("src/hot.rs")]
                .into_iter()
                .collect();
        let focus = Focus::derive("agent-a", &snap, 1, Some(&allowed)).expect("derives");
        let order: Vec<&str> = focus.targets.iter().map(TargetId::as_str).collect();
        assert_eq!(order, vec!["src/hot.rs", "src/cold.rs"]);
    }

    #[test]
    fn derive_is_identical_for_independent_agents() {
        // The protocol's core claim in miniature: two agents that have
        // never met derive the same focus from the same snapshot.
        let snap = snapshot();
        let a = Focus::derive("agent-a", &snap, 1, None).expect("derives");
        let b = Focus::derive("agent-b", &snap, 1, None).expect("derives");
        assert_eq!(a.targets, b.targets);
    }

    #[test]
    fn validate_rejects_an_oversized_focus() {
        let mut focus = Focus::derive("agent-a", &snapshot(), 1, None).expect("derives");
        while focus.targets.len() <= FOCUS_TARGETS_MAX {
            focus.targets.push(target("src/extra.rs"));
        }
        let err = focus.validate().expect_err("oversized focus must fail");
        assert!(matches!(err, AttentionError::TooManyTargets { .. }));
    }

    #[test]
    fn validate_rejects_duplicate_targets() {
        let mut focus = Focus::derive("agent-a", &snapshot(), 1, None).expect("derives");
        focus.targets.push(target("src/hot.rs"));
        let err = focus.validate().expect_err("duplicate must fail");
        assert!(matches!(err, AttentionError::InvalidFocus(_)));
    }

    /// The shared cross-implementation vector: periscope's vendored
    /// protocol core carries this exact document in its tests
    /// (`SHARED_FOCUS_JSON` in periscope's `src/attention.rs`); both
    /// sides must agree on the wire shape byte for byte.
    #[test]
    fn focus_json_shape_is_stable() {
        let focus = Focus {
            agent_id: "alice".to_owned(),
            snapshot_hash: "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
                .to_owned(),
            seq: 3,
            targets: vec![
                TargetId::new("src/a.rs").expect("id"),
                TargetId::new("src/b.rs").expect("id"),
            ],
        };
        let json = serde_json::to_string(&focus).expect("serialises");
        assert_eq!(
            json,
            r#"{"agent_id":"alice","snapshot_hash":"9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08","seq":3,"targets":["src/a.rs","src/b.rs"]}"#
        );
    }

    #[test]
    fn wire_shape_round_trips_through_json() {
        let focus = Focus::derive("agent-a", &snapshot(), 7, None).expect("derives");
        let json = serde_json::to_string(&focus).expect("serialises");
        let back: Focus = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(focus, back);
        back.validate().expect("round-tripped focus stays valid");
    }

    #[test]
    fn agent_id_rules_are_enforced() {
        let snap = snapshot();
        assert!(Focus::derive("", &snap, 1, None).is_err());
        assert!(Focus::derive(&"a".repeat(AGENT_ID_MAX_BYTES + 1), &snap, 1, None).is_err());
        assert!(Focus::derive("agent-a", &snap, 1, None).is_ok());
    }
}
