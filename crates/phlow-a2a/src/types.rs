//! The A2A 1.0 wire model: Agent Card, Task, Message, Part,
//! Artifact, request parameters, and streaming events.
//!
//! Serialisation follows the specification's canonical data model
//! (ProtoJSON naming, §5 of the spec): enum values carry their proto
//! names (`TASK_STATE_WORKING`, `ROLE_USER`). Deserialisation is
//! deliberately liberal — the bare spellings used by the 0.2/0.3-era
//! bindings (`working`, `user`) are accepted — because interop is
//! the point of the crate; emission is always the 1.0 form.
//!
//! Field-level bounds are *not* enforced by (de)serialisation; the
//! server and the store validate counts and sizes at their
//! boundaries via [`Message::validate`].

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{PART_TEXT_BYTES_MAX, PARTS_MAX};

// ---------------------------------------------------------------------
// Enums with ProtoJSON names and liberal parsing.

/// The task lifecycle states (spec §4.1.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskState {
    /// The task was submitted but has not begun.
    Submitted,
    /// The agent is actively working.
    Working,
    /// Terminal: finished successfully.
    Completed,
    /// Terminal: finished with an error.
    Failed,
    /// Terminal: cancelled by the requester.
    Canceled,
    /// Terminal: refused (e.g. no executor capacity).
    Rejected,
    /// Paused: the agent needs more input. Non-terminal.
    InputRequired,
    /// Paused: the agent needs authentication. Non-terminal.
    AuthRequired,
    /// State unspecified (never emitted; parseable for robustness).
    Unspecified,
}

impl TaskState {
    /// Is this a terminal state? Terminal tasks accept no further
    /// messages and no transitions.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskState::Completed | TaskState::Failed | TaskState::Canceled | TaskState::Rejected
        )
    }

    /// The ProtoJSON name.
    pub fn as_proto_name(self) -> &'static str {
        match self {
            TaskState::Submitted => "TASK_STATE_SUBMITTED",
            TaskState::Working => "TASK_STATE_WORKING",
            TaskState::Completed => "TASK_STATE_COMPLETED",
            TaskState::Failed => "TASK_STATE_FAILED",
            TaskState::Canceled => "TASK_STATE_CANCELED",
            TaskState::Rejected => "TASK_STATE_REJECTED",
            TaskState::InputRequired => "TASK_STATE_INPUT_REQUIRED",
            TaskState::AuthRequired => "TASK_STATE_AUTH_REQUIRED",
            TaskState::Unspecified => "TASK_STATE_UNSPECIFIED",
        }
    }

    /// Parse a state name, accepting both the 1.0 ProtoJSON form and
    /// the bare kebab-case spellings of earlier bindings.
    pub fn from_name(name: &str) -> Option<TaskState> {
        match name {
            "TASK_STATE_SUBMITTED" | "submitted" => Some(TaskState::Submitted),
            "TASK_STATE_WORKING" | "working" => Some(TaskState::Working),
            "TASK_STATE_COMPLETED" | "completed" => Some(TaskState::Completed),
            "TASK_STATE_FAILED" | "failed" => Some(TaskState::Failed),
            "TASK_STATE_CANCELED" | "canceled" | "cancelled" => Some(TaskState::Canceled),
            "TASK_STATE_REJECTED" | "rejected" => Some(TaskState::Rejected),
            "TASK_STATE_INPUT_REQUIRED" | "input-required" => Some(TaskState::InputRequired),
            "TASK_STATE_AUTH_REQUIRED" | "auth-required" => Some(TaskState::AuthRequired),
            "TASK_STATE_UNSPECIFIED" | "unspecified" => Some(TaskState::Unspecified),
            _ => None,
        }
    }
}

impl Serialize for TaskState {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_proto_name())
    }
}

impl<'de> Deserialize<'de> for TaskState {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<TaskState, D::Error> {
        let name = String::deserialize(deserializer)?;
        TaskState::from_name(&name)
            .ok_or_else(|| serde::de::Error::unknown_variant(&name, &["TASK_STATE_*"]))
    }
}

/// A message's sender role.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    /// From the requesting client or agent.
    User,
    /// From the serving agent.
    Agent,
}

impl Serialize for Role {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Role::User => "ROLE_USER",
            Role::Agent => "ROLE_AGENT",
        })
    }
}

