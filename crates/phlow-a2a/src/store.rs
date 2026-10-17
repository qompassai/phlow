//! The task store: an in-memory, bounded state machine for the
//! A2A task lifecycle.
//!
//! Plain words: tasks live in one [`TaskStore`] value owned by the
//! server — there is no global registry and no persistence; a
//! restart starts empty, and the SPEC says so. Every capacity is
//! checked *before* allocation: task count, per-task history, and
//! per-task artifacts. State changes go through
//! [`TaskStore::transition`], which enforces the legal-transition
//! table from the specification; illegal transitions (including any
//! move out of a terminal state) are errors, never silent no-ops.
//! Time enters only as caller-supplied RFC 3339 strings, which keeps
//! the machine deterministic under test.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::types::{
    Artifact, ListTasksParams, ListTasksResult, Message, Task, TaskState, TaskStatus,
};
use crate::{ARTIFACTS_MAX, HISTORY_MAX, TASKS_MAX};

/// Why a store operation failed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreError {
    /// The store already holds [`TASKS_MAX`] tasks.
    Full,
    /// No task with that id.
    NotFound(String),
    /// The requested state change is not in the transition table.
    InvalidTransition {
        /// Current state.
        from: TaskState,
        /// Requested state.
        to: TaskState,
    },
    /// The task already holds [`ARTIFACTS_MAX`] artifacts.
    TooManyArtifacts,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Full => write!(formatter, "task store is full ({TASKS_MAX} tasks)"),
            StoreError::NotFound(id) => write!(formatter, "task not found: {id}"),
            StoreError::InvalidTransition { from, to } => write!(
                formatter,
                "invalid task transition: {} -> {}",
                from.as_proto_name(),
                to.as_proto_name()
            ),
            StoreError::TooManyArtifacts => {
                write!(formatter, "task already holds {ARTIFACTS_MAX} artifacts")
            }
        }
    }
}

impl std::error::Error for StoreError {}

/// Is `from -> to` a legal lifecycle transition?
///
/// The table (spec §4.1.2 and the program SPEC §8.2): the active
/// states flow forward; `InputRequired`/`AuthRequired` can resume
/// to `Working` or end; terminal states have no exits; a task can
/// be cancelled from any non-terminal state; `Submitted` may be
/// rejected before work begins.
pub fn transition_valid(from: TaskState, to: TaskState) -> bool {
    use TaskState as S;
    if from.is_terminal() || from == to {
        // No exits from terminal states; same-state "transitions"
        // are status updates, handled by appending a message instead.
        return false;
    }
    matches!(
        (from, to),
        (S::Submitted, S::Working)
            | (S::Submitted, S::Rejected)
            | (S::Working, S::Completed)
            | (S::Working, S::Failed)
            | (S::Working, S::InputRequired)
            | (S::Working, S::AuthRequired)
            | (S::InputRequired, S::Working)
            | (S::InputRequired, S::Failed)
            | (S::AuthRequired, S::Working)
            | (S::AuthRequired, S::Failed)
            | (_, S::Canceled)
    )
}

struct Record {
    task: Task,
    cancel_flag: Arc<AtomicBool>,
}

/// The task store. One value, owned explicitly (by the server);
/// methods take `&mut self`, so the owner serialises access with
/// whatever lock fits its threading model.
pub struct TaskStore {
    seed: u64,
    counter: u64,
    records: HashMap<String, Record>,
    creation_order: Vec<String>,
}

impl TaskStore {
    /// A store with the given id seed (see [`crate::util::boot_seed`]).
    pub fn new(seed: u64) -> TaskStore {
        TaskStore {
            seed,
            counter: 0,
            records: HashMap::new(),
            creation_order: Vec::new(),
        }
    }

    fn mint(&mut self, kind: &str) -> String {
        self.counter += 1;
        crate::util::mint_id(kind, self.seed, self.counter)
    }

