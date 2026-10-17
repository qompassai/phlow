//! A trajectory: one agent's append-only event log under one
//! policy, with its label *derived* by folding the log.
//!
//! Plain words: there is no label field to set, patch, or forget to
//! update. [`fold`] walks the log from the start label and joins the
//! contribution of every label-carrying event; [`Trajectory::label`]
//! is that fold, recomputed on demand (cheap: logs are bounded by
//! [`EVENTS_MAX`]). Because the label is a pure function of the log,
//! two observers folding the same log under the same policy always
//! agree — the property the gate's replay-determinism rests on.
//!
//! A sub-agent's trajectory starts from an explicit inherited label
//! recorded in its first event. Nothing else is copied from the
//! parent — in particular, no approval events — so a child can never
//! spend a parent's approvals (the confused-deputy defence).

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::EVENTS_MAX;
use crate::error::FlowError;
use crate::event::{Event, JoinSpec, ProposedAction};
use crate::gate::{Decision, decide};
use crate::label::{Label, LabelSnapshot};
use crate::policy::Policy;

/// The result of folding a log: the current label and the set of
/// effects recorded so far.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fold {
    /// The trajectory's current label.
    pub label: Label,
    /// Every effect string recorded by results and explicit events.
    pub effects: BTreeSet<String>,
}

/// Fold a log under a policy.
///
/// The start label is the first event's recorded start snapshot for
/// sub-agent trajectories, else the policy start label. Every
/// label-carrying event joins its contribution; all other events are
/// label-neutral.
///
/// # Errors
///
/// Fails closed: an event naming something the policy no longer
/// declares returns [`FlowError`], and callers (the gate) must treat
/// that as a block, never as permission.
pub fn fold(policy: &Policy, log: &[Event]) -> Result<Fold, FlowError> {
    let mut label = match log.first() {
        Some(Event::SessionStarted {
            start: Some(snapshot),
            ..
        }) => policy.resolve_snapshot(snapshot)?,
        _ => policy.start_label(),
    };
    let mut effects = BTreeSet::new();
    for event in log {
        match event {
            Event::DataRead { delta, .. } => {
                label = label.join(&policy.resolve_delta(delta)?);
            }
            Event::MessageReceived { join, .. } => {
                label = label.join(&resolve_join(policy, join)?);
            }
            Event::ToolResult {
                delta,
                effects: recorded,
                ..
            } => {
                label = label.join(&policy.resolve_delta(delta)?);
                effects.extend(recorded.iter().cloned());
            }
            Event::EffectRecorded { effect } => {
                effects.insert(effect.clone());
            }
            Event::SubagentResult {
                label: child_label, ..
            } => {
                label = label.join(&policy.resolve_snapshot(child_label)?);
            }
            Event::Sanitized { result_delta, .. } => {
                label = label.join(&policy.resolve_delta(result_delta)?);
            }
            _ => {}
        }
    }
    Ok(Fold { label, effects })
}

/// Resolve a [`JoinSpec`] to the label it contributes.
fn resolve_join(policy: &Policy, join: &JoinSpec) -> Result<Label, FlowError> {
    match join {
        JoinSpec::Delta(delta) => policy.resolve_delta(delta),
        JoinSpec::Snapshot(snapshot) => policy.resolve_snapshot(snapshot),
    }
}

/// One trajectory: a policy plus its append-only log.
#[derive(Debug)]
pub struct Trajectory {
    policy: Arc<Policy>,
    events: Vec<Event>,
}

impl Trajectory {
    /// Begin a top-level trajectory under `policy`.
    pub fn new(policy: Arc<Policy>) -> Trajectory {
        Trajectory {
            policy,
            events: vec![Event::SessionStarted {
                parent: None,
                start: None,
            }],
        }
    }

    /// Begin a sub-agent trajectory. `start` is the inherited label
    /// computed by the spawner (parent label ⊔ requested delta ⊔
    /// policy child restrictions) and is recorded verbatim in the
    /// first event, so the child's fold is self-contained.
    pub fn child(policy: Arc<Policy>, parent: &str, start: LabelSnapshot) -> Trajectory {
        Trajectory {
            policy,
            events: vec![Event::SessionStarted {
                parent: Some(parent.to_string()),
                start: Some(start),
            }],
        }
    }