impl<'de> Deserialize<'de> for Role {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Role, D::Error> {
        let name = String::deserialize(deserializer)?;
        match name.as_str() {
            "ROLE_USER" | "user" => Ok(Role::User),
            "ROLE_AGENT" | "agent" => Ok(Role::Agent),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["ROLE_USER", "ROLE_AGENT"],
            )),
        }
    }
}

// ---------------------------------------------------------------------
// Core model.

/// A single message in either direction (spec §4.1.1).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Unique identifier for this message.
    pub message_id: String,
    /// Groups messages into one interaction; server-assigned when a
    /// message starts a new task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    /// The task this message belongs to, when it belongs to one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Who sent it.
    pub role: Role,
    /// The content, as parts.
    pub parts: Vec<Part>,
    /// Continuation: tasks this message relates to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_task_ids: Vec<String>,
    /// Extension URIs active for this message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    /// Opaque metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl Message {
    /// A user-role message with a single text part.
    pub fn text(message_id: &str, text: &str) -> Message {
        Message {
            message_id: message_id.to_string(),
            context_id: None,
            task_id: None,
            role: Role::User,
            parts: vec![Part::text(text)],
            reference_task_ids: Vec::new(),
            extensions: Vec::new(),
            metadata: None,
        }
    }

    /// An agent-role message with a single text part.
    pub fn agent_text(message_id: &str, text: &str) -> Message {
        Message {
            role: Role::Agent,
            ..Message::text(message_id, text)
        }
    }

    /// All text parts' contents, in order.
    pub fn text_contents(&self) -> Vec<&str> {
        self.parts
            .iter()
            .filter_map(|part| part.text.as_deref())
            .collect()
    }

    /// Boundary validation: part count, per-part text size, and a
    /// non-empty message id. Called by the server on receive and by
    /// the client on send.
    pub fn validate(&self) -> Result<(), String> {
        if self.message_id.is_empty() {
            return Err("messageId must not be empty".to_string());
        }
        if self.parts.len() > PARTS_MAX {
            return Err(format!(
                "message has {} parts; the bound is {PARTS_MAX}",
                self.parts.len()
            ));
        }
        for part in &self.parts {
            if let Some(text) = &part.text
                && text.len() > PART_TEXT_BYTES_MAX
            {
                return Err(format!(
                    "text part has {} bytes; the bound is {PART_TEXT_BYTES_MAX}",
                    text.len()
                ));
            }
        }
        Ok(())
    }
}

/// One discrete piece of content in a message or artifact. Exactly
/// one of `text`, `data`, or `raw` is present in practice; the shape
/// is permissive on parse and helpers build well-formed values.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    /// Text content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Structured JSON content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    /// Base64-encoded binary content (the spec's `bytes`, renamed
    /// for JS compatibility in the 1.0 JSON mapping).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    /// A filename for file content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// The content's media (MIME) type, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    /// Opaque metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl Part {
    /// A text part.
    pub fn text(text: &str) -> Part {
        Part {
            text: Some(text.to_string()),
            data: None,
            raw: None,
            filename: None,
            media_type: None,
            metadata: None,
        }
    }

    /// A structured-data part.
    pub fn data(data: serde_json::Value) -> Part {
        Part {
            text: None,
            data: Some(data),
            raw: None,
            filename: None,
            media_type: None,
            metadata: None,
        }
    }
}

/// A tangible output of a task (spec §4.1.4).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    /// Unique within the task.
    pub artifact_id: String,
    /// Human-readable name, when the producer gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What this artifact is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The artifact's content.
    pub parts: Vec<Part>,
    /// Extension URIs that shaped this artifact.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    /// Opaque metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// A task's status at a moment (spec §4.1.3).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    /// The current lifecycle state.
    pub state: TaskState,
    /// An explanatory message, when the state carries one (a
    /// question for `InputRequired`, a reason for `Failed` — or, in
    /// phlow, a remedy plan rendered as a data part).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<Box<Message>>,
    /// RFC 3339 timestamp of the last state change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// One unit of work (spec §4.1.2).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    /// The task's unique identifier.
    pub id: String,
    /// The conversation this task belongs to.
    pub context_id: String,
    /// Current status.
    pub status: TaskStatus,
    /// Outputs produced so far.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
    /// The conversation history for this task.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Message>,
    /// Opaque metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Status-change stream event (spec §4.2.3).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatusUpdateEvent {
    /// The task.
    pub task_id: String,
    /// Its context.
    pub context_id: String,
    /// The new status.
    pub status: TaskStatus,
    /// Opaque metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Artifact-arrival stream event (spec §4.2.3).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskArtifactUpdateEvent {
    /// The task.
    pub task_id: String,
    /// Its context.
    pub context_id: String,
    /// The artifact or chunk.
    pub artifact: Artifact,
    /// Whether this chunk appends to a previous artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub append: Option<bool>,
    /// Whether this is the final chunk for the artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_chunk: Option<bool>,
}

