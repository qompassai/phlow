//! The declarative policy: TOML in, a validated [`Policy`] out.
//!
//! Plain words: the policy declares the trust chain, the named
//! audiences (the `self` and `internal` builtins and any `@group`s),
//! a contract for every tool (what calling it does to the label, what
//! it forwards, what it requires, what effects it records), the named
//! message destinations, the named inbound sources, the sanitizers,
//! and the authorities. Loading validates *everything* up front —
//! dangling references, group cycles, `within` assertions, unknown
//! trust ranks, size bounds — so a policy that loads is a policy the
//! gate can evaluate without surprises. [`Policy::validate_coverage`]
//! then checks the operator's tool inventory against the contracts:
//! a tool with no contract is refused by the gate, so coverage gaps
//! are policy bugs to fix at load time, not runtime discoveries.
//!
//! Audience *expansion* is the one non-obvious piece: tokens are
//! compared by expanded reader set. `public` expands to
//! [`ReaderSet::All`]; `self`/`internal` expand to their declared
//! token lists; `@group` expands to its members; anything else is a
//! literal reader id and expands to itself. Expansion recursion depth
//! is capped at [`EXPANSION_DEPTH_MAX`] and cycles are rejected at
//! load, so expansion always terminates.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::FlowError;
use crate::label::{Label, LabelDelta, LabelSnapshot, ReaderSet};
use crate::{POLICY_TOOLS_MAX, READER_TOKENS_MAX, TOKEN_BYTES_MAX};

/// Deepest audience-token expansion chain (group → group → …).
pub const EXPANSION_DEPTH_MAX: usize = 8;

/// The default trust chain, least → most trusted, when the policy
/// does not declare one (OpenAPPA's default).
pub const TRUST_CHAIN_DEFAULT: [&str; 2] = ["suspicious", "trusted"];

// ---------------------------------------------------------------------
// TOML shapes (private; the public surface is `Policy`).

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyToml {
    version: u32,
    #[serde(default)]
    trust_chain: Option<Vec<String>>,
    #[serde(default)]
    trajectory: TrajectoryToml,
    #[serde(default)]
    audiences: AudiencesToml,
    #[serde(default)]
    tools: Vec<ToolContract>,
    #[serde(default)]
    destinations: Vec<Destination>,
    #[serde(default)]
    sources: Vec<Source>,
    #[serde(default)]
    sanitizers: Vec<Sanitizer>,
    #[serde(default)]
    authorities: Vec<Authority>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrajectoryToml {
    start_audience: Option<Vec<String>>,
    start_trust: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AudiencesToml {
    #[serde(default, rename = "self")]
    self_: Vec<String>,
    #[serde(default)]
    internal: Vec<String>,
    #[serde(default)]
    groups: BTreeMap<String, GroupToml>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupToml {
    #[serde(default)]
    members: Vec<String>,
    within: Option<String>,
}

/// A tool's flow contract.
///
/// - `delta`: joined into the trajectory label when the tool
///   *returns* — how its results restrict the trajectory.
/// - `forwards`: the label of what the tool sends onward to an
///   effector beyond its visible arguments, if anything.
/// - `requires`: preconditions checked before dispatch.
/// - `effects`: capability strings recorded on success.
/// - `tags`: labels authorities and sanitizers can select by.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolContract {
    /// Tool name as dispatched.
    pub name: String,
    /// Result delta (joined on return).
    #[serde(default)]
    pub delta: Option<LabelDelta>,
    /// Forwarded-content label, if the tool forwards content.
    #[serde(default)]
    pub forwards: Option<LabelDelta>,
    /// Pre-dispatch requirements.
    #[serde(default)]
    pub requires: Requires,
    /// Effects recorded on success.
    #[serde(default)]
    pub effects: Vec<String>,
    /// Selection tags for authorities/sanitizers.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Pre-dispatch requirements on a tool call.
///
/// `audience_contains` entries may be `$argument` placeholders —
/// resolved per call from the call's arguments — and are only legal
/// here (OpenAPPA restricts placeholders to `requires`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requires {
    /// Readers that must be in the send label's audience.
    #[serde(default)]
    pub audience_contains: Vec<String>,
    /// Minimum trust rank name.
    #[serde(default)]
    pub trust: Option<String>,
    /// Effects that must already be recorded.
    #[serde(default)]
    pub effects_contains: Vec<String>,
    /// Effects that must not be recorded.
    #[serde(default)]
    pub effects_excludes: Vec<String>,
    /// Attention marks that need a live authority approval.
    #[serde(default)]
    pub attention: Vec<String>,
}

/// A named message destination: who reads it, and the least trust a
/// payload may carry to be sent there.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    /// Destination name as addressed by agents
    /// (e.g. `agent:planner`, `external:weather-bot`).
    pub name: String,
    /// Reader tokens that receive at this destination.
    pub readers: Vec<String>,
    /// Minimum trust rank for payloads sent here, if any.
    #[serde(default)]
    pub min_trust: Option<String>,
}

/// A named inbound source: the delta joined into the receiving
/// trajectory's label when content from this source is read.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// Source name (e.g. `external:weather-bot`, `file:notes`).
    pub name: String,
    /// The delta this source's content carries.
    pub delta: LabelDelta,
}

