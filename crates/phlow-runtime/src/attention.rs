//! The joint-attention dispatch gate.
//!
//! When `[attention] enabled` is set in the operator config and a gate
//! is installed, [`Runtime::run`](crate::runtime::Runtime) consults the
//! coordination board after planning and before the first coder cycle:
//! it snapshots the workspace's saliency, derives and publishes its own
//! focus, and applies the `phlow-attention` coordination rule against
//! every partner focus on the same frame. Losing a contended top target
//! defers the run (the report says who holds the target) instead of two
//! unfamiliar agents editing the same file in parallel.
//!
//! Fail-closed throughout: a focus that cannot be derived or published
//! defers the run with the reason recorded; a partner on a different
//! frame is recorded as uncoordinated, never silently aligned with.

use phlow_attention::{Decision, Focus, FocusBoard, SNAPSHOT_TARGETS_MAX, SaliencyInput, Snapshot};
use phlow_workspace::Workspace;
use serde_json::{Map, Value};
use std::sync::{Arc, Mutex};

/// The outcome of one gate consultation.
#[derive(Debug)]
pub enum GateVerdict {
    /// Dispatch may proceed; `record` goes into the run report.
    Proceed {
        /// The `attention` object recorded in the run report.
        record: Value,
    },
    /// Dispatch must not proceed this run.
    Defer {
        /// Who holds the contended target (the gate's own agent id when
        /// the failure is local: derive/publish failures fail closed).
        holder: String,
        /// Machine-stable reason: `held_by_partner`, `publish_failed`,
        /// or `derive_failed`.
        reason: &'static str,
        /// The `attention` object recorded in the run report.
        record: Value,
        /// The contended target, when one exists.
        target: String,
    },
}

/// One agent's handle on the shared coordination board.
#[derive(Debug)]
pub struct AttentionGate {
    agent_id: String,
    board: Arc<Mutex<FocusBoard>>,
    next_seq: Mutex<u64>,
}

impl AttentionGate {
    /// Create a gate for `agent_id` over a shared board. The board is
    /// typically the MCP server's, shared via
    /// `McpServer::attention_board`, so tool-published partner foci and
    /// the gate see one coordination state.
    pub fn new(agent_id: &str, board: Arc<Mutex<FocusBoard>>) -> AttentionGate {
        AttentionGate {
            agent_id: agent_id.to_owned(),
            board,
            next_seq: Mutex::new(0),
        }
    }

    /// Create a gate over a fresh board, for single-process embeddings
    /// (the CLI): the MCP server adopts this gate's board via
    /// [`AttentionGate::board`] so both surfaces share one state.
    pub fn with_fresh_board(agent_id: &str) -> AttentionGate {
        AttentionGate::new(agent_id, Arc::new(Mutex::new(FocusBoard::new())))
    }

    /// The shared board this gate consults.
    pub fn board(&self) -> Arc<Mutex<FocusBoard>> {
        Arc::clone(&self.board)
    }

    /// Publish this agent's focus for `snapshot` and decide against
    /// every partner on the board. Deterministic: partners are visited
    /// in sorted id order and the smallest (holder, target) deferral
    /// wins, so two gates consulting the same board state agree.
    pub fn consult(&self, snapshot: &Snapshot) -> GateVerdict {
        let mut record = Map::new();
        record.insert(
            "snapshot_hash".to_owned(),
            Value::String(snapshot.hash().to_owned()),
        );
        let seq = {
            let mut next = self.next_seq.lock().expect("gate sequence lock");
            *next += 1;
            *next
        };
        let own = match Focus::derive(&self.agent_id, snapshot, seq, None) {
            Ok(focus) => focus,
            Err(error) => {
                record.insert("error".to_owned(), Value::String(error.to_string()));
                return GateVerdict::Defer {
                    holder: self.agent_id.clone(),
                    reason: "derive_failed",
                    record: Value::Object(record),
                    target: String::new(),
                };
            }
        };
        record.insert(
            "focus".to_owned(),
            own.targets
                .iter()
                .map(|target| Value::String(target.to_string()))
                .collect(),
        );
        let mut board = self.board.lock().expect("attention board lock");
        if let Err(error) = board.publish(&self.agent_id, own.clone()) {
            record.insert("error".to_owned(), Value::String(error.to_string()));
            return GateVerdict::Defer {
                holder: self.agent_id.clone(),
                reason: "publish_failed",
                record: Value::Object(record),
                target: String::new(),
            };
        }
        let mut decisions = Vec::new();
        let mut deferral: Option<(String, String)> = None;
        for partner in board.agent_ids() {
            if partner == self.agent_id {
                continue;
            }
            let Some(decision) = board.decide_between(&self.agent_id, &partner) else {
                continue;
            };
            if let Decision::Serialize { holder, target } = &decision
                && *holder != self.agent_id
            {
                let candidate = (holder.clone(), target.to_string());
                if deferral.as_ref().is_none_or(|held| candidate < *held) {
                    deferral = Some(candidate);
                }
            }
            decisions.push(serde_json::json!({
                "agent": partner,
                "decision": serde_json::to_value(&decision)
                    .expect("decision serialization cannot fail"),
            }));
        }
        record.insert("decisions".to_owned(), Value::Array(decisions));
        match deferral {
            Some((holder, target)) => GateVerdict::Defer {
                holder,
                reason: "held_by_partner",
                record: Value::Object(record),
                target,
            },
            None => GateVerdict::Proceed {
                record: Value::Object(record),
            },
        }
    }
}

