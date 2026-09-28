//! Task 131 — scheduled scope refresh (rust, V).
//!
//! The cycle starts from fresh scope: a timer polls the scope feed
//! (bbscope-style) on a fixed interval and files each new snapshot as
//! a new version in the [`ScopeStore`]. Identical polls are
//! idempotent — they file nothing new. A malformed poll is typed,
//! logged, and leaves the old scope in place. A clock jump fires
//! exactly one catch-up poll (never a poll storm) and reschedules
//! from *now*.
//!
//! The driver uses the scripted doubles: [`ScriptedFeed`] (MOCK)
//! serving v1, v1, v2, and [`ManualClock`] (MOCK) driving the poll
//! timer at T=0,60,120s.
//!
//! Primary source for the "bbscope-style" claim: sw33tlie/bbscope
//! polls platform scopes on a `--poll-interval` default of **6
//! hours** (0 disables background polling) and tracks scope changes
//! across polls (website/README.md,
//! docs/src/web/self-hosting.md). Our driver accelerates the cadence
//! to 60s scripted ticks; the feed stays "bbscope-style", not
//! "bbscope-compatible" — no wire compatibility is claimed.

use crate::bounty::clock::{Clock, ManualClock};
use crate::bounty::feed::{FeedError, ScopeFeed, ScriptedFeed, snapshot, target};
use crate::bounty::store::ScopeStore;
use crate::bounty::types::{ScopeSnapshot, TargetKind};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-131";
/// Task name.
pub const NAME: &str = "scheduled scope refresh";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "versioned_polls",
    "fetched_at_monotonic",
    "malformed_poll_retains_scope",
    "clock_jump_single_catchup",
];
/// Scripted poll cadence. This is a test constant — bbscope's real
/// default is 6 hours (see module docs); the driver accelerates it so
/// the gauntlet stays fast.
pub const POLL_INTERVAL_SECS: u64 = 60;
/// Scripted epoch for the [`ManualClock`].
pub const CLOCK_START: u64 = 1_000_000;

fn snapshot_v1(fetched_at: u64) -> ScopeSnapshot {
    snapshot(
        1,
        fetched_at,
        vec![
            target("a", TargetKind::Domain, "a.example.com"),
            target("b", TargetKind::Domain, "b.example.com"),
            target("c", TargetKind::Domain, "c.example.com"),
        ],
    )
}

fn snapshot_v2(fetched_at: u64) -> ScopeSnapshot {
    let mut targets = snapshot_v1(fetched_at).targets;
    targets.push(target("d", TargetKind::Domain, "d.example.com"));
    snapshot(2, fetched_at, targets)
}

/// Poller state: when the next poll is due (clock seconds).
struct PollState {
    next_due: u64,
}

/// Outcome of one poller tick: whether a poll fired, and — when a new
/// version was filed — that version's (version, fetched_at).
struct TickOutcome {
    fired: bool,
    filed: Option<(u64, u64)>,
    feed_error: Option<FeedError>,
}

