//! Gate scenarios: half validation (the machinery allows what it
//! should), half adversarial (the named attacks fail). The four
//! attacks from the A2A program spec — laundering, injection,
//! stripping, downgrade — each have a dedicated test here at the
//! algebra level; the runtime-level versions live in phlow-runtime's
//! multiagent tests.

use std::sync::Arc;

use phlow_flowguard::event::{Event, JoinSpec, ProposedAction};
use phlow_flowguard::gate::{BlockReason, Decision, RemedyOption};
use phlow_flowguard::label::LabelDelta;
use phlow_flowguard::{Policy, Trajectory};
use serde_json::json;

const POLICY: &str = r#"
version = 1
trust_chain = ["suspicious", "trusted"]

[audiences]
self = ["op@example.com"]
internal = ["op@example.com", "dev@example.com", "cfo@example.com"]

[audiences.groups.finance]
members = ["op@example.com", "cfo@example.com"]
within = "internal"

[[sources]]
name = "file:secret"
delta = { audience = ["self"] }

[[sources]]
name = "external:peer"
delta = { trust = "suspicious" }

[[sources]]
name = "web:feed"
delta = { trust = "suspicious" }

[[destinations]]
name = "agent:peer"
readers = ["internal"]

[[destinations]]
name = "external:world"
readers = ["public"]

[[tools]]
name = "send_email"
requires = { audience_contains = ["$recipient"] }

[[tools]]
name = "deploy"
requires = { trust = "trusted", effects_contains = ["tests-passed"] }

[[tools]]
name = "publish_digest"
requires = { audience_contains = ["internal"], trust = "trusted" }

[[tools]]
name = "release"
requires = { attention = ["human-review"] }

[[tools]]
name = "wipe_disk"
requires = { attention = ["blocked"] }

[[sanitizers]]
name = "scrub"
tools = ["publish_digest"]
kind = "drop_parts"
result_delta = { audience = ["public"], trust = "trusted" }

[[authorities]]
name = "operator"
permits_tools = ["send_email", "deploy", "publish_digest", "release", "wipe_disk"]
permits_attention = ["human-review"]
"#;

fn policy() -> Arc<Policy> {
    Arc::new(Policy::from_toml(POLICY).expect("test policy loads"))
}

fn email(recipient: &str) -> ProposedAction {
    ProposedAction::ToolCall {
        tool: "send_email".to_string(),
        args: json!({ "recipient": recipient, "body": "quarterly numbers" }),
    }
}

fn block_reasons(decision: &Decision) -> Vec<BlockReason> {
    match decision {
        Decision::Block(info) => info.reasons.clone(),
        Decision::Allow(info) => panic!("expected a block, got allow: {info:?}"),
    }
}

// ----------------------------------------------------------------
// Validation: what should pass, passes.

#[test]
fn clean_trajectory_may_email_anyone() {
    let mut trajectory = Trajectory::new(policy());
    let decision = trajectory
        .propose(&email("cfo@example.com"))
        .expect("propose");
    assert!(decision.is_allow(), "{decision:?}");
}

#[test]
fn message_to_declared_destination_passes_when_clean() {
    let mut trajectory = Trajectory::new(policy());
    let action = ProposedAction::Message {
        destination: "external:world".to_string(),
        payload_label: None,
    };
    let decision = trajectory.propose(&action).expect("propose");
    assert!(decision.is_allow(), "{decision:?}");
}

#[test]
fn effects_requirement_is_satisfiable_by_recording_the_effect() {
    let p = policy();
    let mut trajectory = Trajectory::new(p);
    let deploy = ProposedAction::ToolCall {
        tool: "deploy".to_string(),
        args: json!({}),
    };
    let decision = trajectory.propose(&deploy).expect("propose");
    assert!(matches!(
        block_reasons(&decision).as_slice(),
        [BlockReason::EffectsMissing { .. }]
    ));
    trajectory
        .record(Event::EffectRecorded {
            effect: "tests-passed".to_string(),
        })
        .expect("record");
    let decision = trajectory.propose(&deploy).expect("propose");
    assert!(decision.is_allow(), "{decision:?}");
}

// ----------------------------------------------------------------
// Adversarial (i): the two-agent laundering attack.

