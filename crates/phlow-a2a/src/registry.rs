//! The external-agent registry: the meeting point of the A2A
//! client and the flow policy.
//!
//! Plain words: registering an external agent does two independent
//! things. It records how to *reach* the agent (its interface URL
//! and an optional bearer token), and it names the **policy
//! destination** the agent corresponds to — the destination whose
//! declared readers and minimum trust the flow gate checks before
//! any send. The registry entry is not itself a classification; the
//! policy is. An agent whose destination the policy does not
//! declare cannot be sent to at all (the gate blocks it as an
//! undeclared destination), and an agent's own Agent Card never
//! changes any of this — cards are inert data.
//!
//! [`ExternalRegistry::send_task`] is the gated path: propose the
//! send on the caller's trajectory, let the gate decide, and only
//! then touch the network. The registry also implements
//! [`OutboundSink`] so a server's executors can deliver through it.

use std::collections::HashMap;

use phlow_flowguard::event::ProposedAction;
use phlow_flowguard::gate::{BlockInfo, Decision};
use phlow_flowguard::trajectory::Trajectory;

use crate::AGENTS_MAX;
use crate::client::{A2aClient, A2aError, ClientConfig};
use crate::server::OutboundSink;
use crate::types::{Message, Task};

/// One registered external agent.
#[derive(Clone, Debug)]
pub struct ExternalAgent {
    /// The agent's registry name (how local code refers to it).
    pub name: String,
    /// The A2A interface URL (from its card's primary interface,
    /// recorded at registration — by the operator, not by the card
    /// fetch itself).
    pub interface_url: String,
    /// The policy destination name sends to this agent are gated
    /// against (e.g. `external:weather-bot`).
    pub destination: String,
    /// Bearer token for the peer, when it requires one.
    pub token: Option<String>,
}

/// Why a registry operation failed.
#[derive(Debug)]
pub enum RegistryError {
    /// No agent registered under that name (or no agent for that
    /// destination, for sink deliveries).
    UnknownAgent(String),
    /// The registry is at capacity.
    Full,
    /// The flow gate blocked the send. The block (with its remedy
    /// plan) is the payload.
    Blocked(BlockInfo),
    /// The A2A call itself failed.
    Client(A2aError),
    /// A flow-side failure (fold errors and the like).
    Flow(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::UnknownAgent(name) => {
                write!(formatter, "unknown external agent: {name}")
            }
            RegistryError::Full => write!(formatter, "external agent registry is full"),
            RegistryError::Blocked(info) => {
                write!(formatter, "flow gate blocked the send: {:?}", info.reasons)
            }
            RegistryError::Client(err) => write!(formatter, "{err}"),
            RegistryError::Flow(detail) => write!(formatter, "flow: {detail}"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// The registry. Clients are built per call from the registered
/// entries (construction is cheap; no connection is opened until a
/// call is made), so a server's outbound sink and direct callers
/// share exactly the same registration data.
pub struct ExternalRegistry {
    agents: HashMap<String, ExternalAgent>,
}

impl ExternalRegistry {
    /// An empty registry.
    pub fn new() -> ExternalRegistry {
        ExternalRegistry {
            agents: HashMap::new(),
        }
    }

    /// Register (or replace) an agent.
    ///
    /// # Errors
    ///
    /// [`RegistryError::Full`] at [`AGENTS_MAX`] distinct names.
    pub fn register(&mut self, agent: ExternalAgent) -> Result<(), RegistryError> {
        if !self.agents.contains_key(&agent.name) && self.agents.len() >= AGENTS_MAX {
            return Err(RegistryError::Full);
        }
        self.agents.insert(agent.name.clone(), agent);
        Ok(())
    }

    /// The registered agent under `name`.
    pub fn agent(&self, name: &str) -> Option<&ExternalAgent> {
        self.agents.get(name)
    }

    /// All registered names, sorted.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.agents.keys().cloned().collect();
        names.sort();
        names
    }

    /// A client for the named agent (built and cached on first use).
    ///
    /// # Errors
    ///
    /// [`RegistryError::UnknownAgent`]; [`RegistryError::Client`]
    /// for client construction failures.
    pub fn client_for(&self, name: &str) -> Result<A2aClient, RegistryError> {
        let agent = self
            .agents
            .get(name)
            .ok_or_else(|| RegistryError::UnknownAgent(name.to_string()))?;
        A2aClient::new(
            &agent.interface_url,
            ClientConfig {
                token: agent.token.clone(),
                ..ClientConfig::default()
            },
        )
        .map_err(RegistryError::Client)
    }

    /// Send a tasking message to a registered agent, gated by the
    /// caller's trajectory: the gate decides from the policy the
    /// trajectory carries and its log, the proposal and verdict are
    /// recorded in the trajectory, and the network is touched only
    /// on an allow.
    ///
    /// # Errors
    ///
    /// [`RegistryError::Blocked`] with the gate's block (including
    /// its remedy plan) when the send is not allowed.
    pub fn send_task(
        &self,
        sender: &mut Trajectory,
        agent_name: &str,
        message: &Message,
    ) -> Result<Task, RegistryError> {
        let agent = self
            .agents
            .get(agent_name)
            .ok_or_else(|| RegistryError::UnknownAgent(agent_name.to_string()))?;
        let decision = sender
            .propose(&ProposedAction::Message {
                destination: agent.destination.clone(),
                payload_label: None,
            })
            .map_err(|err| RegistryError::Flow(err.to_string()))?;
        match decision {
            Decision::Allow(_) => {}
            Decision::Block(info) => return Err(RegistryError::Blocked(info)),
        }
        let client = self.client_for(agent_name)?;
        client.send_message(message).map_err(RegistryError::Client)
    }
}

impl Default for ExternalRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl OutboundSink for ExternalRegistry {
    fn deliver(&self, destination: &str, message: &Message) -> Result<(), String> {
        let agent = self
            .agents
            .values()
            .find(|agent| agent.destination == destination)
            .ok_or_else(|| format!("no registered agent for destination {destination}"))?;
        let client = self
            .client_for(&agent.name)
            .map_err(|err| err.to_string())?;
        client
            .send_message(message)
            .map(|_| ())
            .map_err(|err| err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_bounds_and_replacement() {
        let mut registry = ExternalRegistry::new();
        for index in 0..AGENTS_MAX {
            registry
                .register(ExternalAgent {
                    name: format!("agent-{index}"),
                    interface_url: "http://127.0.0.1:9/".to_string(),
                    destination: format!("external:{index}"),
                    token: None,
                })
                .expect("register");
        }
        assert!(matches!(
            registry.register(ExternalAgent {
                name: "one-too-many".to_string(),
                interface_url: "http://127.0.0.1:9/".to_string(),
                destination: "external:x".to_string(),
                token: None,
            }),
            Err(RegistryError::Full)
        ));
        // Replacing an existing name is not growth.
        registry
            .register(ExternalAgent {
                name: "agent-0".to_string(),
                interface_url: "http://127.0.0.1:10/".to_string(),
                destination: "external:0".to_string(),
                token: None,
            })
            .expect("replace");
        assert_eq!(registry.names().len(), AGENTS_MAX);
    }
}