/// One poller tick. When a poll is due: poll the feed once, file the
/// snapshot (an idempotent re-file files nothing new), and reschedule
/// the next poll from *now* — not from the old due time — so a clock
/// jump produces exactly one catch-up poll instead of a storm. A
/// malformed poll is typed and logged; the old scope stays filed.
fn poller_tick(
    clock: &ManualClock,
    feed: &mut ScriptedFeed,
    store: &mut ScopeStore,
    state: &mut PollState,
    audit: &mut Vec<String>,
) -> Result<TickOutcome, TaskDriverError> {
    let now = clock.now();
    if now < state.next_due {
        return Ok(TickOutcome {
            fired: false,
            filed: None,
            feed_error: None,
        });
    }
    // Reschedule from now BEFORE polling: a failing poll must not
    // wedge the timer. The timer is fail-open; the data stays
    // fail-closed (old scope retained).
    state.next_due = now.saturating_add(POLL_INTERVAL_SECS);
    match feed.poll() {
        Ok(snap) => {
            let version = snap.version;
            let fetched_at = snap.fetched_at;
            let is_new = store.file(snap).map_err(|e| TaskDriverError::Arm {
                arm: "scope-store-file".to_string(),
                detail: format!("version {version} refused: {e:?}"),
            })?;
            audit.push(format!("poll ok: v{version} new={is_new}"));
            Ok(TickOutcome {
                fired: true,
                filed: is_new.then_some((version, fetched_at)),
                feed_error: None,
            })
        }
        Err(e) => {
            audit.push(format!("poll failed: {e:?}; old scope retained"));
            Ok(TickOutcome {
                fired: true,
                filed: None,
                feed_error: Some(e),
            })
        }
    }
}

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// V1: three scripted polls (v1, v1, v2) at T=0,60,120s. The
/// latest-version sequence must be exactly [1,1,2] and the version
/// count [1,1,2] — the unchanged second poll files nothing new.
fn case_versioned_polls() -> Result<CaseReport, TaskDriverError> {
    let mut clock = ManualClock::new(CLOCK_START);
    let mut feed = ScriptedFeed::new(vec![
        snapshot_v1(CLOCK_START),
        snapshot_v1(CLOCK_START + POLL_INTERVAL_SECS),
        snapshot_v2(CLOCK_START + 2 * POLL_INTERVAL_SECS),
    ]);
    let mut store = ScopeStore::new();
    let mut state = PollState {
        next_due: CLOCK_START,
    };
    let mut audit = Vec::new();
    let mut failures = Vec::new();
    let mut versions = Vec::new();
    let mut counts = Vec::new();
    for tick in 0..3u64 {
        let out = poller_tick(&clock, &mut feed, &mut store, &mut state, &mut audit)?;
        if !out.fired {
            failures.push(format!("tick {tick}: poll did not fire when due"));
        }
        versions.push(store.latest().map(|s| s.version).unwrap_or(0));
        counts.push(store.version_count());
        clock.advance(POLL_INTERVAL_SECS);
    }
    if versions != [1, 1, 2] {
        failures.push(format!(
            "latest-version sequence {versions:?}, want [1, 1, 2]"
        ));
    }
    if counts != [1, 1, 2] {
        failures.push(format!("version counts {counts:?}, want [1, 1, 2]"));
    }
    let mut evidence = vec![format!(
        "3 scripted polls (v1, v1, v2) at 60s cadence: latest versions \
         {versions:?}, version counts {counts:?}"
    )];
    evidence.extend(audit);
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "latest_versions": versions,
            "version_counts": counts,
            "poll_interval_secs": POLL_INTERVAL_SECS,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: every newly filed version carries a `fetched_at`, and the
/// filed sequence is monotonic — a new version is never stamped
/// older than the version it supersedes.
fn case_fetched_at_monotonic() -> Result<CaseReport, TaskDriverError> {
    let mut clock = ManualClock::new(CLOCK_START);
    let mut feed = ScriptedFeed::new(vec![
        snapshot_v1(CLOCK_START),
        snapshot_v1(CLOCK_START + POLL_INTERVAL_SECS),
        snapshot_v2(CLOCK_START + 2 * POLL_INTERVAL_SECS),
    ]);
    let mut store = ScopeStore::new();
    let mut state = PollState {
        next_due: CLOCK_START,
    };
    let mut audit = Vec::new();
    let mut filed: Vec<(u64, u64)> = Vec::new();
    for _ in 0..3 {
        let out = poller_tick(&clock, &mut feed, &mut store, &mut state, &mut audit)?;
        if let Some(f) = out.filed {
            filed.push(f);
        }
        clock.advance(POLL_INTERVAL_SECS);
    }
    let mut failures = Vec::new();
    if filed.len() != 2 {
        failures.push(format!("filed {} new versions, want 2", filed.len()));
    }
    let monotonic = filed.windows(2).all(|w| w[0].1 <= w[1].1);
    if !monotonic {
        failures.push(format!("fetched_at not monotonic: {filed:?}"));
    }
    for (v, at) in &filed {
        // Each version was filed on its poll tick: v1 at T=0, v2 at
        // T=2*interval (the unchanged middle poll filed nothing).
        let want_at = CLOCK_START + (v - 1) * 2 * POLL_INTERVAL_SECS;
        if *at != want_at {
            failures.push(format!(
                "v{v} fetched_at={at}, want its poll time {want_at}"
            ));
        }
        audit.push(format!("filed v{v} fetched_at={at}"));
    }
    let mut evidence = vec![format!(
        "filed versions (version, fetched_at): {filed:?}; monotonic: {monotonic}"
    )];
    evidence.extend(audit);
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "filed": filed.iter().map(|(v, at)| serde_json::json!({"version": v, "fetched_at": at})).collect::<Vec<_>>(),
            "monotonic": monotonic,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1 (adversarial): the feed goes malformed on the second poll. The
/// error must be typed [`FeedError::Malformed`], logged, and the old
/// scope (v1) retained — `latest().version == 1`.
fn case_malformed_poll_retains_scope() -> Result<CaseReport, TaskDriverError> {
    let mut clock = ManualClock::new(CLOCK_START);
    let mut feed = ScriptedFeed::new(vec![snapshot_v1(CLOCK_START)]).with_malformed_at(1);
    let mut store = ScopeStore::new();
    let mut state = PollState {
        next_due: CLOCK_START,
    };
    let mut audit = Vec::new();
    let mut failures = Vec::new();
    // Tick 1: v1 files cleanly.
    poller_tick(&clock, &mut feed, &mut store, &mut state, &mut audit)?;
    clock.advance(POLL_INTERVAL_SECS);
    // Tick 2: malformed.
    let out = poller_tick(&clock, &mut feed, &mut store, &mut state, &mut audit)?;
    match &out.feed_error {
        Some(FeedError::Malformed(_)) => {
            audit.push("typed error confirmed: FeedError::Malformed".to_string());
        }
        other => {
            failures.push(format!("malformed poll gave {other:?}, want Malformed"));
        }
    }
    let latest_version = store.latest().map(|s| s.version);
    if latest_version != Some(1) {
        failures.push(format!(
            "latest version after malformed poll: {latest_version:?}, want Some(1)"
        ));
    }
    if store.version_count() != 1 {
        failures.push(format!(
            "version count after malformed poll: {}, want 1",
            store.version_count()
        ));
    }
    // Tick 3: the feed repeats its last good snapshot — idempotent,
    // still no new version.
    clock.advance(POLL_INTERVAL_SECS);
    poller_tick(&clock, &mut feed, &mut store, &mut state, &mut audit)?;
    if store.version_count() != 1 {
        failures.push("re-poll after malformed filed a new version".to_string());
    }
    let mut evidence = vec![format!(
        "malformed poll: typed Malformed, logged; latest version \
         {latest_version:?}, version count {}",
        store.version_count()
    )];
    evidence.extend(audit);
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "latest_version": latest_version,
            "version_count": store.version_count(),
            "typed_malformed": matches!(out.feed_error, Some(FeedError::Malformed(_))),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2 (adversarial): the clock jumps +3600s. Exactly one catch-up
/// poll must fire (no poll storm), and the filed versions stay
/// gapless: [1, 2].
fn case_clock_jump_single_catchup() -> Result<CaseReport, TaskDriverError> {
    let mut clock = ManualClock::new(CLOCK_START);
    let mut feed = ScriptedFeed::new(vec![
        snapshot_v1(CLOCK_START),
        snapshot_v2(CLOCK_START + 3600),
    ]);
    let mut store = ScopeStore::new();
    let mut state = PollState {
        next_due: CLOCK_START,
    };
    let mut audit = Vec::new();
    let mut failures = Vec::new();
    poller_tick(&clock, &mut feed, &mut store, &mut state, &mut audit)?;
    // The adversarial jump: +3600s in one step.
    clock.advance(3600);
    let polls_before = feed.polls_done();
    let out = poller_tick(&clock, &mut feed, &mut store, &mut state, &mut audit)?;
    let polls_in_jump = feed.polls_done() - polls_before;
    if polls_in_jump != 1 {
        failures.push(format!(
            "clock jump fired {polls_in_jump} polls, want exactly 1 (no storm)"
        ));
    }
    if out.filed != Some((2, CLOCK_START + 3600)) {
        failures.push(format!(
            "catch-up poll filed {:?}, want v2 (gapless)",
            out.filed
        ));
    }
    if store.version_count() != 2 {
        failures.push(format!(
            "version count after jump: {}, want 2",
            store.version_count()
        ));
    }
    // The timer recovered: the next tick fires on the normal cadence.
    clock.advance(POLL_INTERVAL_SECS);
    let out2 = poller_tick(&clock, &mut feed, &mut store, &mut state, &mut audit)?;
    if !out2.fired {
        failures.push("timer did not recover to normal cadence after jump".to_string());
    }
    let mut evidence = vec![format!(
        "+3600s clock jump: polls fired in jump window = {polls_in_jump} \
         (bar: <= 2, want exactly 1); filed versions gapless [1, 2]; \
         timer recovered to {POLL_INTERVAL_SECS}s cadence"
    )];
    evidence.extend(audit);
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "polls_in_jump_window": polls_in_jump,
            "version_count": store.version_count(),
            "timer_recovered": out2.fired,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "versioned_polls" => case_versioned_polls(),
        "fetched_at_monotonic" => case_fetched_at_monotonic(),
        "malformed_poll_retains_scope" => case_malformed_poll_retains_scope(),
        "clock_jump_single_catchup" => case_clock_jump_single_catchup(),
        _ => Err(arm_error(
            "case",
            format!("task-131: unknown case '{case}'"),
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
            where_: "task-131".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-131".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
