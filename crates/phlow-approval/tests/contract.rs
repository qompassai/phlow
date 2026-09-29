//! Boundary and limit contracts not covered by the gauntlet task-2xx cases.
//! First half: validation at the edges. Second half: adversarial inputs.

use std::time::SystemTime;

use phlow_approval::{
    ApprovalQueue, DEFAULT_TTL, EVENTS_MAX, Error, EventKind, EventPayload, EventSink,
    HumanVerdict, PERMISSIONS_MAX, Policy, QUEUE_RECORDS_MAX, RULES_MAX, Request, State, Verdict,
    decide,
};
use serde_json::{Value, json};

fn request(extra: Value) -> Request {
    let mut value = json!({ "tool": "fs.write", "risk": "local_reversible", "paths": ["/work/a"] });
    for (key, item) in extra.as_object().expect("fixture object") {
        value[key] = item.clone();
    }
    Request::from_json(&value).expect("fixture request")
}

fn rule(decision: &str) -> Value {
    json!({ "risk": "local_reversible", "decision": decision, "tools": ["fs.write"], "paths": ["/work/a"] })
}

fn verdict(policy: &Value, request: &Request) -> Verdict {
    decide(Policy::from_json(policy).ok().as_ref(), request.scope()).verdict
}

// ---- validation ----

#[test]
fn rules_at_limit_accepted() {
    let rules: Vec<Value> = (0..RULES_MAX).map(|_| rule("allow")).collect();
    assert!(Policy::from_json(&json!({ "version": 1, "rules": rules })).is_ok());
}

#[test]
fn permissions_at_limit_accepted() {
    let after: Vec<String> = (0..PERMISSIONS_MAX).map(|i| format!("p{i}")).collect();
    let mut queue = ApprovalQueue::new(&["operator"]).unwrap();
    let id = queue.request(
        "run",
        request(json!({ "permissions_after": after })),
        DEFAULT_TTL,
    );
    assert_eq!(
        queue
            .get(&id.unwrap())
            .unwrap()
            .permission_delta
            .added
            .len(),
        PERMISSIONS_MAX
    );
}

#[test]
fn resourceless_deny_rule_covers_whole_tool() {
    let deny = json!({ "risk": "local_reversible", "decision": "deny", "tools": ["fs.write"] });
    let policy = json!({ "version": 1, "default": "allow", "rules": [deny] });
    assert_eq!(
        verdict(&policy, &request(json!({ "paths": ["/elsewhere"] }))),
        Verdict::Deny
    );
}

#[test]
fn approval_outranks_allow() {
    let policy = json!({ "version": 1, "rules": [rule("allow"), rule("approval")] });
    assert_eq!(verdict(&policy, &request(json!({}))), Verdict::Approval);
}

#[test]
fn root_path_is_canonical() {
    let parsed =
        Request::from_json(&json!({ "tool": "fs.read", "risk": "observe", "paths": ["/"] }));
    assert!(parsed.is_ok());
}

#[test]
fn events_filter_by_run() {
    let mut sink = EventSink::new();
    let payload = EventPayload {
        tool: "check".to_owned(),
        ..EventPayload::default()
    };
    sink.append("one", EventKind::ToolStarted, payload.clone())
        .unwrap();
    sink.append("two", EventKind::ToolStarted, payload).unwrap();
    assert_eq!(sink.events(Some("two")).len(), 1);
    assert_eq!(sink.events(None).len(), 2);
}

#[test]
fn deadline_expiry_excludes_from_pending_surface() {
    let mut queue = ApprovalQueue::new(&["operator"]).unwrap();
    let id = queue
        .request("run", request(json!({})), DEFAULT_TTL)
        .unwrap();
    let deadline = queue.get(&id).unwrap().deadline;
    assert_eq!(queue.pending().len(), 1);
    assert_eq!(queue.sweep_expired(deadline), 1);
    assert!(queue.pending().is_empty());
}

// ---- adversarial ----

#[test]
fn over_limit_lists_rejected() {
    let rules: Vec<Value> = (0..=RULES_MAX).map(|_| rule("allow")).collect();
    let policy = Policy::from_json(&json!({ "version": 1, "rules": rules }));
    assert!(matches!(policy, Err(Error::TooMany { field: "rules", .. })));
    let after: Vec<String> = (0..=PERMISSIONS_MAX).map(|i| format!("p{i}")).collect();
    let value = json!({ "tool": "t", "risk": "observe", "permissions_after": after });
    assert!(matches!(
        Request::from_json(&value),
        Err(Error::TooMany { .. })
    ));
}

