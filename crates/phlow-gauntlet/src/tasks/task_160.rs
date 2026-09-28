// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 160 — subscribe-on-reconnect (rust, V).
//!
//! Subscriptions are client state, not connection state: after any
//! reconnect the client re-subscribes to exactly its prior set — no
//! drops, no duplicates. The driver holds a [`SubscriptionSet`](crate::daemon_client::SubscriptionSet)
//! and a [`ScriptedLink`](crate::daemon_client::ScriptedLink) double
//! (MOCK); the daemon side is the link's outbound frame ledger, parsed
//! for `sub:<topic>` frames. A drop is modeled as `link.close()` between
//! two connects, mirroring the worker's release-the-socket path.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::daemon_client::{Link, ScriptedLink, ScriptedRead, SubscriptionSet};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-160";
/// Task name.
pub const NAME: &str = "subscribe-on-reconnect";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation.
pub const CASES: [&str; 2] = ["resubscribe_exactly_once", "subscribe_during_outage"];
/// Bounded write for the scripted link.
const WRITE_TIMEOUT: Duration = Duration::from_millis(10);

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// One (re)connect: open the link and subscribe the whole client-held
/// set, in the set's deterministic order.
fn connect_and_subscribe(
    link: &mut ScriptedLink,
    subs: &SubscriptionSet,
) -> Result<(), TaskDriverError> {
    link.connect()
        .map_err(|e| arm_error("connect", format!("scripted connect failed: {e:?}")))?;
    for topic in subs.topics() {
        link.write_frame(&format!("sub:{topic}"), WRITE_TIMEOUT)
            .map_err(|e| arm_error("subscribe", format!("scripted subscribe failed: {e:?}")))?;
    }
    Ok(())
}

/// Daemon-side view: per-topic subscribe counts parsed from the
/// outbound `sub:<topic>` frames.
fn subscribe_counts(link: &ScriptedLink) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for frame in link.outbound() {
        if let Some(topic) = frame.strip_prefix("sub:") {
            *counts.entry(topic.to_string()).or_insert(0) += 1;
        }
    }
    counts
}

/// V1: subscribed to {a, b}; the connection drops and reconnects. The
/// daemon must observe subscribe(a), subscribe(b) exactly once per
/// connection — the post-reconnect set equals the pre-drop set.
fn case_resubscribe_exactly_once() -> Result<CaseReport, TaskDriverError> {
    let mut subs = SubscriptionSet::new();
    subs.subscribe("a");
    subs.subscribe("b");
    let mut link = ScriptedLink::new(true, ScriptedRead::Timeout, true);
    let mut failures = Vec::new();

    connect_and_subscribe(&mut link, &subs)?;
    let first_batch = link.outbound().to_vec();
    link.close(); // the drop
    connect_and_subscribe(&mut link, &subs)?;
    let second_batch: Vec<String> = link.outbound()[first_batch.len()..].to_vec();
    link.close();

    for (n, batch) in [&first_batch, &second_batch].iter().enumerate() {
        if batch.as_slice() != ["sub:a".to_string(), "sub:b".to_string()] {
            failures.push(format!(
                "connection {} subscribes {batch:?}, want exactly [sub:a, sub:b] once each",
                n + 1
            ));
        }
    }
    let counts = subscribe_counts(&link);
    for topic in ["a", "b"] {
        if counts.get(topic) != Some(&2) {
            failures.push(format!(
                "daemon-side subscribes for {topic}: {:?}, want exactly 2 (one per connection)",
                counts.get(topic)
            ));
        }
    }
    if link.open_handles() != 0 {
        failures.push(format!(
            "link handles open: {}, want 0",
            link.open_handles()
        ));
    }
    if link.connect_count() != 2 || link.close_count() != 2 {
        failures.push(format!(
            "connects={} closes={}, want 2/2 (exactly-once release)",
            link.connect_count(),
            link.close_count()
        ));
    }

    let mut evidence = vec![
        format!("connection 1 subscribes: {first_batch:?}"),
        format!("connection 2 subscribes: {second_batch:?}"),
        format!("daemon-side counts: {counts:?}"),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "subscribe_counts": counts,
            "connects": link.connect_count(),
            "closes": link.close_count(),
            "open_handles": link.open_handles(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.failures = failures;
    report.passed = report.failures.is_empty();
    Ok(report)
}

/// V2: subscribe(c) *during* the outage — no link involved, pure
/// client state — then reconnect. The daemon must see exactly {a, b, c},
/// once each, with no duplicates.
fn case_subscribe_during_outage() -> Result<CaseReport, TaskDriverError> {
    let mut subs = SubscriptionSet::new();
    subs.subscribe("a");
    subs.subscribe("b");
    let mut link = ScriptedLink::new(true, ScriptedRead::Timeout, true);
    let mut failures = Vec::new();

    connect_and_subscribe(&mut link, &subs)?;
    link.close(); // the drop; the outage begins
    // subscribe(c) during the outage: client state only, no link.
    if !subs.subscribe("c") {
        failures.push("subscribe(c) during outage was not accepted".to_string());
    }
    if link.outbound().len() != 2 {
        failures.push("frames were written while the link was down".to_string());
    }
    connect_and_subscribe(&mut link, &subs)?;
    link.close();

    let second_batch: Vec<String> = link.outbound()[2..].to_vec();
    if second_batch
        != [
            "sub:a".to_string(),
            "sub:b".to_string(),
            "sub:c".to_string(),
        ]
    {
        failures.push(format!(
            "post-reconnect subscribes {second_batch:?}, want exactly [sub:a, sub:b, sub:c]"
        ));
    }
    let counts = subscribe_counts(&link);
    let want = [("a", 2), ("b", 2), ("c", 1)];
    for (topic, n) in want {
        if counts.get(topic) != Some(&n) {
            failures.push(format!(
                "daemon-side subscribes for {topic}: {:?}, want {n}",
                counts.get(topic)
            ));
        }
    }
    if counts.len() != 3 {
        failures.push(format!(
            "daemon saw topics {counts:?}, want exactly {{a, b, c}}"
        ));
    }

    let mut evidence = vec![
        "subscribed c during outage (link down, 0 frames written)".to_string(),
        format!("post-reconnect subscribes: {second_batch:?}"),
        format!("daemon-side counts: {counts:?}"),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "subscribe_counts": counts,
            "post_reconnect_batch": second_batch,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.failures = failures;
    report.passed = report.failures.is_empty();
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "resubscribe_exactly_once" => case_resubscribe_exactly_once(),
        "subscribe_during_outage" => case_subscribe_during_outage(),
        _ => Err(arm_error(
            "case",
            format!("task-160: unknown case '{case}'"),
        )),
    }
}

/// Task-level entry for the gauntlet runner: the headline case.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-160".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-160".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