/// One item of a streaming response: a task snapshot or an update
/// event. Untagged, distinguished by shape (`id`+`status` for a
/// task, `taskId`+`status` for a status update, `taskId`+`artifact`
/// for an artifact update).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum SendStreamingResponse {
    /// A full task snapshot.
    Task(Task),
    /// A status update.
    StatusUpdate(TaskStatusUpdateEvent),
    /// An artifact update.
    ArtifactUpdate(TaskArtifactUpdateEvent),
}

// ---------------------------------------------------------------------
// Operation parameters and results.

/// Parameters for `SendMessage` / `SendStreamingMessage`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageParams {
    /// The message to send (required).
    pub message: Message,
    /// Optional configuration (accepted output modes, etc.).
    /// Accepted and recorded; output-mode negotiation is the
    /// executor's business in this increment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<serde_json::Value>,
    /// Opaque metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Parameters for `GetTask`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTaskParams {
    /// The task id.
    pub id: String,
    /// How much history to include (most recent N messages).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_length: Option<u32>,
}

/// Parameters for `ListTasks`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksParams {
    /// Filter to one context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    /// Filter to one state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskState>,
    /// Page size, 1–100 (default 50, per spec).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_size: Option<u32>,
    /// Continuation token from a previous page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
    /// History per task (most recent N messages).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_length: Option<u32>,
    /// Only tasks updated at or after this RFC 3339 time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_update_after: Option<String>,
    /// Whether to include artifacts in the returned tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_artifacts: Option<bool>,
}

/// Result of `ListTasks`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksResult {
    /// The page of tasks.
    pub tasks: Vec<Task>,
    /// Total matching tasks across pages.
    pub total_size: u32,
    /// The page size actually applied.
    pub page_size: u32,
    /// Token for the next page; empty when this is the last page.
    pub next_page_token: String,
}

/// Parameters for `CancelTask` / `SubscribeToTask`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskIdParams {
    /// The task id.
    pub id: String,
    /// Opaque metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------
// Agent Card.

/// How to reach one interface of the agent (spec §4.4.2).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInterface {
    /// The endpoint URL.
    pub url: String,
    /// The binding: `JSONRPC`, `GRPC`, or `HTTP+JSON`.
    pub protocol_binding: String,
    /// The protocol version served here.
    pub protocol_version: String,
    /// Tenant discriminator, when one endpoint serves many.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
}

/// Who runs the agent (spec §4.4.3).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProvider {
    /// The providing organization.
    pub organization: String,
    /// The organization's URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// What the agent can do beyond the mandatory core (spec §4.4.4).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    /// Streaming responses are supported.
    pub streaming: bool,
    /// Push notifications are supported. This crate's server does
    /// NOT implement them and declares `false`.
    pub push_notifications: bool,
    /// Extension declarations (agent extension objects).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<serde_json::Value>,
}

/// One skill the agent advertises (spec §4.4.6).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    /// Unique skill id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// What the skill does.
    pub description: String,
    /// Discovery tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Example prompts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
    /// Input modes when different from the agent defaults.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_modes: Vec<String>,
    /// Output modes when different from the agent defaults.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modes: Vec<String>,
}