#[test]
fn laundering_through_a_clean_agent_is_blocked() {
    let p = policy();
    // Agent A reads the private file: its label narrows to `self`.
    let mut agent_a = Trajectory::new(p.clone());
    agent_a.record_read("file:secret").expect("read");
    let a_label = agent_a.snapshot().expect("snapshot");

    // A's own send to an outside recipient is blocked.
    let decision = agent_a
        .propose(&email("ext@evil.example"))
        .expect("propose");
    assert!(matches!(
        block_reasons(&decision).as_slice(),
        [BlockReason::AudienceMissing { .. }]
    ));

    // Agent B is clean and *could* send — until it receives A's
    // data. The handoff joins A's label into B's, so B's identical
    // send is now blocked too. This join is the whole defence.
    let mut agent_b = Trajectory::new(p.clone());
    let clean = agent_b
        .propose(&email("ext@evil.example"))
        .expect("propose");
    assert!(clean.is_allow(), "B starts clean: {clean:?}");

    let mut agent_b2 = Trajectory::new(p);
    agent_b2
        .record_message_received("agent:a", JoinSpec::Snapshot(a_label))
        .expect("receive");
    let decision = agent_b2
        .propose(&email("ext@evil.example"))
        .expect("propose");
    assert!(
        matches!(
            block_reasons(&decision).as_slice(),
            [BlockReason::AudienceMissing { .. }]
        ),
        "laundering must fail: {decision:?}"
    );
}

// ----------------------------------------------------------------
// Adversarial (ii): prompt injection via external content.

#[test]
fn injected_external_content_lowers_trust_and_cannot_authorize() {
    let p = policy();
    let mut trajectory = Trajectory::new(p);
    // Content arrives from an unvetted external peer: trust drops.
    trajectory
        .record_message_received(
            "external:peer",
            JoinSpec::Delta(LabelDelta::trust("suspicious")),
        )
        .expect("receive");
    trajectory
        .record(Event::EffectRecorded {
            effect: "tests-passed".to_string(),
        })
        .expect("record");
    // The injected content now "instructs" a deploy. The gate does
    // not read instructions; the label says the trajectory's trust
    // is too low, full stop.
    let deploy = ProposedAction::ToolCall {
        tool: "deploy".to_string(),
        args: json!({}),
    };
    let decision = trajectory.propose(&deploy).expect("propose");
    assert!(
        block_reasons(&decision)
            .iter()
            .any(|reason| matches!(reason, BlockReason::TrustTooLow { .. })),
        "{decision:?}"
    );
    // And nothing the content does later can raise trust back: joins
    // only take the minimum.
    trajectory
        .record_message_received(
            "external:peer",
            JoinSpec::Delta(LabelDelta {
                audience: vec!["public".to_string()],
                trust: Some("trusted".to_string()),
            }),
        )
        .expect("receive");
    let label = trajectory.label().expect("label");
    assert_eq!(
        label.trust,
        trajectory.policy().trust_rank("suspicious").expect("rank")
    );
}

// ----------------------------------------------------------------
// Adversarial (iii): label stripping through a sub-agent.

#[test]
fn subagent_inherits_the_narrowed_label_and_cannot_strip_it() {
    let p = policy();
    let mut parent = Trajectory::new(p.clone());
    parent.record_read("file:secret").expect("read");
    let inherited = parent.snapshot().expect("snapshot");

    // The child starts exactly where the parent's label says.
    let mut child = Trajectory::child(p.clone(), "parent", inherited);
    let decision = child.propose(&email("ext@evil.example")).expect("propose");
    assert!(
        matches!(
            block_reasons(&decision).as_slice(),
            [BlockReason::AudienceMissing { .. }]
        ),
        "child must inherit the restriction: {decision:?}"
    );

    // A second, uninvolved agent that accepts the child's result
    // joins the child's end label — the restriction propagates
    // through results, not just through spawns.
    let child_end = child.snapshot().expect("child snapshot");
    let mut receiver = Trajectory::new(p);
    receiver
        .record(Event::SubagentResult {
            child: "child".to_string(),
            label: child_end,
        })
        .expect("record result");
    let decision = receiver
        .propose(&email("ext@evil.example"))
        .expect("propose");
    assert!(matches!(
        block_reasons(&decision).as_slice(),
        [BlockReason::AudienceMissing { .. }]
    ));
}

// ----------------------------------------------------------------
// Adversarial (iv): downgrade attempts.

#[test]
fn downgrade_cannot_widen_after_narrowing() {
    let p = policy();
    let mut trajectory = Trajectory::new(p);
    trajectory.record_read("file:secret").expect("read");
    // Reads of public material afterwards do not widen anything:
    // joins intersect, they never union.
    trajectory
        .record_message_received(
            "agent:peer",
            JoinSpec::Delta(LabelDelta::audience(&["public"])),
        )
        .expect("receive");
    let action = ProposedAction::Message {
        destination: "external:world".to_string(),
        payload_label: None,
    };
    let decision = trajectory.propose(&action).expect("propose");
    assert!(
        matches!(
            block_reasons(&decision).as_slice(),
            [BlockReason::AudienceMissing { readers }] if readers == &vec!["public".to_string()]
        ),
        "{decision:?}"
    );
}