/// A payload transform offered as a remedy.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sanitizer {
    /// Sanitizer name.
    pub name: String,
    /// Tools it applies to (exact names).
    #[serde(default)]
    pub tools: Vec<String>,
    /// Tool tags it applies to.
    #[serde(default)]
    pub tags: Vec<String>,
    /// What it does to the payload.
    pub kind: SanitizerKind,
    /// Argument fields to redact (`redact_fields` only).
    #[serde(default)]
    pub fields: Vec<String>,
    /// The declared label of the sanitized output; joined with the
    /// original delta on the retry.
    pub result_delta: LabelDelta,
}

/// The payload transforms this increment implements.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SanitizerKind {
    /// Replace the named argument fields' values with `[REDACTED]`.
    RedactFields,
    /// Drop every non-text part from a message (artifacts and files
    /// do not cross).
    DropParts,
}

/// An authority that may approve one gated action.
///
/// An approval satisfies attention marks and may override the
/// audience/trust requirements *for that one action*, within these
/// permits — and never changes the trajectory's label.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authority {
    /// Authority name (in phlow, the operator).
    pub name: String,
    /// Tools it may approve (exact names).
    #[serde(default)]
    pub permits_tools: Vec<String>,
    /// Tool tags it may approve.
    #[serde(default)]
    pub permits_tags: Vec<String>,
    /// Attention marks it may clear.
    #[serde(default)]
    pub permits_attention: Vec<String>,
}

impl Authority {
    /// May this authority approve a call to `contract`?
    pub fn permits_tool(&self, contract: &ToolContract) -> bool {
        self.permits_tools.iter().any(|tool| tool == &contract.name)
            || contract
                .tags
                .iter()
                .any(|tag| self.permits_tags.iter().any(|permitted| permitted == tag))
    }

    /// May this authority clear `mark`?
    pub fn permits_attention(&self, mark: &str) -> bool {
        self.permits_attention
            .iter()
            .any(|permitted| permitted == mark)
    }
}

impl Sanitizer {
    /// Does this sanitizer apply to `contract`?
    pub fn applies_to(&self, contract: &ToolContract) -> bool {
        self.tools.iter().any(|tool| tool == &contract.name)
            || contract
                .tags
                .iter()
                .any(|tag| self.tags.iter().any(|selected| selected == tag))
    }
}

// ---------------------------------------------------------------------
// The validated policy.

/// A loaded, validated policy. Every lookup the gate performs is
/// total over a `Policy` value; there is no other policy state.
#[derive(Debug)]
pub struct Policy {
    fingerprint: String,
    trust_chain: Vec<String>,
    start: Label,
    self_tokens: Vec<String>,
    internal_tokens: Vec<String>,
    groups: BTreeMap<String, GroupToml>,
    tools: BTreeMap<String, ToolContract>,
    destinations: BTreeMap<String, Destination>,
    sources: BTreeMap<String, Source>,
    sanitizers: Vec<Sanitizer>,
    authorities: Vec<Authority>,
}