    /// Number of tasks currently held.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Is the store empty?
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Create a task in `Submitted` from an initial message. The
    /// message is stamped with the task/context ids and becomes the
    /// first history entry. A fresh context id is minted when the
    /// message does not carry one.
    ///
    /// # Errors
    ///
    /// [`StoreError::Full`] at capacity.
    pub fn create(&mut self, message: Message, now: &str) -> Result<Task, StoreError> {
        if self.records.len() >= TASKS_MAX {
            return Err(StoreError::Full);
        }
        let task_id = self.mint("task");
        let context_id = match &message.context_id {
            Some(existing) if !existing.is_empty() => existing.clone(),
            _ => self.mint("ctx"),
        };
        let mut stamped = message;
        stamped.task_id = Some(task_id.clone());
        stamped.context_id = Some(context_id.clone());
        let task = Task {
            id: task_id.clone(),
            context_id,
            status: TaskStatus {
                state: TaskState::Submitted,
                message: None,
                timestamp: Some(now.to_string()),
            },
            artifacts: Vec::new(),
            history: vec![stamped],
            metadata: None,
        };
        self.records.insert(
            task_id.clone(),
            Record {
                task: task.clone(),
                cancel_flag: Arc::new(AtomicBool::new(false)),
            },
        );
        self.creation_order.push(task_id);
        Ok(task)
    }

    /// Fetch a task, with history truncated to the most recent
    /// `history_length` messages when given.
    pub fn get(&self, id: &str, history_length: Option<u32>) -> Option<Task> {
        let record = self.records.get(id)?;
        Some(truncated(&record.task, history_length, true))
    }

    /// The cancellation flag for a task, for executors to poll.
    pub fn cancel_flag(&self, id: &str) -> Option<Arc<AtomicBool>> {
        self.records
            .get(id)
            .map(|record| record.cancel_flag.clone())
    }