#[test]
fn unknown_tool_and_unknown_destination_fail_closed() {
    let mut trajectory = Trajectory::new(policy());
    let tool = ProposedAction::ToolCall {
        tool: "shell_exec".to_string(),
        args: json!({ "cmd": "id" }),
    };
    let decision = trajectory.propose(&tool).expect("propose");
    assert!(matches!(
        block_reasons(&decision).as_slice(),
        [BlockReason::NoContract { .. }]
    ));
    let message = ProposedAction::Message {
        destination: "external:nowhere".to_string(),
        payload_label: None,
    };
    let decision = trajectory.propose(&message).expect("propose");
    assert!(matches!(
        block_reasons(&decision).as_slice(),
        [BlockReason::NoDestination { .. }]
    ));
}

#[test]
fn unresolvable_recipient_placeholder_fails_closed() {
    let mut trajectory = Trajectory::new(policy());
    let action = ProposedAction::ToolCall {
        tool: "send_email".to_string(),
        args: json!({ "body": "no recipient at all" }),
    };
    let decision = trajectory.propose(&action).expect("propose");
    assert!(matches!(
        block_reasons(&decision).as_slice(),
        [BlockReason::AudienceUnresolvable { .. }]
    ));
}

// ----------------------------------------------------------------
// Remedies.

#[test]
fn authority_approval_is_single_use_and_never_lifts_the_label() {
    let p = policy();
    let mut trajectory = Trajectory::new(p);
    trajectory.record_read("file:secret").expect("read");
    let before = trajectory.label().expect("label");

    let action = email("cfo@example.com");
    let decision = trajectory.propose(&action).expect("propose");
    let digest = decision.digest().to_string();
    assert!(matches!(
        block_reasons(&decision).as_slice(),
        [BlockReason::AudienceMissing { .. }]
    ));
    // The plan offers the operator authority.
    let Decision::Block(info) = &decision else {
        panic!("expected block");
    };
    assert!(
        info.remedies
            .options
            .iter()
            .any(|option| matches!(option, RemedyOption::RequestApproval { authority, .. } if authority == "operator")),
        "remedies: {:?}",
        info.remedies
    );

    // Approve exactly this action; the identical retry passes…
    trajectory
        .record(Event::ApprovalGranted {
            authority: "operator".to_string(),
            digest: digest.clone(),
            marks: Vec::new(),
        })
        .expect("approve");
    let decision = trajectory.propose(&action).expect("propose");
    let Decision::Allow(allow) = &decision else {
        panic!("approved action must pass: {decision:?}");
    };
    assert_eq!(allow.via_approval.as_deref(), Some("operator"));
    // …the label did not move…
    assert_eq!(trajectory.label().expect("label"), before);
    // …and once the result lands, the approval is spent: the same
    // call blocks again.
    trajectory
        .record_tool_result("send_email", &digest)
        .expect("result");
    let decision = trajectory.propose(&action).expect("propose");
    assert!(matches!(
        block_reasons(&decision).as_slice(),
        [BlockReason::AudienceMissing { .. }]
    ));
}

#[test]
fn approval_cannot_clear_effect_requirements() {
    let p = policy();
    let mut trajectory = Trajectory::new(p);
    let deploy = ProposedAction::ToolCall {
        tool: "deploy".to_string(),
        args: json!({}),
    };
    let decision = trajectory.propose(&deploy).expect("propose");
    let digest = decision.digest().to_string();
    trajectory
        .record(Event::ApprovalGranted {
            authority: "operator".to_string(),
            digest,
            marks: Vec::new(),
        })
        .expect("approve");
    let decision = trajectory.propose(&deploy).expect("propose");
    assert!(
        matches!(
            block_reasons(&decision).as_slice(),
            [BlockReason::EffectsMissing { .. }]
        ),
        "effects are facts, not permissions: {decision:?}"
    );
}

#[test]
fn attention_mark_needs_a_matching_approval() {
    let p = policy();
    let mut trajectory = Trajectory::new(p);
    let release = ProposedAction::ToolCall {
        tool: "release".to_string(),
        args: json!({}),
    };
    let decision = trajectory.propose(&release).expect("propose");
    let digest = decision.digest().to_string();
    assert!(matches!(
        block_reasons(&decision).as_slice(),
        [BlockReason::AttentionRequired { marks }] if marks == &vec!["human-review".to_string()]
    ));
    // An approval that does not list the mark does not clear it.
    trajectory
        .record(Event::ApprovalGranted {
            authority: "operator".to_string(),
            digest: digest.clone(),
            marks: Vec::new(),
        })
        .expect("approve without mark");
    let decision = trajectory.propose(&release).expect("propose");
    assert!(!decision.is_allow(), "{decision:?}");
    // One that lists it does — once.
    trajectory
        .record(Event::ApprovalGranted {
            authority: "operator".to_string(),
            digest,
            marks: vec!["human-review".to_string()],
        })
        .expect("approve with mark");
    let decision = trajectory.propose(&release).expect("propose");
    assert!(decision.is_allow(), "{decision:?}");
}

