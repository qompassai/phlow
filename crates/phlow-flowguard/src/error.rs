//! Failure taxonomy for the flow guard.
//!
//! Plain words: loading a policy can fail (bad TOML, dangling
//! references, a group cycle, an unknown trust rank) and folding a log
//! can fail (an event names a token the policy no longer declares).
//! Both fail *closed*: the gate treats a fold error as a block, never
//! as permission.

use std::fmt;

/// Everything that can go wrong before a decision is reached.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FlowError {
    /// An event or delta named an audience token the policy does not
    /// declare (an unknown `@group`).
    UnknownAudienceToken(String),
    /// A trust rank name is not in the policy's trust chain.
    UnknownTrustRank(String),
    /// A group expansion recursed (a group that contains itself,
    /// directly or transitively), or expansion exceeded its depth cap.
    AudienceCycle(String),
    /// The policy TOML did not parse.
    Toml(String),
    /// The policy parsed but is inconsistent: duplicate names, a
    /// `within` assertion that does not hold, an undeclared tool in a
    /// sanitizer, a size bound exceeded, and similar load-time faults.
    /// The string names the fault precisely for the policy author.
    Policy(String),
    /// An action was proposed against a log that already holds the
    /// maximum number of events.
    LogFull,
    /// A caller asked for something by a name the policy does not
    /// carry (sanitizer, source, destination).
    UnknownName(String),
}

impl fmt::Display for FlowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlowError::UnknownAudienceToken(token) => {
                write!(formatter, "unknown audience token: {token}")
            }
            FlowError::UnknownTrustRank(rank) => {
                write!(formatter, "unknown trust rank: {rank}")
            }
            FlowError::AudienceCycle(token) => {
                write!(formatter, "audience group cycle at: {token}")
            }
            FlowError::Toml(message) => write!(formatter, "policy TOML: {message}"),
            FlowError::Policy(message) => write!(formatter, "policy: {message}"),
            FlowError::LogFull => write!(formatter, "trajectory log is full"),
            FlowError::UnknownName(name) => write!(formatter, "unknown name: {name}"),
        }
    }
}

impl std::error::Error for FlowError {}