impl Policy {
    /// Load and validate a policy from TOML text.
    ///
    /// # Errors
    ///
    /// [`FlowError::Toml`] for syntax faults; [`FlowError::Policy`]
    /// (or a more specific variant) for consistency faults. Nothing
    /// is partially loaded: either the whole policy validates or the
    /// load fails.
    pub fn from_toml(text: &str) -> Result<Policy, FlowError> {
        let parsed: PolicyToml =
            toml::from_str(text).map_err(|err| FlowError::Toml(err.to_string()))?;
        if parsed.version != 1 {
            return Err(FlowError::Policy(format!(
                "unsupported policy version {} (this build speaks version 1)",
                parsed.version
            )));
        }
        let trust_chain = parsed.trust_chain.unwrap_or_else(|| {
            TRUST_CHAIN_DEFAULT
                .iter()
                .map(|rank| (*rank).to_string())
                .collect()
        });
        if trust_chain.is_empty() || trust_chain.len() > 16 {
            return Err(FlowError::Policy(
                "trust_chain must name between 1 and 16 ranks".to_string(),
            ));
        }
        let mut seen_ranks = BTreeSet::new();
        for rank in &trust_chain {
            check_token(rank, "trust rank")?;
            if !seen_ranks.insert(rank.clone()) {
                return Err(FlowError::Policy(format!("duplicate trust rank: {rank}")));
            }
        }
        if parsed.tools.len() > POLICY_TOOLS_MAX {
            return Err(FlowError::Policy(format!(
                "policy declares {} tools; the bound is {POLICY_TOOLS_MAX}",
                parsed.tools.len()
            )));
        }
        let mut tools = BTreeMap::new();
        for contract in parsed.tools {
            check_token(&contract.name, "tool name")?;
            if tools.insert(contract.name.clone(), contract).is_some() {
                return Err(FlowError::Policy("duplicate tool contract".to_string()));
            }
        }
        let mut destinations = BTreeMap::new();
        for destination in parsed.destinations {
            check_token(&destination.name, "destination name")?;
            if destinations
                .insert(destination.name.clone(), destination)
                .is_some()
            {
                return Err(FlowError::Policy("duplicate destination".to_string()));
            }
        }
        let mut sources = BTreeMap::new();
        for source in parsed.sources {
            check_token(&source.name, "source name")?;
            if sources.insert(source.name.clone(), source).is_some() {
                return Err(FlowError::Policy("duplicate source".to_string()));
            }
        }
        let mut fingerprint_hasher = Sha256::new();
        fingerprint_hasher.update(text.as_bytes());
        let fingerprint = format!("{:x}", fingerprint_hasher.finalize());

        let mut policy = Policy {
            fingerprint,
            trust_chain,
            // Placeholder until the start label is resolved below;
            // resolution needs the audience tables in place.
            start: Label {
                readers: ReaderSet::All,
                trust: 0,
            },
            self_tokens: parsed.audiences.self_,
            internal_tokens: parsed.audiences.internal,
            groups: parsed.audiences.groups,
            tools,
            destinations,
            sources,
            sanitizers: parsed.sanitizers,
            authorities: parsed.authorities,
        };

        // The trajectory start label: policy-declared, or the top
        // label (public audience, highest trust).
        let start_audience = parsed
            .trajectory
            .start_audience
            .unwrap_or_else(|| vec!["public".to_string()]);
        let start_trust_name = parsed
            .trajectory
            .start_trust
            .unwrap_or_else(|| policy.trust_chain.last().cloned().unwrap_or_default());
        policy.start = Label {
            readers: policy.expand_audience(&start_audience)?,
            trust: policy.trust_rank(&start_trust_name)?,
        };

        policy.validate()?;
        Ok(policy)
    }

