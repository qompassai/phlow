//! Integration tests for task-131 (scheduled scope refresh).
//!
//! Four driver cases — 2 validation, 2 adversarial — against the
//! clearly labeled scripted doubles (`ScriptedFeed`, `ManualClock`):
//! three polls (v1, v1, v2) file versions [1,1,2] with nothing new
//! on the unchanged poll; `fetched_at` is monotonic; a malformed poll
//! is typed `FeedError::Malformed` and retains the old scope; a
//! +3600s clock jump fires exactly one catch-up poll.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_131;

fn check_case(case: &str) -> CaseReport {
    let report = task_131::run_case(case)
        .unwrap_or_else(|e| panic!("task-131 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-131 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: three polls → latest versions exactly [1,1,2]; the unchanged
/// poll files nothing new.
#[test]
fn versioned_polls() {
    assert_eq!(task_131::ID, "task-131");
    assert_eq!(task_131::POLL_INTERVAL_SECS, 60);
    let report = check_case("versioned_polls");
    let m = &report.metrics;
    let versions: Vec<u64> = m["latest_versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    assert_eq!(versions, vec![1, 1, 2], "latest-version sequence");
    let counts: Vec<u64> = m["version_counts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    assert_eq!(
        counts,
        vec![1, 1, 2],
        "version counts (idempotent middle poll)"
    );
}

/// V2: `fetched_at` recorded per filed version, monotonic.
#[test]
fn fetched_at_monotonic() {
    let report = check_case("fetched_at_monotonic");
    let m = &report.metrics;
    assert_eq!(m["monotonic"].as_bool(), Some(true));
    let filed = m["filed"].as_array().unwrap();
    assert_eq!(filed.len(), 2, "exactly two new versions filed");
    let at: Vec<u64> = filed
        .iter()
        .map(|f| f["fetched_at"].as_u64().unwrap())
        .collect();
    assert!(at[0] <= at[1], "fetched_at must be monotonic: {at:?}");
}

// --- adversarial ---

/// A1: malformed poll → typed `FeedError::Malformed`, old scope
/// retained (`latest().version == 1`).
#[test]
fn malformed_poll_retains_scope() {
    let report = check_case("malformed_poll_retains_scope");
    let m = &report.metrics;
    assert_eq!(m["latest_version"].as_u64(), Some(1));
    assert_eq!(m["version_count"].as_u64(), Some(1));
    assert_eq!(m["typed_malformed"].as_bool(), Some(true));
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("Malformed"),
        "evidence must show the typed error:\n{joined}"
    );
}

/// A2: +3600s clock jump → exactly one catch-up poll, versions
/// gapless, timer recovers to the normal cadence.
#[test]
fn clock_jump_single_catchup() {
    let report = check_case("clock_jump_single_catchup");
    let m = &report.metrics;
    assert_eq!(
        m["polls_in_jump_window"].as_u64(),
        Some(1),
        "no poll storm on clock jump"
    );
    assert_eq!(m["version_count"].as_u64(), Some(2), "gapless versions");
    assert_eq!(m["timer_recovered"].as_bool(), Some(true));
}
