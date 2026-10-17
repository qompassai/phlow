//! The trajectory event log vocabulary and the actions the gate
//! decides on.
//!
//! Plain words: a trajectory is an append-only list of these events.
//! The trajectory's label is *defined* as the fold of the log (see
//! [`crate::trajectory`]) — there is no separately stored label that
//! could drift, and no event kind narrows less than the data it
//! records. Label-carrying events name their contribution as a
//! [`JoinSpec`]: either a policy delta in audience tokens, or a
//! concrete snapshot captured at a handoff.
//!
//! Approval events deserve a note: an [`Event::ApprovalGranted`]
//! carries the digest of the one action it approves. The gate counts
//! an approval as consumed by the first later allowed action with
//! the same digest, so an approval is exactly single-use — replaying
//! it, or replaying the action under a mutated log, fails the digest
//! check.

use serde::{Deserialize, Serialize};

use crate::label::{LabelDelta, LabelSnapshot};

/// How an event's label contribution is recorded.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum JoinSpec {
    /// A policy delta (audience tokens + optional trust rank name).
    Delta(LabelDelta),
    /// A concrete label captured at a handoff boundary.
    Snapshot(LabelSnapshot),
}

/// One entry in a trajectory's append-only log.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Event {
    /// The trajectory began. `start` is `Some` only for sub-agent
    /// trajectories, whose start label is the inherited join computed
    /// at spawn; `None` means "the policy start label". Approval
    /// events are never part of a child's copied context — a child
    /// log starts here and only here.
    SessionStarted {
        /// The spawning trajectory's agent id, if any.
        parent: Option<String>,
        /// The inherited start label, for sub-agents.
        start: Option<LabelSnapshot>,
    },
    /// The agent read data from a named source; the source's delta
    /// was joined into the label.
    DataRead {
        /// The policy source name.
        source: String,
        /// The contribution that was joined.
        delta: LabelDelta,
    },
    /// A message arrived from another agent or an external sender;
    /// the sender-side contribution was joined into the label.
    MessageReceived {
        /// Sender name (agent id or external agent name).
        from: String,
        /// The contribution that was joined.
        join: JoinSpec,
    },
    /// A tool call was proposed to the gate.
    ToolProposed {
        /// Tool name.
        tool: String,
        /// Digest of the proposed action.
        digest: String,
    },
    /// The gate allowed a tool call.
    ToolAllowed {
        /// Tool name.
        tool: String,
        /// Digest of the allowed action.
        digest: String,
    },
    /// The gate blocked a tool call; the reasons are reproducible by
    /// re-running the gate over the log prefix, so they are not
    /// stored here.
    ToolBlocked {
        /// Tool name.
        tool: String,
        /// Digest of the blocked action.
        digest: String,
    },
    /// A tool returned; its contract delta was joined and its
    /// declared effects recorded. Carrying the action digest is
    /// what makes approvals single-use: an approval is consumed by
    /// the first later `ToolResult` with the same digest.
    ToolResult {
        /// Tool name.
        tool: String,
        /// Digest of the action that produced this result.
        digest: String,
        /// The result delta that was joined.
        delta: LabelDelta,
        /// Effects recorded by this result.
        effects: Vec<String>,
    },
    /// A capability effect was recorded outside a tool result.
    EffectRecorded {
        /// The effect string.
        effect: String,
    },
    /// A message send was proposed to the gate.
    MessageProposed {
        /// Destination name.
        destination: String,
        /// Digest of the proposed action.
        digest: String,
    },
    /// The gate allowed a message send.
    MessageAllowed {
        /// Destination name.
        destination: String,
        /// Digest of the allowed action.
        digest: String,
    },
    /// The gate blocked a message send.
    MessageBlocked {
        /// Destination name.
        destination: String,
        /// Digest of the blocked action.
        digest: String,
    },
    /// The session published a focus on the shared attention board.
    FocusPublished {
        /// The board sequence the publication used.
        seq: u64,
    },
    /// The session switched focus from one target to another because
    /// of attention state (the explicit, logged dynamic-switch
    /// event).
    FocusSwitched {
        /// The target given up.
        from: String,
        /// The target taken up.
        to: String,
        /// The board agent holding the given-up target, if the
        /// switch was contention-driven.
        because: Option<String>,
    },
    /// A sub-agent was spawned. Informational in the parent log; the
    /// child's own log carries the inherited start label.
    SubagentSpawned {
        /// The child's agent id.
        child: String,
        /// The start label the child inherited.
        start: LabelSnapshot,
    },
    /// A sub-agent's result was accepted into this trajectory; the
    /// child's end label was joined (this is the join that defeats
    /// label laundering through a child).
    SubagentResult {
        /// The child's agent id.
        child: String,
        /// The child's end label, joined on acceptance.
        label: LabelSnapshot,
    },
    /// An authority approved one action, identified by digest.
    ApprovalGranted {
        /// The approving authority's name.
        authority: String,
        /// Digest of the one approved action.
        digest: String,
        /// Attention marks this approval clears.
        marks: Vec<String>,
    },
    /// A sanitizer transformed a payload; the sanitized output's
    /// declared delta was joined.
    Sanitized {
        /// The sanitizer's name.
        sanitizer: String,
        /// The result delta that was joined.
        result_delta: LabelDelta,
    },
}

/// An action presented to the gate for a decision.
///
/// The action is data, not behaviour: the gate reads the policy and
/// the log, folds the label, and decides. Tool arguments ride along
/// because `requires.audience_contains` may resolve `$argument`
/// placeholders from them — the arguments are *examined*, never
/// executed, by the gate.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum ProposedAction {
    /// Call a tool with JSON arguments.
    ToolCall {
        /// Tool name; must have a contract in policy.
        tool: String,
        /// The call arguments.
        args: serde_json::Value,
    },
    /// Send a message to a named destination. `payload_label` is the
    /// label of the content being sent when it differs from the
    /// trajectory's own label (e.g. forwarding a result); the
    /// effective send label is the join of the two.
    Message {
        /// Destination name; must be declared in policy.
        destination: String,
        /// The attached content's label, if separately known.
        payload_label: Option<LabelSnapshot>,
    },
}

impl ProposedAction {
    /// A canonical, digest-sized summary of the action: kind + name.
    /// (The full digest input is the canonical JSON serialisation,
    /// computed in [`crate::gate::action_digest`].)
    pub fn kind_and_name(&self) -> (&'static str, &str) {
        match self {
            ProposedAction::ToolCall { tool, .. } => ("tool", tool),
            ProposedAction::Message { destination, .. } => ("message", destination),
        }
    }
}