    fn validate(&self) -> Result<(), FlowError> {
        // Every token list is bounded and every token well-formed.
        for (name, group) in &self.groups {
            check_token(name, "group name")?;
            check_tokens(&group.members, "group members")?;
            // Expand every group once so unknown members and cycles
            // surface at load even if nothing references the group.
            self.expand_audience(&[format!("@{name}")])?;
            if let Some(within) = &group.within {
                // `within` is an assertion: every member's expansion
                // must sit inside the named builtin's expansion.
                let container = self.expand_audience(std::slice::from_ref(within))?;
                for member in &group.members {
                    let member_set = self.expand_audience(std::slice::from_ref(member))?;
                    if !container.is_superset_of(&member_set) {
                        return Err(FlowError::Policy(format!(
                            "group @{name}: member {member} is not within {within}"
                        )));
                    }
                }
            }
        }
        check_tokens(&self.self_tokens, "self audience")?;
        check_tokens(&self.internal_tokens, "internal audience")?;
        // The builtin layering self ⊆ internal must hold as declared.
        let self_set = self.expand_audience(&["self".to_string()])?;
        let internal_set = self.expand_audience(&["internal".to_string()])?;
        if !internal_set.is_superset_of(&self_set) {
            return Err(FlowError::Policy(
                "the self audience must be a subset of the internal audience".to_string(),
            ));
        }
        // Expand everything once: unknown tokens and cycles surface
        // here, at load, not mid-flight.
        for contract in self.tools.values() {
            for delta in [&contract.delta, &contract.forwards].into_iter().flatten() {
                self.resolve_delta(delta)?;
            }
            self.resolve_delta(&contract.requires.as_delta())?;
            for entry in &contract.requires.audience_contains {
                if !entry.starts_with('$') {
                    self.expand_audience(std::slice::from_ref(entry))?;
                }
            }
            for sanitizer in &self.sanitizers {
                if sanitizer.applies_to(contract) {
                    self.resolve_delta(&sanitizer.result_delta)?;
                }
            }
        }
        for destination in self.destinations.values() {
            check_tokens(&destination.readers, "destination readers")?;
            self.expand_audience(&destination.readers)?;
            if let Some(min_trust) = &destination.min_trust {
                self.trust_rank(min_trust)?;
            }
        }
        for source in self.sources.values() {
            self.resolve_delta(&source.delta)?;
        }
        for sanitizer in &self.sanitizers {
            self.resolve_delta(&sanitizer.result_delta)?;
        }
        Ok(())
    }

    /// SHA-256 of the policy's TOML text; binds approvals to the
    /// exact policy they were considered under.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// The trust chain, least → most trusted.
    pub fn trust_chain(&self) -> &[String] {
        &self.trust_chain
    }

    /// The rank index of a trust rank name.
    ///
    /// # Errors
    ///
    /// [`FlowError::UnknownTrustRank`] if the name is not in the chain.
    pub fn trust_rank(&self, name: &str) -> Result<usize, FlowError> {
        self.trust_chain
            .iter()
            .position(|rank| rank == name)
            .ok_or_else(|| FlowError::UnknownTrustRank(name.to_string()))
    }

    /// The rank name at `index`, if in range.
    pub fn trust_name(&self, index: usize) -> Option<&str> {
        self.trust_chain.get(index).map(String::as_str)
    }

    /// The trajectory start label declared by this policy.
    pub fn start_label(&self) -> Label {
        self.start.clone()
    }

    /// The contract for a tool, if declared.
    pub fn tool(&self, name: &str) -> Option<&ToolContract> {
        self.tools.get(name)
    }

    /// All declared tool names, sorted (BTreeMap order).
    pub fn tool_names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// The destination declaration for `name`, if declared.
    pub fn destination(&self, name: &str) -> Option<&Destination> {
        self.destinations.get(name)
    }

    /// The source declaration for `name`, if declared.
    pub fn source(&self, name: &str) -> Option<&Source> {
        self.sources.get(name)
    }

    /// The sanitizer declared under `name`, if any (existence
    /// lookup; applicability to a tool is [`Policy::sanitizers_for`]).
    pub fn sanitizer_named(&self, name: &str) -> Option<&Sanitizer> {
        self.sanitizers
            .iter()
            .find(|sanitizer| sanitizer.name == name)
    }

    /// Sanitizers applying to `contract`, in declaration order.
    pub fn sanitizers_for(&self, contract: &ToolContract) -> Vec<&Sanitizer> {
        self.sanitizers
            .iter()
            .filter(|sanitizer| sanitizer.applies_to(contract))
            .collect()
    }

    /// All declared authorities, in declaration order.
    pub fn authorities(&self) -> &[Authority] {
        &self.authorities
    }