    /// Has cancellation been requested for this task?
    pub fn is_cancelled(&self, id: &str) -> bool {
        self.records
            .get(id)
            .map(|record| record.cancel_flag.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// Move a task to a new state, optionally attaching a status
    /// message (also appended to history, bounded). Returns the new
    /// status.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`]; [`StoreError::InvalidTransition`]
    /// for moves outside the table.
    pub fn transition(
        &mut self,
        id: &str,
        to: TaskState,
        message: Option<Message>,
        now: &str,
    ) -> Result<TaskStatus, StoreError> {
        let record = self
            .records
            .get_mut(id)
            .ok_or_else(|| StoreError::NotFound(id.to_string()))?;
        let from = record.task.status.state;
        if !transition_valid(from, to) {
            return Err(StoreError::InvalidTransition { from, to });
        }
        record.task.status = TaskStatus {
            state: to,
            message: message.clone().map(Box::new),
            timestamp: Some(now.to_string()),
        };
        if let Some(message) = message {
            push_history(&mut record.task, message);
        }
        Ok(record.task.status.clone())
    }

    /// Append a message to a task's history without a state change
    /// (e.g. a follow-up message while work continues).
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`]; [`StoreError::InvalidTransition`]
    /// if the task is terminal — messages to terminal tasks are
    /// rejected by the protocol.
    pub fn append_message(&mut self, id: &str, message: Message) -> Result<(), StoreError> {
        let record = self
            .records
            .get_mut(id)
            .ok_or_else(|| StoreError::NotFound(id.to_string()))?;
        let state = record.task.status.state;
        if state.is_terminal() {
            return Err(StoreError::InvalidTransition {
                from: state,
                to: state,
            });
        }
        push_history(&mut record.task, message);
        Ok(())
    }

    /// Add an artifact, or append parts to the existing artifact
    /// with the same id (the spec's chunked-update semantics).
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`]; [`StoreError::TooManyArtifacts`]
    /// for a *new* artifact past the bound.
    pub fn add_artifact(&mut self, id: &str, artifact: Artifact) -> Result<Artifact, StoreError> {
        let record = self
            .records
            .get_mut(id)
            .ok_or_else(|| StoreError::NotFound(id.to_string()))?;
        if let Some(existing) = record
            .task
            .artifacts
            .iter_mut()
            .find(|held| held.artifact_id == artifact.artifact_id)
        {
            existing.parts.extend(artifact.parts.iter().cloned());
            return Ok(existing.clone());
        }
        if record.task.artifacts.len() >= ARTIFACTS_MAX {
            return Err(StoreError::TooManyArtifacts);
        }
        record.task.artifacts.push(artifact.clone());
        Ok(artifact)
    }

    /// Cancel a task: set its flag (executors poll it) and move it
    /// to `Canceled`. Returns the updated task.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`]; [`StoreError::InvalidTransition`]
    /// if the task is already terminal (the protocol's
    /// TaskNotCancelable case).
    pub fn cancel(&mut self, id: &str, now: &str) -> Result<Task, StoreError> {
        let flag = self
            .cancel_flag(id)
            .ok_or_else(|| StoreError::NotFound(id.to_string()))?;
        flag.store(true, Ordering::SeqCst);
        self.transition(id, TaskState::Canceled, None, now)?;
        self.get(id, None)
            .ok_or_else(|| StoreError::NotFound(id.to_string()))
    }

    /// List tasks with the spec's filters and pagination: page size
    /// clamped to 1–100 (default 50), page token = the next offset,
    /// creation order stable.
    pub fn list(&self, params: &ListTasksParams) -> ListTasksResult {
        let page_size = params.page_size.unwrap_or(50).clamp(1, 100);
        let offset: usize = params
            .page_token
            .as_deref()
            .and_then(|token| token.parse().ok())
            .unwrap_or(0);
        let matching: Vec<&Task> = self
            .creation_order
            .iter()
            .filter_map(|id| self.records.get(id))
            .map(|record| &record.task)
            .filter(|task| {
                params
                    .context_id
                    .as_ref()
                    .map(|wanted| &task.context_id == wanted)
                    .unwrap_or(true)
            })
            .filter(|task| {
                params
                    .status
                    .map(|wanted| task.status.state == wanted)
                    .unwrap_or(true)
            })
            .filter(
                |task| match (&params.last_update_after, &task.status.timestamp) {
                    (Some(after), Some(stamp)) => stamp.as_str() >= after.as_str(),
                    (Some(_), None) => false,
                    _ => true,
                },
            )
            .collect();
        let total = matching.len();
        let page: Vec<Task> = matching
            .into_iter()
            .skip(offset)
            .take(page_size as usize)
            .map(|task| {
                truncated(
                    task,
                    params.history_length,
                    params.include_artifacts.unwrap_or(true),
                )
            })
            .collect();
        let next_offset = offset + page.len();
        ListTasksResult {
            tasks: page,
            total_size: u32::try_from(total).unwrap_or(u32::MAX),
            page_size,
            next_page_token: if next_offset < total {
                next_offset.to_string()
            } else {
                String::new()
            },
        }
    }
}

/// Append to history, evicting the oldest entries past the bound.
fn push_history(task: &mut Task, message: Message) {
    task.history.push(message);
    if task.history.len() > HISTORY_MAX {
        let excess = task.history.len() - HISTORY_MAX;
        task.history.drain(0..excess);
    }
}

/// A task clone with history truncated to the most recent N messages
/// (all of it when `history_length` is `None`) and, optionally,
/// artifacts elided.
fn truncated(task: &Task, history_length: Option<u32>, include_artifacts: bool) -> Task {
    let mut out = task.clone();
    if let Some(length) = history_length {
        let keep = (length as usize).min(out.history.len());
        let skip = out.history.len() - keep;
        out.history.drain(0..skip);
    }
    if !include_artifacts {
        out.artifacts.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Part;

    const NOW: &str = "2026-10-17T00:00:00Z";

    fn store_with_task() -> (TaskStore, Task) {
        let mut store = TaskStore::new(42);
        let task = store
            .create(Message::text("m1", "do the thing"), NOW)
            .expect("create");
        (store, task)
    }

    #[test]
    fn lifecycle_happy_path_and_terminal_lockout() {
        let (mut store, task) = store_with_task();
        assert_eq!(task.status.state, TaskState::Submitted);
        store
            .transition(&task.id, TaskState::Working, None, NOW)
            .expect("work");
        store
            .transition(&task.id, TaskState::InputRequired, None, NOW)
            .expect("pause");
        store
            .transition(&task.id, TaskState::Working, None, NOW)
            .expect("resume");
        store
            .transition(&task.id, TaskState::Completed, None, NOW)
            .expect("complete");
        // Terminal: nothing leaves, not even cancel.
        assert!(
            store
                .transition(&task.id, TaskState::Working, None, NOW)
                .is_err()
        );
        assert!(store.cancel(&task.id, NOW).is_err());
        assert!(
            store
                .append_message(&task.id, Message::text("m2", "more"))
                .is_err()
        );
    }

    #[test]
    fn illegal_skips_are_rejected() {
        let (mut store, task) = store_with_task();
        assert!(matches!(
            store.transition(&task.id, TaskState::Completed, None, NOW),
            Err(StoreError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn cancel_sets_the_flag_and_state() {
        let (mut store, task) = store_with_task();
        store
            .transition(&task.id, TaskState::Working, None, NOW)
            .expect("work");
        let cancelled = store.cancel(&task.id, NOW).expect("cancel");
        assert_eq!(cancelled.status.state, TaskState::Canceled);
        assert!(store.is_cancelled(&task.id));
    }

    #[test]
    fn history_is_bounded_and_truncation_keeps_the_newest() {
        let (mut store, task) = store_with_task();
        for index in 0..(HISTORY_MAX + 5) {
            store
                .append_message(&task.id, Message::text(&format!("m{index}"), "x"))
                .expect("append");
        }
        let full = store.get(&task.id, None).expect("get");
        assert_eq!(full.history.len(), HISTORY_MAX);
        let short = store.get(&task.id, Some(3)).expect("get short");
        assert_eq!(short.history.len(), 3);
        assert_eq!(
            short.history.last().map(|m| m.message_id.as_str()),
            Some(format!("m{}", HISTORY_MAX + 4).as_str())
        );
    }

    #[test]
    fn artifacts_append_by_id_and_are_bounded() {
        let (mut store, task) = store_with_task();
        let artifact = Artifact {
            artifact_id: "a1".to_string(),
            name: None,
            description: None,
            parts: vec![Part::text("one")],
            extensions: Vec::new(),
            metadata: None,
        };
        store.add_artifact(&task.id, artifact.clone()).expect("add");
        let grown = store.add_artifact(&task.id, artifact).expect("append");
        assert_eq!(grown.parts.len(), 2);
        for index in 0..ARTIFACTS_MAX {
            let filler = Artifact {
                artifact_id: format!("f{index}"),
                ..grown.clone()
            };
            let _ = store.add_artifact(&task.id, filler);
        }
        let extra = Artifact {
            artifact_id: "one-too-many".to_string(),
            name: None,
            description: None,
            parts: Vec::new(),
            extensions: Vec::new(),
            metadata: None,
        };
        assert_eq!(
            store.add_artifact(&task.id, extra),
            Err(StoreError::TooManyArtifacts)
        );
    }

    #[test]
    fn listing_filters_and_paginates() {
        let mut store = TaskStore::new(7);
        let mut ids = Vec::new();
        for index in 0..5 {
            let task = store
                .create(Message::text(&format!("m{index}"), "x"), NOW)
                .expect("create");
            ids.push(task.id);
        }
        store
            .transition(&ids[0], TaskState::Working, None, NOW)
            .expect("work");
        let page1 = store.list(&ListTasksParams {
            page_size: Some(2),
            ..Default::default()
        });
        assert_eq!(page1.tasks.len(), 2);
        assert_eq!(page1.total_size, 5);
        assert_eq!(page1.next_page_token, "2");
        let page2 = store.list(&ListTasksParams {
            page_size: Some(2),
            page_token: Some(page1.next_page_token.clone()),
            ..Default::default()
        });
        assert_eq!(page2.tasks.len(), 2);
        let working = store.list(&ListTasksParams {
            status: Some(TaskState::Working),
            ..Default::default()
        });
        assert_eq!(working.tasks.len(), 1);
        assert_eq!(working.tasks[0].id, ids[0]);
    }

    #[test]
    fn context_id_is_stable_across_a_tasks_messages() {
        let (mut store, task) = store_with_task();
        let mut follow_up = Message::text("m9", "more detail");
        follow_up.context_id = Some(task.context_id.clone());
        let second = store.create(follow_up, NOW).expect("second task");
        assert_eq!(second.context_id, task.context_id);
        assert_ne!(second.id, task.id);
    }
}