/// The Agent Card (spec §4.4): the agent's self-description, served
/// at `/.well-known/agent-card.json`. In this crate a card is inert
/// data — it never sets a classification or a capability grant.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    /// The agent's name.
    pub name: String,
    /// What the agent does.
    pub description: String,
    /// Reachable interfaces. The first is the primary.
    pub supported_interfaces: Vec<AgentInterface>,
    /// Who provides the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<AgentProvider>,
    /// The implementation version (distinct from protocol version).
    pub version: String,
    /// Documentation URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<String>,
    /// Capabilities.
    pub capabilities: AgentCapabilities,
    /// Declared security schemes, by name (scheme objects ride as
    /// JSON — their schema is the spec's union type).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub security_schemes: BTreeMap<String, serde_json::Value>,
    /// Acceptable security requirement sets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security: Vec<BTreeMap<String, Vec<String>>>,
    /// Default accepted input media types.
    pub default_input_modes: Vec<String>,
    /// Default produced output media types.
    pub default_output_modes: Vec<String>,
    /// Advertised skills.
    pub skills: Vec<AgentSkill>,
    /// Icon URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    /// JWS signatures over the card (RFC 8785 canonicalisation).
    /// Parsed and carried; verification is a later rung.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signatures: Vec<serde_json::Value>,
}

impl AgentCard {
    /// The primary interface URL (the first declared), if any.
    pub fn primary_url(&self) -> Option<&str> {
        self.supported_interfaces
            .first()
            .map(|iface| iface.url.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_state_names_round_trip_and_accept_legacy_spellings() {
        for state in [
            TaskState::Submitted,
            TaskState::Working,
            TaskState::Completed,
            TaskState::InputRequired,
        ] {
            let json = serde_json::to_string(&state).expect("serialize");
            let parsed: TaskState = serde_json::from_str(&json).expect("parse own output");
            assert_eq!(parsed, state);
        }
        let legacy: TaskState = serde_json::from_str("\"input-required\"").expect("legacy");
        assert_eq!(legacy, TaskState::InputRequired);
        assert!(serde_json::from_str::<TaskState>("\"bogus\"").is_err());
    }

    #[test]
    fn terminal_states_are_exactly_the_four() {
        for (state, terminal) in [
            (TaskState::Submitted, false),
            (TaskState::Working, false),
            (TaskState::InputRequired, false),
            (TaskState::AuthRequired, false),
            (TaskState::Completed, true),
            (TaskState::Failed, true),
            (TaskState::Canceled, true),
            (TaskState::Rejected, true),
        ] {
            assert_eq!(state.is_terminal(), terminal, "{state:?}");
        }
    }

    #[test]
    fn message_shape_uses_camel_case_and_proto_enums() {
        let message = Message::text("m1", "hello");
        let json = serde_json::to_value(&message).expect("to value");
        assert_eq!(json["messageId"], "m1");
        assert_eq!(json["role"], "ROLE_USER");
        assert_eq!(json["parts"][0]["text"], "hello");
    }

    #[test]
    fn message_validation_enforces_bounds() {
        let mut message = Message::text("m1", "x");
        message.parts = (0..=PARTS_MAX).map(|_| Part::text("y")).collect();
        assert!(message.validate().is_err());
        let huge = Message::text("m2", &"z".repeat(PART_TEXT_BYTES_MAX + 1));
        assert!(huge.validate().is_err());
    }

    #[test]
    fn streaming_response_shapes_are_distinguished() {
        let task_json = serde_json::json!({
            "id": "t1", "contextId": "c1",
            "status": { "state": "TASK_STATE_WORKING" },
            "artifacts": [], "history": []
        });
        let parsed: SendStreamingResponse = serde_json::from_value(task_json).expect("task shape");
        assert!(matches!(parsed, SendStreamingResponse::Task(_)));

        let status_json = serde_json::json!({
            "taskId": "t1", "contextId": "c1",
            "status": { "state": "TASK_STATE_COMPLETED" }
        });
        let parsed: SendStreamingResponse =
            serde_json::from_value(status_json).expect("status shape");
        assert!(matches!(parsed, SendStreamingResponse::StatusUpdate(_)));

        let artifact_json = serde_json::json!({
            "taskId": "t1", "contextId": "c1",
            "artifact": { "artifactId": "a1", "parts": [{ "text": "hi" }] }
        });
        let parsed: SendStreamingResponse =
            serde_json::from_value(artifact_json).expect("artifact shape");
        assert!(matches!(parsed, SendStreamingResponse::ArtifactUpdate(_)));
    }
}