#[test]
fn non_canonical_paths_rejected() {
    for path in [
        "/work/./a",
        "/work/../a",
        "work/a",
        "/work//a",
        "/work/a/",
        "",
    ] {
        let value = json!({ "tool": "fs.write", "risk": "local_reversible", "paths": [path] });
        assert!(Request::from_json(&value).is_err(), "{path:?} admitted");
        let policy = json!({ "version": 1, "rules": [{
            "risk": "local_reversible", "decision": "deny", "tools": ["fs.write"], "paths": [path]
        }] });
        assert!(
            Policy::from_json(&policy).is_err(),
            "{path:?} in rule admitted"
        );
    }
}

#[test]
fn control_characters_rejected() {
    let tool = json!({ "tool": "fs.write\u{1b}[2J", "risk": "observe" });
    assert!(Request::from_json(&tool).is_err());
    let summary = json!({ "tool": "t", "risk": "observe", "summary": "ok\u{1b}]0;pwned\u{7}" });
    assert!(Request::from_json(&summary).is_err());
}

#[test]
fn wrong_version_shapes_rejected() {
    for version in [json!("1"), json!(0), json!(-1), json!(1.0), json!(null)] {
        assert!(
            Policy::from_json(&json!({ "version": version })).is_err(),
            "{version}"
        );
    }
    let bogus = json!({ "version": 1, "rules": [{ "risk": "bogus", "decision": "allow", "tools": ["t"] }] });
    assert!(Policy::from_json(&bogus).is_err());
}

#[test]
fn reserved_and_empty_operator_lists_rejected() {
    for operators in [
        &[][..],
        &["Agent"][..],
        &["operator", "MODEL"][..],
        &[""][..],
    ] {
        assert!(ApprovalQueue::new(operators).is_err(), "{operators:?}");
    }
}

#[test]
fn revoke_requires_operator_and_approval() {
    let mut queue = ApprovalQueue::new(&["operator"]).unwrap();
    let id = queue
        .request("run", request(json!({})), DEFAULT_TTL)
        .unwrap();
    assert_eq!(
        queue.revoke(&id, Some("operator")),
        Err(Error::WrongState {
            state: State::Pending
        })
    );
    queue
        .decide(&id, HumanVerdict::Approve, Some("operator"))
        .unwrap();
    assert_eq!(queue.revoke(&id, None), Err(Error::ActorRefused));
    assert_eq!(queue.revoke(&id, Some("agent")), Err(Error::ActorRefused));
    assert_eq!(queue.get(&id).unwrap().state, State::Approved);
}

#[test]
fn full_queue_and_log_reject_without_eviction() {
    let mut queue = ApprovalQueue::new(&["operator"]).unwrap();
    let first = queue
        .request("run", request(json!({})), DEFAULT_TTL)
        .unwrap();
    for _ in 1..QUEUE_RECORDS_MAX {
        queue
            .request("run", request(json!({})), DEFAULT_TTL)
            .unwrap();
    }
    assert!(
        queue
            .request("run", request(json!({})), DEFAULT_TTL)
            .is_err()
    );
    assert!(queue.get(&first).is_some());
    let mut sink = EventSink::new();
    let payload = EventPayload {
        tool: "check".to_owned(),
        ..EventPayload::default()
    };
    for _ in 0..EVENTS_MAX {
        sink.append("run", EventKind::ToolStarted, payload.clone())
            .unwrap();
    }
    assert!(sink.append("run", EventKind::ToolStarted, payload).is_err());
    assert_eq!(sink.events(None)[0].seq, 1);
}

#[test]
fn null_hole_in_set_rejected() {
    let value = json!({ "tool": "t", "risk": "observe", "permissions_after": [null, "root"] });
    assert!(Request::from_json(&value).is_err());
}

#[test]
fn future_event_time_is_not_silently_clamped() {
    let payload = EventPayload {
        tool: "check".to_owned(),
        ..EventPayload::default()
    };
    let far = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(u64::MAX / 2);
    assert!(phlow_approval::make_envelope("run", EventKind::ToolStarted, payload, far).is_err());
}
