//! The deterministic pre-dispatch gate.
//!
//! Plain words: [`decide`] is a pure function of `(policy, log,
//! action)`. It folds the log to the trajectory's label, joins the
//! action's forwarded-content label, and checks the contract's
//! requirements in a fixed order — contract exists → audience →
//! trust → effects → attention — reporting *every* failure, not the
//! first. A live, unconsumed, digest-bound authority approval can
//! clear audience, trust, and attention failures for that one
//! action; it never clears effect failures (those are facts about
//! what has happened, not permissions) and it never changes the
//! label.
//!
//! A block carries a [`RemedyPlan`]: the sanitizers that apply (with
//! an honest preview of whether sanitizing would make the call
//! pass), the authorities that could approve it and the marks they
//! could clear, and — for tool calls — the isolation option. The
//! reserved `blocked` attention mark short-circuits all of it: no
//! approval covers it and no remedy is offered.
//!
//! Two mechanisms keep this replayable rather than advisory:
//!
//! - **Approvals are single-use.** An approval is *live* until the
//!   first later `ToolResult` bearing the same action digest; after
//!   that it is spent, and the identical call blocks again.
//! - **Sanitizers are single-shot.** A `Sanitized` event arms its
//!   declared result label for the next decision on an applicable
//!   tool, and is consumed by the next recorded `ToolResult` — the
//!   same consumption point as approvals, so a dispatch-point
//!   re-decision over the post-verdict log (which already contains
//!   the `ToolAllowed`) sees the same armed state the session saw.
//!   The sanitized payload's send label *is* the declared result
//!   delta — that is the point of the transform — while the
//!   trajectory label still joins the result delta for everything
//!   after.

use sha2::{Digest, Sha256};

use crate::ATTENTION_BLOCKED;
use crate::error::FlowError;
use crate::event::{Event, ProposedAction};
use crate::label::{Label, ReaderSet};
use crate::policy::{Policy, Sanitizer, SanitizerKind, ToolContract};
use crate::trajectory::fold;

/// The gate's verdict on one proposed action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Decision {
    /// The action may proceed. Carries the label the outgoing
    /// material bears and the effects to record on success.
    Allow(AllowInfo),
    /// The action must not proceed as proposed. Carries every
    /// failing check and the remedy plan.
    Block(BlockInfo),
}

impl Decision {
    /// The digest of the decided action (as recorded in the log).
    pub fn digest(&self) -> &str {
        match self {
            Decision::Allow(info) => &info.digest,
            Decision::Block(info) => &info.digest,
        }
    }

    /// Did the gate allow the action?
    pub fn is_allow(&self) -> bool {
        matches!(self, Decision::Allow(_))
    }
}

/// An allow verdict.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllowInfo {
    /// Digest of the allowed action.
    pub digest: String,
    /// The label the outgoing material bears for this action.
    pub send_label: Label,
    /// Effects the contract records on success.
    pub effects: Vec<String>,
    /// The authority whose approval cleared the way, if any.
    pub via_approval: Option<String>,
}

/// A block verdict.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockInfo {
    /// Digest of the blocked action.
    pub digest: String,
    /// Every failing check, in check order.
    pub reasons: Vec<BlockReason>,
    /// What could be done instead.
    pub remedies: RemedyPlan,
}

/// One failing check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockReason {
    /// The tool has no contract in policy (fail closed; a policy
    /// bug, fixable only by the policy author).
    NoContract {
        /// The undeclared tool.
        tool: String,
    },
    /// The destination is not declared in policy.
    NoDestination {
        /// The undeclared destination.
        destination: String,
    },
    /// Readers the destination requires that the send label's
    /// audience does not cover.
    AudienceMissing {
        /// The uncovered readers, sorted.
        readers: Vec<String>,
    },
    /// A `$argument` placeholder could not be resolved to a reader
    /// (missing argument, non-string argument, or an expansion that
    /// does not resolve) — the gate cannot prove the destination,
    /// so it refuses.
    AudienceUnresolvable {
        /// The placeholder name (without `$`).
        placeholder: String,
    },
    /// The send label's trust is below the required floor.
    TrustTooLow {
        /// The send label's trust rank name.
        have: String,
        /// The required rank name.
        need: String,
    },
    /// Effects the contract requires that the log has not recorded.
    EffectsMissing {
        /// The missing effects, sorted.
        effects: Vec<String>,
    },
    /// Effects the contract excludes that the log has recorded.
    EffectsExcluded {
        /// The offending effects, sorted.
        effects: Vec<String>,
    },
    /// Attention marks requiring a live authority approval.
    AttentionRequired {
        /// The uncovered marks, sorted.
        marks: Vec<String>,
    },
    /// The log and the policy no longer agree (an event names
    /// something undeclared). Fail closed.
    PolicyMismatch {
        /// What disagreed.
        detail: String,
    },
}