    /// Check an operator tool inventory against the contracts.
    ///
    /// Returns the sorted list of inventory tools with **no**
    /// contract. Those tools will be refused by the gate, so a
    /// non-empty result is a policy bug to fix before deployment.
    pub fn validate_coverage(&self, inventory: &[String]) -> Vec<String> {
        let mut uncovered: Vec<String> = inventory
            .iter()
            .filter(|tool| !self.tools.contains_key(*tool))
            .cloned()
            .collect();
        uncovered.sort();
        uncovered.dedup();
        uncovered
    }

    /// Expand audience tokens to their reader set.
    ///
    /// # Errors
    ///
    /// [`FlowError::UnknownAudienceToken`] for an undeclared
    /// `@group`; [`FlowError::AudienceCycle`] for a group cycle.
    pub fn expand_audience(&self, tokens: &[String]) -> Result<ReaderSet, FlowError> {
        let mut expanded = ReaderSet::empty();
        for token in tokens {
            expanded = expanded.union(&self.expand_token(token, 0)?);
        }
        Ok(expanded)
    }

    fn expand_token(&self, token: &str, depth: usize) -> Result<ReaderSet, FlowError> {
        if depth > EXPANSION_DEPTH_MAX {
            return Err(FlowError::AudienceCycle(token.to_string()));
        }
        match token {
            "public" => Ok(ReaderSet::All),
            "self" => self.expand_tokens(&self.self_tokens, depth + 1),
            "internal" => self.expand_tokens(&self.internal_tokens, depth + 1),
            _ => {
                if let Some(group_name) = token.strip_prefix('@') {
                    let group = self
                        .groups
                        .get(group_name)
                        .ok_or_else(|| FlowError::UnknownAudienceToken(token.to_string()))?;
                    self.expand_tokens(&group.members, depth + 1)
                } else {
                    let mut single = BTreeSet::new();
                    single.insert(token.to_string());
                    Ok(ReaderSet::Only(single))
                }
            }
        }
    }

    fn expand_tokens(&self, tokens: &[String], depth: usize) -> Result<ReaderSet, FlowError> {
        let mut expanded = ReaderSet::empty();
        for token in tokens {
            expanded = expanded.union(&self.expand_token(token, depth)?);
        }
        Ok(expanded)
    }

    /// Resolve a delta to the label it contributes: the expansion of
    /// its audience tokens at the named trust rank. Fields the delta
    /// leaves open contribute the top element (no constraint).
    ///
    /// # Errors
    ///
    /// Propagates expansion and rank errors.
    pub fn resolve_delta(&self, delta: &LabelDelta) -> Result<Label, FlowError> {
        let readers = if delta.audience.is_empty() {
            ReaderSet::All
        } else {
            self.expand_audience(&delta.audience)?
        };
        let trust = match &delta.trust {
            Some(name) => self.trust_rank(name)?,
            None => self.trust_chain.len().saturating_sub(1),
        };
        Ok(Label { readers, trust })
    }

    /// Resolve a concrete snapshot into this policy's label space.
    /// A snapshot whose trust name is unknown here fails closed.
    ///
    /// # Errors
    ///
    /// [`FlowError::UnknownTrustRank`] for an unknown rank name.
    pub fn resolve_snapshot(&self, snapshot: &LabelSnapshot) -> Result<Label, FlowError> {
        Ok(Label {
            readers: snapshot.readers.clone(),
            trust: self.trust_rank(&snapshot.trust)?,
        })
    }

    /// Render a label as a snapshot under this policy (for handoffs).
    pub fn snapshot_of(&self, label: &Label) -> LabelSnapshot {
        LabelSnapshot {
            readers: label.readers.clone(),
            trust: self
                .trust_name(label.trust)
                .unwrap_or("suspicious")
                .to_string(),
        }
    }
}

impl Requires {
    /// The requires clause viewed as a delta (for load-time
    /// resolution checks): its literal audience entries and its trust
    /// floor. Placeholders are skipped — they resolve per call.
    pub(crate) fn as_delta(&self) -> LabelDelta {
        LabelDelta {
            audience: self
                .audience_contains
                .iter()
                .filter(|entry| !entry.starts_with('$'))
                .cloned()
                .collect(),
            trust: self.trust.clone(),
        }
    }
}

