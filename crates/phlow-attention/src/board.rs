//! The focus board: the rendezvous where agents advertise attention.
//!
//! Plain words: a board is a bounded in-memory table of the latest
//! focus per agent. Transports (the phlow MCP server, periscope, an
//! A2A relay) own one board per coordination session and pass the
//! *authenticated caller identity* into [`FocusBoard::publish`] —
//! identity is never taken from the focus payload itself, which is
//! what makes focus spoofing a rejected publish instead of a silent
//! overwrite of a partner's advertised attention.
//!
//! The board is deliberately not a database: foci are perishable
//! coordination signals, valid only inside their snapshot frame. When
//! the workspace moves on, hashes diverge and every decision fails
//! closed until agents re-derive from a fresh snapshot.

use std::collections::HashMap;

use crate::BOARD_AGENTS_MAX;
use crate::alignment::{Alignment, Decision, alignment, decide};
use crate::error::AttentionError;
use crate::focus::Focus;

/// The latest advertised focus per agent, with publish-time guards.
#[derive(Debug, Default)]
pub struct FocusBoard {
    foci: HashMap<String, Focus>,
}

impl FocusBoard {
    /// An empty board.
    pub fn new() -> FocusBoard {
        FocusBoard {
            foci: HashMap::new(),
        }
    }

    /// Number of agents currently tracked.
    pub fn len(&self) -> usize {
        self.foci.len()
    }

    /// Whether no agent is tracked.
    pub fn is_empty(&self) -> bool {
        self.foci.is_empty()
    }