/// A machine-readable plan for getting a blocked action done some
/// other way. Executing any option ends by re-running the gate;
/// there is no path around a second decision.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RemedyPlan {
    /// The available remedies, in preference order (sanitize,
    /// approve, isolate).
    pub options: Vec<RemedyOption>,
}

/// One remedy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemedyOption {
    /// Transform the payload with this sanitizer and retry.
    Sanitize {
        /// The sanitizer's name.
        name: String,
        /// Honest preview: would the retried action pass the
        /// audience and trust checks under the sanitizer's declared
        /// result label?
        would_allow: bool,
    },
    /// Ask this authority to approve the one action.
    RequestApproval {
        /// The authority's name.
        authority: String,
        /// Attention marks this approval would clear (may be empty
        /// when the approval's value is overriding audience/trust).
        marks: Vec<String>,
    },
    /// Run the step in a disposable sub-agent trajectory whose
    /// result — if the parent accepts it at all — joins the child's
    /// end label into the parent's.
    Isolate,
}

/// The action digest: SHA-256 over the policy fingerprint and the
/// action's canonical JSON. Binding the policy fingerprint means an
/// approval granted under one policy text is meaningless under
/// another; binding the full action means any argument change is a
/// different action.
pub fn action_digest(policy: &Policy, action: &ProposedAction) -> String {
    let canonical = match action {
        ProposedAction::ToolCall { tool, args } => {
            serde_json::json!({ "kind": "tool", "tool": tool, "args": args })
        }
        ProposedAction::Message {
            destination,
            payload_label,
        } => serde_json::json!({
            "kind": "message",
            "destination": destination,
            "payload_label": payload_label,
        }),
    };
    let mut hasher = Sha256::new();
    hasher.update(policy.fingerprint().as_bytes());
    hasher.update(b"\n");
    hasher.update(canonical.to_string().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Decide one action against a policy and a log. Pure: the same
/// inputs always produce the same decision.
pub fn decide(policy: &Policy, log: &[Event], action: &ProposedAction) -> Decision {
    let digest = action_digest(policy, action);
    let folded = match fold(policy, log) {
        Ok(folded) => folded,
        Err(err) => {
            return Decision::Block(BlockInfo {
                digest,
                reasons: vec![BlockReason::PolicyMismatch {
                    detail: err.to_string(),
                }],
                remedies: RemedyPlan::default(),
            });
        }
    };
    match action {
        ProposedAction::ToolCall { tool, args } => {
            decide_tool(policy, log, &folded, tool, args, digest)
        }
        ProposedAction::Message {
            destination,
            payload_label,
        } => decide_message(policy, log, &folded, destination, payload_label, digest),
    }
}

// ---------------------------------------------------------------------
// Tool calls.

#[allow(clippy::too_many_arguments)]
fn decide_tool(
    policy: &Policy,
    log: &[Event],
    folded: &crate::trajectory::Fold,
    tool: &str,
    args: &serde_json::Value,
    digest: String,
) -> Decision {
    let Some(contract) = policy.tool(tool) else {
        return Decision::Block(BlockInfo {
            digest,
            reasons: vec![BlockReason::NoContract {
                tool: tool.to_string(),
            }],
            remedies: RemedyPlan::default(),
        });
    };
    // A sanitizer armed by the log replaces the send label with its
    // declared result label and the arguments with their sanitized
    // form, for this one decision.
    let armed = armed_sanitizer(policy, log, contract);
    let effective_args;
    let base_label = match &armed {
        Some(sanitizer) => {
            effective_args =
                apply_sanitizer(policy, &sanitizer.name, args).unwrap_or_else(|_| args.clone());
            match policy.resolve_delta(&sanitizer.result_delta) {
                Ok(label) => label,
                Err(err) => return policy_mismatch(digest, &err),
            }
        }
        None => {
            effective_args = args.clone();
            let mut label = folded.label.clone();
            if let Some(forwards) = &contract.forwards {
                match policy.resolve_delta(forwards) {
                    Ok(forwards_label) => label = label.join(&forwards_label),
                    Err(err) => return policy_mismatch(digest, &err),
                }
            }
            label
        }
    };
    let mut reasons = check_tool(policy, folded, contract, &base_label, &effective_args);
    if reasons.is_empty() {
        return allow(digest, base_label, contract, None);
    }
    // A live approval may clear audience/trust/attention failures.
    if let Some((authority_name, marks)) = live_approval(policy, log, &digest)
        && let Some(authority) = policy
            .authorities()
            .iter()
            .find(|candidate| candidate.name == authority_name)
        && authority.permits_tool(contract)
    {
        reasons.retain(|reason| !approval_clears(authority, &marks, reason));
        if reasons.is_empty() {
            return allow(digest, base_label, contract, Some(authority_name));
        }
    }
    let remedies = if contract
        .requires
        .attention
        .iter()
        .any(|mark| mark == ATTENTION_BLOCKED)
    {
        RemedyPlan::default()
    } else {
        tool_remedies(policy, contract, args, &reasons)
    };
    Decision::Block(BlockInfo {
        digest,
        reasons,
        remedies,
    })
}

/// The audience/trust/effects/attention checks for a tool call,
/// against an already-computed send label and effective arguments.
fn check_tool(
    policy: &Policy,
    folded: &crate::trajectory::Fold,
    contract: &ToolContract,
    send_label: &Label,
    args: &serde_json::Value,
) -> Vec<BlockReason> {
    let mut reasons = Vec::new();
    // Audience.
    let (required, unresolvable) =
        required_readers(policy, &contract.requires.audience_contains, args);
    if let Some(placeholder) = unresolvable {
        reasons.push(BlockReason::AudienceUnresolvable { placeholder });
    }
    let missing = missing_readers(&required, &send_label.readers);
    if !missing.is_empty() {
        reasons.push(BlockReason::AudienceMissing { readers: missing });
    }
    // Trust.
    if let Some(need_name) = &contract.requires.trust
        && let Ok(need) = policy.trust_rank(need_name)
        && send_label.trust < need
    {
        reasons.push(BlockReason::TrustTooLow {
            have: policy
                .trust_name(send_label.trust)
                .unwrap_or("?")
                .to_string(),
            need: need_name.clone(),
        });
    }
    // Effects.
    let missing_effects: Vec<String> = contract
        .requires
        .effects_contains
        .iter()
        .filter(|effect| !folded.effects.contains(*effect))
        .cloned()
        .collect();
    if !missing_effects.is_empty() {
        reasons.push(BlockReason::EffectsMissing {
            effects: missing_effects,
        });
    }
    let excluded_present: Vec<String> = contract
        .requires
        .effects_excludes
        .iter()
        .filter(|effect| folded.effects.contains(*effect))
        .cloned()
        .collect();
    if !excluded_present.is_empty() {
        reasons.push(BlockReason::EffectsExcluded {
            effects: excluded_present,
        });
    }
    // Attention: marks are reported here against *no* approval; the
    // approval pass in decide_tool re-checks with live approvals.
    if !contract.requires.attention.is_empty() {
        reasons.push(BlockReason::AttentionRequired {
            marks: contract.requires.attention.clone(),
        });
    }
    reasons
}

fn tool_remedies(
    policy: &Policy,
    contract: &ToolContract,
    args: &serde_json::Value,
    reasons: &[BlockReason],
) -> RemedyPlan {
    let mut options = Vec::new();
    let overridable = reasons.iter().any(|reason| {
        matches!(
            reason,
            BlockReason::AudienceMissing { .. }
                | BlockReason::AudienceUnresolvable { .. }
                | BlockReason::TrustTooLow { .. }
                | BlockReason::AttentionRequired { .. }
        )
    });
    if overridable {
        for sanitizer in policy.sanitizers_for(contract) {
            let would_allow = match policy.resolve_delta(&sanitizer.result_delta) {
                Ok(candidate) => {
                    let sanitized_args = apply_sanitizer(policy, &sanitizer.name, args)
                        .unwrap_or_else(|_| args.clone());
                    let (required, unresolvable) = required_readers(
                        policy,
                        &contract.requires.audience_contains,
                        &sanitized_args,
                    );
                    unresolvable.is_none()
                        && missing_readers(&required, &candidate.readers).is_empty()
                        && contract.requires.trust.as_ref().is_none_or(|need| {
                            policy
                                .trust_rank(need)
                                .is_ok_and(|rank| candidate.trust >= rank)
                        })
                }
                Err(_) => false,
            };
            options.push(RemedyOption::Sanitize {
                name: sanitizer.name.clone(),
                would_allow,
            });
        }
        for authority in policy.authorities() {
            if authority.permits_tool(contract) {
                let marks: Vec<String> = contract
                    .requires
                    .attention
                    .iter()
                    .filter(|mark| authority.permits_attention(mark))
                    .cloned()
                    .collect();
                options.push(RemedyOption::RequestApproval {
                    authority: authority.name.clone(),
                    marks,
                });
            }
        }
    }
    options.push(RemedyOption::Isolate);
    RemedyPlan { options }
}

// ---------------------------------------------------------------------
// Messages.

fn decide_message(
    policy: &Policy,
    log: &[Event],
    folded: &crate::trajectory::Fold,
    destination: &str,
    payload_label: &Option<crate::label::LabelSnapshot>,
    digest: String,
) -> Decision {
    let Some(declared) = policy.destination(destination) else {
        return Decision::Block(BlockInfo {
            digest,
            reasons: vec![BlockReason::NoDestination {
                destination: destination.to_string(),
            }],
            remedies: RemedyPlan::default(),
        });
    };
    let mut send_label = folded.label.clone();
    if let Some(snapshot) = payload_label {
        match policy.resolve_snapshot(snapshot) {
            Ok(payload) => send_label = send_label.join(&payload),
            Err(err) => return policy_mismatch(digest, &err),
        }
    }
    let dest_readers = match policy.expand_audience(&declared.readers) {
        Ok(readers) => readers,
        Err(err) => return policy_mismatch(digest, &err),
    };
    let mut reasons = Vec::new();
    let missing = missing_readers(&dest_readers, &send_label.readers);
    if !missing.is_empty() {
        reasons.push(BlockReason::AudienceMissing { readers: missing });
    }
    if let Some(need_name) = &declared.min_trust
        && let Ok(need) = policy.trust_rank(need_name)
        && send_label.trust < need
    {
        reasons.push(BlockReason::TrustTooLow {
            have: policy
                .trust_name(send_label.trust)
                .unwrap_or("?")
                .to_string(),
            need: need_name.clone(),
        });
    }
    if reasons.is_empty() {
        return Decision::Allow(AllowInfo {
            digest,
            send_label,
            effects: Vec::new(),
            via_approval: None,
        });
    }
    // Approval override: an authority that lists this destination
    // among its permitted names may approve the one send.
    if let Some((authority_name, _marks)) = live_approval(policy, log, &digest)
        && policy.authorities().iter().any(|authority| {
            authority.name == authority_name
                && authority
                    .permits_tools
                    .iter()
                    .any(|name| name == destination)
        })
    {
        return Decision::Allow(AllowInfo {
            digest,
            send_label,
            effects: Vec::new(),
            via_approval: Some(authority_name),
        });
    }
    let mut options = Vec::new();
    for authority in policy.authorities() {
        if authority
            .permits_tools
            .iter()
            .any(|name| name == destination)
        {
            options.push(RemedyOption::RequestApproval {
                authority: authority.name.clone(),
                marks: Vec::new(),
            });
        }
    }
    options.push(RemedyOption::Isolate);
    Decision::Block(BlockInfo {
        digest,
        reasons,
        remedies: RemedyPlan { options },
    })
}

// ---------------------------------------------------------------------
// Shared helpers.

fn allow(
    digest: String,
    send_label: Label,
    contract: &ToolContract,
    via_approval: Option<String>,
) -> Decision {
    Decision::Allow(AllowInfo {
        digest,
        send_label,
        effects: contract.effects.clone(),
        via_approval,
    })
}

fn policy_mismatch(digest: String, err: &FlowError) -> Decision {
    Decision::Block(BlockInfo {
        digest,
        reasons: vec![BlockReason::PolicyMismatch {
            detail: err.to_string(),
        }],
        remedies: RemedyPlan::default(),
    })
}

/// Resolve `requires.audience_contains` entries against the call
/// arguments: `$name` reads `args[name]` as a reader token; other
/// entries are tokens directly. Returns the required reader set and
/// the first unresolvable placeholder, if any.
fn required_readers(
    policy: &Policy,
    entries: &[String],
    args: &serde_json::Value,
) -> (ReaderSet, Option<String>) {
    let mut required = ReaderSet::empty();
    for entry in entries {
        let token = if let Some(name) = entry.strip_prefix('$') {
            match args.get(name).and_then(serde_json::Value::as_str) {
                Some(value) => value.to_string(),
                None => return (required, Some(name.to_string())),
            }
        } else {
            entry.clone()
        };
        match policy.expand_audience(&[token]) {
            Ok(expanded) => required = required.union(&expanded),
            Err(_) => {
                return (
                    required,
                    Some(entry.strip_prefix('$').unwrap_or(entry).to_string()),
                );
            }
        }
    }
    (required, None)
}

/// The members of `required` not covered by `available`, sorted.
/// An unmet `All` requirement reports the single token `public`.
fn missing_readers(required: &ReaderSet, available: &ReaderSet) -> Vec<String> {
    match required {
        ReaderSet::All => match available {
            ReaderSet::All => Vec::new(),
            ReaderSet::Only(_) => vec!["public".to_string()],
        },
        ReaderSet::Only(set) => set
            .iter()
            .filter(|reader| !available.contains(reader))
            .cloned()
            .collect(),
    }
}

/// The live approval for `digest`: an `ApprovalGranted` with no
/// later `ToolResult` bearing the same digest, issued by an
/// authority the policy still declares. Returns the authority name
/// and the marks the approval lists.
fn live_approval(policy: &Policy, log: &[Event], digest: &str) -> Option<(String, Vec<String>)> {
    let mut live = None;
    for event in log {
        match event {
            Event::ApprovalGranted {
                authority,
                digest: approved,
                marks,
            } if approved == digest
                && policy
                    .authorities()
                    .iter()
                    .any(|declared| declared.name == *authority) =>
            {
                live = Some((authority.clone(), marks.clone()));
            }
            Event::ToolResult { digest: spent, .. } if spent == digest => {
                live = None;
            }
            _ => {}
        }
    }
    live
}

/// Does this live approval clear this reason? Audience and trust
/// failures clear outright; attention failures clear per listed,
/// permitted mark; effect failures never clear.
fn approval_clears(
    authority: &crate::policy::Authority,
    marks: &[String],
    reason: &BlockReason,
) -> bool {
    match reason {
        BlockReason::AudienceMissing { .. }
        | BlockReason::AudienceUnresolvable { .. }
        | BlockReason::TrustTooLow { .. } => true,
        BlockReason::AttentionRequired { marks: required } => required.iter().all(|mark| {
            mark != ATTENTION_BLOCKED
                && authority.permits_attention(mark)
                && marks.iter().any(|granted| granted == mark)
        }),
        _ => false,
    }
}

/// The sanitizer armed for this decision: the latest `Sanitized`
/// event naming a sanitizer that applies to `contract`, with no
/// `ToolResult` after it (a result means the sanitized call has
/// executed and spent the arming).
fn armed_sanitizer<'p>(
    policy: &'p Policy,
    log: &[Event],
    contract: &ToolContract,
) -> Option<&'p Sanitizer> {
    let mut armed = None;
    for event in log {
        match event {
            Event::Sanitized { sanitizer, .. } => {
                if let Some(found) = policy
                    .sanitizers_for(contract)
                    .into_iter()
                    .find(|candidate| candidate.name == *sanitizer)
                {
                    armed = Some(found);
                }
            }
            Event::ToolResult { .. } => {
                armed = None;
            }
            _ => {}
        }
    }
    armed
}