fn check_token(token: &str, what: &str) -> Result<(), FlowError> {
    if token.is_empty()
        || token.len() > TOKEN_BYTES_MAX
        || token
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err(FlowError::Policy(format!(
            "invalid {what}: {token:?} (empty, over {TOKEN_BYTES_MAX} bytes, or contains whitespace/control characters)"
        )));
    }
    Ok(())
}

fn check_tokens(tokens: &[String], what: &str) -> Result<(), FlowError> {
    if tokens.len() > READER_TOKENS_MAX {
        return Err(FlowError::Policy(format!(
            "{what}: {} tokens exceed the bound of {READER_TOKENS_MAX}",
            tokens.len()
        )));
    }
    for token in tokens {
        check_token(token, what)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICY_BASIC: &str = r#"
version = 1
trust_chain = ["suspicious", "trusted"]

[audiences]
self = ["op@example.com"]
internal = ["op@example.com", "dev@example.com", "cfo@example.com"]

[audiences.groups.finance]
members = ["op@example.com", "cfo@example.com"]
within = "internal"

[[tools]]
name = "read_note"
delta = { audience = ["self"] }

[[destinations]]
name = "agent:peer"
readers = ["internal"]
"#;

    #[test]
    fn loads_and_expands_builtins_and_groups() {
        let policy = Policy::from_toml(POLICY_BASIC).expect("policy loads");
        let finance = policy
            .expand_audience(&["@finance".to_string()])
            .expect("group expands");
        assert!(finance.contains("cfo@example.com"));
        assert!(finance.contains("op@example.com"));
        assert!(!finance.contains("dev@example.com"));
        let internal = policy
            .expand_audience(&["internal".to_string()])
            .expect("builtin expands");
        assert!(internal.contains("dev@example.com"));
        assert!(internal.contains("cfo@example.com"));
        assert_eq!(
            policy
                .expand_audience(&["public".to_string()])
                .expect("public"),
            ReaderSet::All
        );
    }

    #[test]
    fn within_assertion_is_enforced() {
        // Drop cfo@example.com from `internal` while @finance still
        // claims within = "internal": the assertion must fail at load.
        let broken = POLICY_BASIC.replace(
            "internal = [\"op@example.com\", \"dev@example.com\", \"cfo@example.com\"]",
            "internal = [\"op@example.com\", \"dev@example.com\"]",
        );
        assert!(Policy::from_toml(&broken).is_err());
    }

    #[test]
    fn group_cycle_is_rejected_at_load() {
        let text = r#"
version = 1
[audiences.groups.loop]
members = ["@loop"]
[[tools]]
name = "t"
delta = { audience = ["@loop"] }
"#;
        assert!(Policy::from_toml(text).is_err());
    }

    #[test]
    fn unknown_trust_rank_is_rejected() {
        let text = "version = 1\n[[tools]]\nname = \"t\"\ndelta = { trust = \"divine\" }\n";
        assert!(matches!(
            Policy::from_toml(text),
            Err(FlowError::UnknownTrustRank(_))
        ));
    }

    #[test]
    fn self_must_be_within_internal() {
        let text = r#"
version = 1
[audiences]
self = ["outsider@example.com"]
internal = ["op@example.com"]
"#;
        assert!(Policy::from_toml(text).is_err());
    }

    #[test]
    fn coverage_reports_undeclared_tools() {
        let policy = Policy::from_toml(POLICY_BASIC).expect("policy loads");
        let inventory = vec![
            "read_note".to_string(),
            "shell_exec".to_string(),
            "send_email".to_string(),
        ];
        assert_eq!(
            policy.validate_coverage(&inventory),
            vec!["send_email".to_string(), "shell_exec".to_string()]
        );
    }

    #[test]
    fn duplicate_tool_contract_is_rejected() {
        let text = "version = 1\n[[tools]]\nname = \"t\"\n[[tools]]\nname = \"t\"\n";
        assert!(Policy::from_toml(text).is_err());
    }

    #[test]
    fn default_chain_is_suspicious_then_trusted() {
        let policy = Policy::from_toml("version = 1\n").expect("minimal policy loads");
        assert_eq!(policy.trust_chain(), ["suspicious", "trusted"]);
        assert_eq!(policy.start_label(), Label::top(2));
    }
}