#[test]
fn blocked_mark_offers_no_remedy_and_no_approval_helps() {
    let p = policy();
    let mut trajectory = Trajectory::new(p);
    let wipe = ProposedAction::ToolCall {
        tool: "wipe_disk".to_string(),
        args: json!({}),
    };
    let decision = trajectory.propose(&wipe).expect("propose");
    let digest = decision.digest().to_string();
    let Decision::Block(info) = &decision else {
        panic!("expected block");
    };
    assert!(
        info.remedies.options.is_empty(),
        "remedies: {:?}",
        info.remedies
    );
    trajectory
        .record(Event::ApprovalGranted {
            authority: "operator".to_string(),
            digest,
            marks: vec!["blocked".to_string()],
        })
        .expect("approve");
    let decision = trajectory.propose(&wipe).expect("propose");
    assert!(!decision.is_allow(), "blocked is blocked: {decision:?}");
}

#[test]
fn sanitize_remedy_relabels_the_payload_and_the_retry_passes() {
    let p = policy();
    let mut trajectory = Trajectory::new(p);
    // Untrusted feed content dropped the trajectory's trust.
    trajectory
        .record_message_received(
            "external:peer",
            JoinSpec::Delta(LabelDelta::trust("suspicious")),
        )
        .expect("receive");
    let publish = ProposedAction::ToolCall {
        tool: "publish_digest".to_string(),
        args: json!({ "parts": [{ "text": "summary" }, { "file": "raw.bin" }] }),
    };
    let decision = trajectory.propose(&publish).expect("propose");
    let Decision::Block(info) = &decision else {
        panic!("expected block: {decision:?}");
    };
    assert!(
        info.reasons
            .iter()
            .any(|reason| matches!(reason, BlockReason::TrustTooLow { .. })),
        "{:?}",
        info.reasons
    );
    assert!(
        info.remedies.options.iter().any(|option| matches!(
            option,
            RemedyOption::Sanitize { name, would_allow } if name == "scrub" && *would_allow
        )),
        "remedies: {:?}",
        info.remedies
    );
    // Execute the remedy: sanitize, record it, retry. The payload is
    // reduced to its text part and the retry passes under the
    // sanitizer's declared result label…
    let sanitized = phlow_flowguard::apply_sanitizer(
        trajectory.policy(),
        "scrub",
        &json!({ "parts": [{ "text": "summary" }, { "file": "raw.bin" }] }),
    )
    .expect("sanitize");
    assert_eq!(sanitized["parts"].as_array().map(Vec::len), Some(1));
    trajectory
        .record(Event::Sanitized {
            sanitizer: "scrub".to_string(),
            result_delta: LabelDelta {
                audience: vec!["public".to_string()],
                trust: Some("trusted".to_string()),
            },
        })
        .expect("record sanitized");
    let retry = ProposedAction::ToolCall {
        tool: "publish_digest".to_string(),
        args: sanitized,
    };
    let decision = trajectory.propose(&retry).expect("propose");
    assert!(decision.is_allow(), "{decision:?}");
    // …while the trajectory's own label keeps the lowered trust.
    let label = trajectory.label().expect("label");
    assert_eq!(
        label.trust,
        trajectory.policy().trust_rank("suspicious").expect("rank")
    );
}

// ----------------------------------------------------------------
// Determinism.

#[test]
fn same_log_same_decision_and_fold_is_order_stable() {
    let p = policy();
    let first = vec![
        Event::SessionStarted {
            parent: None,
            start: None,
        },
        Event::DataRead {
            source: "file:secret".to_string(),
            delta: LabelDelta::audience(&["self"]),
        },
        Event::MessageReceived {
            from: "external:peer".to_string(),
            join: JoinSpec::Delta(LabelDelta::trust("suspicious")),
        },
    ];
    let second = vec![first[0].clone(), first[2].clone(), first[1].clone()];
    let action = email("cfo@example.com");
    let one = phlow_flowguard::decide(&p, &first, &action);
    let two = phlow_flowguard::decide(&p, &first, &action);
    let reordered = phlow_flowguard::decide(&p, &second, &action);
    assert_eq!(one, two, "same log must decide identically");
    assert_eq!(one, reordered, "independent joins commute");
}