/// Apply a named sanitizer to tool-call (or message) arguments.
///
/// - `RedactFields`: each named field present in the argument object
///   is replaced with `"[REDACTED]"`.
/// - `DropParts`: an argument object's `parts` array keeps only its
///   text parts.
///
/// # Errors
///
/// [`FlowError::UnknownName`] if the policy declares no sanitizer by
/// that name.
pub fn apply_sanitizer(
    policy: &Policy,
    name: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, FlowError> {
    let sanitizer = policy
        .sanitizer_named(name)
        .ok_or_else(|| FlowError::UnknownName(name.to_string()))?;
    let mut out = args.clone();
    match sanitizer.kind {
        SanitizerKind::RedactFields => {
            if let Some(object) = out.as_object_mut() {
                for field in &sanitizer.fields {
                    if object.contains_key(field) {
                        object.insert(
                            field.clone(),
                            serde_json::Value::String("[REDACTED]".to_string()),
                        );
                    }
                }
            }
        }
        SanitizerKind::DropParts => {
            if let Some(parts) = out
                .as_object_mut()
                .and_then(|object| object.get_mut("parts"))
                .and_then(serde_json::Value::as_array_mut)
            {
                parts.retain(|part| part.get("text").is_some());
            }
        }
    }
    Ok(out)
}