    /// The policy this trajectory runs under.
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// The log, oldest first.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// The current label: the fold of the log.
    ///
    /// # Errors
    ///
    /// Propagates fold errors (fail closed at call sites).
    pub fn label(&self) -> Result<Label, FlowError> {
        Ok(fold(&self.policy, &self.events)?.label)
    }

    /// The current label rendered as a snapshot, for handoffs.
    ///
    /// # Errors
    ///
    /// Propagates fold errors.
    pub fn snapshot(&self) -> Result<LabelSnapshot, FlowError> {
        Ok(self.policy.snapshot_of(&self.label()?))
    }

    /// Append a raw event, enforcing the log bound. Prefer the typed
    /// recorders below; this exists for embedders with their own
    /// event producers (focus events, spawn records).
    ///
    /// # Errors
    ///
    /// [`FlowError::LogFull`] past [`EVENTS_MAX`].
    pub fn record(&mut self, event: Event) -> Result<(), FlowError> {
        if self.events.len() >= EVENTS_MAX {
            return Err(FlowError::LogFull);
        }
        self.events.push(event);
        Ok(())
    }

    /// Record reading from a named policy source; the source's
    /// declared delta is what gets recorded (and joined).
    ///
    /// # Errors
    ///
    /// [`FlowError::UnknownName`] if the policy declares no such
    /// source; [`FlowError::LogFull`] past the bound.
    pub fn record_read(&mut self, source: &str) -> Result<(), FlowError> {
        let delta = self
            .policy
            .source(source)
            .ok_or_else(|| FlowError::UnknownName(source.to_string()))?
            .delta
            .clone();
        self.record(Event::DataRead {
            source: source.to_string(),
            delta,
        })
    }

    /// Record a received message's contribution.
    ///
    /// # Errors
    ///
    /// [`FlowError::LogFull`] past the bound.
    pub fn record_message_received(&mut self, from: &str, join: JoinSpec) -> Result<(), FlowError> {
        self.record(Event::MessageReceived {
            from: from.to_string(),
            join,
        })
    }

    /// Propose an action to the gate: decide over the current log,
    /// record the proposal and the verdict, and return the decision.
    /// The decision returned is the decision recorded — an embedder
    /// acts on exactly what the log shows.
    ///
    /// # Errors
    ///
    /// [`FlowError::LogFull`] past the bound.
    pub fn propose(&mut self, action: &ProposedAction) -> Result<Decision, FlowError> {
        let decision = decide(&self.policy, &self.events, action);
        let digest = decision.digest().to_string();
        match action {
            ProposedAction::ToolCall { tool, .. } => {
                self.record(Event::ToolProposed {
                    tool: tool.clone(),
                    digest: digest.clone(),
                })?;
                let verdict = match &decision {
                    Decision::Allow(_) => Event::ToolAllowed {
                        tool: tool.clone(),
                        digest,
                    },
                    Decision::Block(_) => Event::ToolBlocked {
                        tool: tool.clone(),
                        digest,
                    },
                };
                self.record(verdict)?;
            }
            ProposedAction::Message { destination, .. } => {
                self.record(Event::MessageProposed {
                    destination: destination.clone(),
                    digest: digest.clone(),
                })?;
                let verdict = match &decision {
                    Decision::Allow(_) => Event::MessageAllowed {
                        destination: destination.clone(),
                        digest,
                    },
                    Decision::Block(_) => Event::MessageBlocked {
                        destination: destination.clone(),
                        digest,
                    },
                };
                self.record(verdict)?;
            }
        }
        Ok(decision)
    }

    /// Record a tool result: the contract's delta joins the label
    /// and its effects are recorded. `digest` is the digest of the
    /// action that produced the result (it is what consumes an
    /// approval).
    ///
    /// # Errors
    ///
    /// [`FlowError::UnknownName`] if the tool has no contract;
    /// [`FlowError::LogFull`] past the bound.
    pub fn record_tool_result(&mut self, tool: &str, digest: &str) -> Result<(), FlowError> {
        let contract = self
            .policy
            .tool(tool)
            .ok_or_else(|| FlowError::UnknownName(tool.to_string()))?;
        let delta = contract.delta.clone().unwrap_or_default();
        let effects = contract.effects.clone();
        self.record(Event::ToolResult {
            tool: tool.to_string(),
            digest: digest.to_string(),
            delta,
            effects,
        })
    }
}