/// Build the saliency snapshot for one run from the workspace itself.
///
/// The signal available at this layer is task/plan naming: a file the
/// task text or the planner's summary names is salient; everything else
/// is quiet. Richer signals (diagnostics, SCIP centrality, recency)
/// enter through the same `SaliencyInput` shape from producers that
/// have them — periscope's editor state, a SCIP index pass — and change
/// only the snapshot's contents, never the protocol.
pub fn snapshot_from_workspace(workspace: &Workspace, task: &str, plan: &str) -> Snapshot {
    let files = workspace
        .list(".")
        .map(|listed| listed.files)
        .unwrap_or_default();
    let mut inputs = Vec::with_capacity(files.len().min(SNAPSHOT_TARGETS_MAX));
    for path in files {
        if inputs.len() >= SNAPSHOT_TARGETS_MAX {
            break;
        }
        let Ok(id) = phlow_attention::TargetId::new(&path) else {
            continue;
        };
        let named = task.contains(&path) || plan.contains(&path);
        inputs.push((
            id,
            SaliencyInput {
                named_in_task: named,
                ..SaliencyInput::quiet()
            },
        ));
    }
    Snapshot::build(inputs).unwrap_or_else(|_| Snapshot::build(Vec::new()).expect("empty builds"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use phlow_attention::{SaliencyInput, TargetId};
    use std::collections::BTreeSet;

    fn snapshot() -> Snapshot {
        Snapshot::build(vec![
            (
                TargetId::new("src/a.rs").expect("id"),
                SaliencyInput {
                    named_in_task: true,
                    ..SaliencyInput::quiet()
                },
            ),
            (
                TargetId::new("src/b.rs").expect("id"),
                SaliencyInput::quiet(),
            ),
        ])
        .expect("snapshot builds")
    }

    fn publish(board: &Arc<Mutex<FocusBoard>>, agent: &str, snapshot: &Snapshot, only: &[&str]) {
        let filter: Option<BTreeSet<TargetId>> = if only.is_empty() {
            None
        } else {
            Some(
                only.iter()
                    .map(|raw| TargetId::new(raw).expect("id"))
                    .collect(),
            )
        };
        let focus = Focus::derive(agent, snapshot, 1, filter.as_ref()).expect("focus derives");
        board
            .lock()
            .expect("board lock")
            .publish(agent, focus)
            .expect("publish succeeds");
    }

    #[test]
    fn empty_board_proceeds_and_publishes_own_focus() {
        let board = Arc::new(Mutex::new(FocusBoard::new()));
        let gate = AttentionGate::new("alice", Arc::clone(&board));
        let snapshot = snapshot();
        match gate.consult(&snapshot) {
            GateVerdict::Proceed { record } => {
                assert_eq!(record["snapshot_hash"], snapshot.hash());
            }
            other => panic!("empty board must proceed, got {other:?}"),
        }
        assert!(
            board.lock().expect("board lock").get("alice").is_some(),
            "the gate publishes its own focus"
        );
    }

    #[test]
    fn partner_holding_the_top_target_defers() {
        let board = Arc::new(Mutex::new(FocusBoard::new()));
        let snapshot = snapshot();
        // Bob's full focus tops at src/a.rs too; "alice" < "bob", so
        // alice would hold it — publish carol instead: "alice" < "carol"
        // still makes alice the holder. Use agent "aaron" as the
        // consulter so the partner (bob) is the deterministic holder.
        publish(&board, "bob", &snapshot, &[]);
        // Holder selection is the lexicographically smaller agent id:
        // "zeke" loses the contended top to "bob" and must defer, while
        // "aaron" would hold it against "bob" and proceed.
        let gate = AttentionGate::new("zeke", Arc::clone(&board));
        match gate.consult(&snapshot) {
            GateVerdict::Defer {
                holder,
                reason,
                target,
                ..
            } => {
                assert_eq!(holder, "bob");
                assert_eq!(reason, "held_by_partner");
                assert_eq!(target, "src/a.rs");
            }
            other => panic!("contended top must defer, got {other:?}"),
        }
        // And the smaller-id agent proceeds against the same board.
        match AttentionGate::new("aaron", board).consult(&snapshot) {
            GateVerdict::Proceed { .. } => {}
            other => panic!("deterministic holder must proceed, got {other:?}"),
        }
    }

    #[test]
    fn disjoint_partner_proceeds() {
        let board = Arc::new(Mutex::new(FocusBoard::new()));
        let snapshot = snapshot();
        publish(&board, "bob", &snapshot, &["src/b.rs"]);
        let gate = AttentionGate::new("alice", Arc::clone(&board));
        match gate.consult(&snapshot) {
            GateVerdict::Proceed { record } => {
                assert_eq!(record["decisions"][0]["agent"], "bob");
            }
            other => panic!("disjoint foci must proceed, got {other:?}"),
        }
    }

    #[test]
    fn workspace_snapshot_names_task_files() {
        let dir =
            std::env::temp_dir().join(format!("phlow-runtime-attention-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).expect("test dirs");
        std::fs::write(dir.join("src/a.rs"), "fn a() {}\n").expect("write");
        std::fs::write(dir.join("src/b.rs"), "fn b() {}\n").expect("write");
        let workspace = Workspace::open(&dir, false, &[]).expect("open workspace");
        let snapshot = snapshot_from_workspace(&workspace, "fix src/a.rs please", "");
        let ranked = snapshot.ranked();
        assert_eq!(ranked.len(), 2);
        assert_eq!(
            ranked[0].0.to_string(),
            "src/a.rs",
            "named file ranks first"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