    /// Publish `focus` on behalf of the authenticated `caller_id`.
    ///
    /// Rejects, in order: a payload whose agent id differs from the
    /// caller (spoofing), a structurally invalid focus, a full board
    /// seeing a brand-new agent (flooding), and a sequence that does
    /// not advance the agent's previous one (stale replay).
    /// The agent ids currently on the board, sorted for deterministic
    /// iteration by gates and tools.
    pub fn agent_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.foci.keys().cloned().collect();
        ids.sort();
        ids
    }

    pub fn publish(&mut self, caller_id: &str, focus: Focus) -> Result<(), AttentionError> {
        if focus.agent_id != caller_id {
            return Err(AttentionError::IdentityMismatch);
        }
        focus.validate()?;
        match self.foci.get(caller_id) {
            Some(previous) => {
                if focus.seq <= previous.seq {
                    return Err(AttentionError::StaleSequence {
                        agent: caller_id.to_owned(),
                        got: focus.seq,
                        last: previous.seq,
                    });
                }
            }
            None => {
                if self.foci.len() >= BOARD_AGENTS_MAX {
                    return Err(AttentionError::BoardFull);
                }
            }
        }
        self.foci.insert(caller_id.to_owned(), focus);
        Ok(())
    }

    /// The latest focus of one agent, if published.
    pub fn get(&self, agent_id: &str) -> Option<&Focus> {
        self.foci.get(agent_id)
    }

    /// Forget one agent (session end). Forgetting is explicit: the
    /// board never evicts silently.
    pub fn remove(&mut self, agent_id: &str) -> Option<Focus> {
        self.foci.remove(agent_id)
    }

    /// Alignment between two tracked agents' foci, if both published.
    pub fn alignment_between(&self, a: &str, b: &str) -> Option<Result<Alignment, AttentionError>> {
        let (fa, fb) = (self.foci.get(a)?, self.foci.get(b)?);
        Some(alignment(fa, fb))
    }

    /// The coordination decision for two tracked agents, if both
    /// published.
    pub fn decide_between(&self, a: &str, b: &str) -> Option<Decision> {
        let (fa, fb) = (self.foci.get(a)?, self.foci.get(b)?);
        Some(decide(fa, fb))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saliency::SaliencyInput;
    use crate::snapshot::{Snapshot, TargetId};

    fn target(raw: &str) -> TargetId {
        TargetId::new(raw).expect("test target id is valid")
    }

    fn snapshot() -> Snapshot {
        let mut named = SaliencyInput::quiet();
        named.named_in_task = true;
        Snapshot::build(vec![
            (target("src/hot.rs"), named),
            (target("src/other.rs"), SaliencyInput::quiet()),
        ])
        .expect("builds")
    }

    fn focus(agent: &str, snap: &Snapshot, seq: u64) -> Focus {
        Focus::derive(agent, snap, seq, None).expect("derives")
    }

    #[test]
    fn publish_and_read_back() {
        let snap = snapshot();
        let mut board = FocusBoard::new();
        board
            .publish("agent-a", focus("agent-a", &snap, 1))
            .expect("publishes");
        assert_eq!(board.len(), 1);
        assert_eq!(board.get("agent-a").map(|f| f.seq), Some(1));
        assert!(board.get("agent-b").is_none());
    }

    #[test]
    fn spoofed_publish_is_rejected() {
        // Adversarial: the payload claims agent-b, the authenticated
        // caller is agent-a. Identity comes from the transport, so
        // this can never overwrite agent-b's advertised focus.
        let snap = snapshot();
        let mut board = FocusBoard::new();
        let forged = focus("agent-b", &snap, 1);
        let err = board
            .publish("agent-a", forged)
            .expect_err("spoofed publish must fail");
        assert_eq!(err, AttentionError::IdentityMismatch);
        assert!(board.get("agent-b").is_none());
    }

    #[test]
    fn stale_sequence_replay_is_rejected() {
        // Adversarial: replaying an older focus (or re-sending the
        // same one) must not roll the board back to a stale frame of
        // attention.
        let snap = snapshot();
        let mut board = FocusBoard::new();
        board
            .publish("agent-a", focus("agent-a", &snap, 5))
            .expect("publishes");
        let err = board
            .publish("agent-a", focus("agent-a", &snap, 5))
            .expect_err("equal seq is stale");
        assert!(matches!(err, AttentionError::StaleSequence { .. }));
        let err = board
            .publish("agent-a", focus("agent-a", &snap, 3))
            .expect_err("older seq is stale");
        assert!(matches!(err, AttentionError::StaleSequence { .. }));
        board
            .publish("agent-a", focus("agent-a", &snap, 6))
            .expect("advancing seq publishes");
    }

    #[test]
    fn flooded_focus_is_rejected_at_publish() {
        // Adversarial: an oversized target list is refused before it
        // can consume board memory or attention bandwidth.
        let snap = snapshot();
        let mut board = FocusBoard::new();
        let mut fat = focus("agent-a", &snap, 1);
        while fat.targets.len() <= crate::FOCUS_TARGETS_MAX {
            fat.targets.push(target("src/extra.rs"));
        }
        assert!(board.publish("agent-a", fat).is_err());
        assert!(board.is_empty());
    }

    #[test]
    fn board_full_rejects_new_agents_but_not_updates() {
        let snap = snapshot();
        let mut board = FocusBoard::new();
        for i in 0..BOARD_AGENTS_MAX {
            let agent = format!("agent-{i:03}");
            board
                .publish(&agent, focus(&agent, &snap, 1))
                .expect("publishes within the cap");
        }
        let err = board
            .publish("agent-zzz", focus("agent-zzz", &snap, 1))
            .expect_err("65th agent must fail");
        assert_eq!(err, AttentionError::BoardFull);
        // A tracked agent can still update its focus.
        board
            .publish("agent-000", focus("agent-000", &snap, 2))
            .expect("update from a tracked agent still works");
    }

    #[test]
    fn decide_between_uses_the_board() {
        let snap = snapshot();
        let mut board = FocusBoard::new();
        board
            .publish("agent-a", focus("agent-a", &snap, 1))
            .expect("publishes");
        assert!(board.decide_between("agent-a", "agent-b").is_none());
        board
            .publish("agent-b", focus("agent-b", &snap, 1))
            .expect("publishes");
        assert_eq!(
            board.decide_between("agent-a", "agent-b"),
            Some(Decision::Serialize {
                holder: "agent-a".to_owned(),
                target: target("src/hot.rs"),
            })
        );
    }
}
