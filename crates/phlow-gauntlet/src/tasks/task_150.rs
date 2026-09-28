//! Task 150 — hostile scope-feed refusal (rust, A).
//!
//! The seam is scope-feed ingestion (`ScopeFeed::poll` → `ScopeStore`):
//! a scope feed that tries to authorize an out-of-bounds target is
//! refused structurally — the snapshot is rejected, the feed
//! quarantined, the previous scope retained. A driver-local ingestion
//! layer enforces the enrollment pins in order: authenticity (the
//! pinned feed key signs every snapshot — a toy keyed tag, labeled
//! MOCK, standing in for real signature verification), version
//! monotonicity (TUF-style rollback protection), then enrollment
//! bounds (allowed suffixes, no CIDR widening). Four scenarios: an
//! out-of-bounds target is refused (V1); a bad signature rejects the
//! snapshot before targets are parsed (V2); CIDR widening is refused
//! (A1); a replayed old version is rejected (A2). Every refusal is
//! typed; the scope store never changes under attack.

use crate::bounty::approve::sha256_hex;
use crate::bounty::feed::{ScopeFeed, snapshot, target};
use crate::bounty::store::ScopeStoreError;
use crate::bounty::{
    FeedError, Program, RateLimit, ScopeSnapshot, ScopeStore, ScriptedFeed, Target, TargetKind,
    TestingWindow,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use std::collections::HashMap;

/// Task id.
pub const ID: &str = "task-150";
/// Task name.
pub const NAME: &str = "hostile scope-feed refusal";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "out_of_bounds_target_refused",
    "bad_signature_rejected_before_parse",
    "cidr_widening_refused",
    "replayed_version_rejected",
];

/// Fixed scripted time (MOCK clock).
const NOW: u64 = 1_700_000_000;

/// What the enrollment pins: the program's scope bounds plus the feed
/// signing key.
struct Enrollment {
    program: Program,
    feed_key: String,
}

/// Typed refusal for scope ingestion.
#[derive(Debug, PartialEq, Eq)]
enum IngestError {
    /// The feed itself failed or self-reported hostile.
    Feed(FeedError),
    /// The snapshot's signature tag did not verify: rejected before
    /// any target was parsed.
    BadSignature { version: u64 },
    /// The snapshot replayed an old version (rollback attempt).
    StaleVersion { got: u64, latest: u64 },
}

/// Toy signature tag: keyed sha256 over the version and target values.
/// MOCK — stands in for pinned-key signature verification at toy
/// scale; it is NOT a real signature scheme.
fn sign_tag(key: &str, snap: &ScopeSnapshot) -> String {
    let mut material = key.as_bytes().to_vec();
    material.extend_from_slice(&snap.version.to_le_bytes());
    for t in &snap.targets {
        material.extend_from_slice(t.value.as_bytes());
    }
    sha256_hex(&material)
}

/// A scripted feed with per-version signature tags.
struct SignedFeed {
    inner: ScriptedFeed,
    tags: HashMap<u64, String>,
}

impl SignedFeed {
    fn new(inner: ScriptedFeed) -> Self {
        SignedFeed {
            inner,
            tags: HashMap::new(),
        }
    }

    /// Record the correct tag for a snapshot version.
    fn tag_version(&mut self, key: &str, snap: &ScopeSnapshot) {
        self.tags.insert(snap.version, sign_tag(key, snap));
    }

    /// Record a tag made with the wrong key (for the bad-signature arm).
    fn tag_with_wrong_key(&mut self, key: &str, snap: &ScopeSnapshot) {
        self.tags.insert(snap.version, sign_tag(key, snap));
    }

    fn tag_for(&self, version: u64) -> Option<&str> {
        self.tags.get(&version).map(|s| s.as_str())
    }
}

impl ScopeFeed for SignedFeed {
    fn poll(&mut self) -> Result<ScopeSnapshot, FeedError> {
        self.inner.poll()
    }
}

/// Host part of a domain or URL target (`*.` wildcards stripped).
fn host_of(value: &str) -> String {
    let mut h = value;
    if let Some((_, after)) = h.split_once("://") {
        h = after;
    }
    if let Some((before, _)) = h.split_once('/') {
        h = before;
    }
    h.strip_prefix("*.").unwrap_or(h).to_string()
}

/// Parse "a.b.c.d/p" into (prefix_len, network u32). None when malformed.
fn parse_cidr_v4(value: &str) -> Option<(u8, u32)> {
    let (addr, prefix) = value.split_once('/')?;
    let prefix: u8 = prefix.parse().ok()?;
    if prefix > 32 {
        return None;
    }
    if addr.split('.').count() != 4 {
        return None;
    }
    let mut octets = [0u8; 4];
    for (i, part) in addr.split('.').enumerate() {
        octets[i] = part.parse().ok()?;
    }
    Some((prefix, u32::from_be_bytes(octets)))
}

/// True when (net/prefix) strictly covers (old_net/old_prefix).
fn cidr_covers(net: u32, prefix: u8, old_net: u32, old_prefix: u8) -> bool {
    if prefix >= old_prefix {
        return false;
    }
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    (net & mask) == (old_net & mask)
}

/// Enrollment check for one target. `Err(reason)` when the target is
/// outside the enrolled scope or widens an enrolled target.
fn target_allowed(t: &Target, enrollment: &Enrollment, store: &ScopeStore) -> Result<(), String> {
    match t.kind {
        TargetKind::Domain | TargetKind::Url => {
            let host = host_of(&t.value);
            let ok = enrollment
                .program
                .allowed_suffixes
                .iter()
                .any(|s| host == *s || host.ends_with(&format!(".{s}")));
            if ok {
                Ok(())
            } else {
                Err(format!(
                    "outside enrolled suffixes {:?}",
                    enrollment.program.allowed_suffixes
                ))
            }
        }
        TargetKind::Cidr => {
            let (prefix, net) =
                parse_cidr_v4(&t.value).ok_or_else(|| "unparseable CIDR".to_string())?;
            // Never widen first: a superset of an enrolled target is
            // not the enrolled scope, whatever the prefix floor says.
            if let Some(latest) = store.latest() {
                for old in &latest.targets {
                    if old.kind != TargetKind::Cidr {
                        continue;
                    }
                    if let Some((old_prefix, old_net)) = parse_cidr_v4(&old.value)
                        && prefix < old_prefix
                        && cidr_covers(net, prefix, old_net, old_prefix)
                    {
                        return Err(format!("widens enrolled target {}", old.value));
                    }
                }
            }
            if prefix < enrollment.program.max_cidr_prefix {
                return Err(format!(
                    "CIDR /{prefix} wider than enrolled max /{}",
                    enrollment.program.max_cidr_prefix
                ));
            }
            Ok(())
        }
    }
}

/// Ingest one poll: authenticity → version monotonicity → enrollment
/// bounds → file. Any refusal quarantines the feed and leaves the
/// scope store untouched.
fn ingest(
    feed: &mut SignedFeed,
    store: &mut ScopeStore,
    enrollment: &Enrollment,
    quarantine: &mut Vec<String>,
) -> Result<u64, IngestError> {
    let snap = match feed.poll() {
        Ok(s) => s,
        Err(e) => {
            quarantine.push(format!(
                "poll refused ({e:?}); feed quarantined, scope retained"
            ));
            return Err(IngestError::Feed(e));
        }
    };
    // Authenticity first: the version number itself is only trustworthy
    // inside a valid signature.
    let want = sign_tag(&enrollment.feed_key, &snap);
    if feed.tag_for(snap.version) != Some(want.as_str()) {
        quarantine.push(format!(
            "v{}: bad signature; snapshot rejected before target parsing",
            snap.version
        ));
        return Err(IngestError::BadSignature {
            version: snap.version,
        });
    }
    // Monotonic versions only: a replayed old snapshot is a rollback.
    if let Some(latest) = store.latest()
        && snap.version <= latest.version
    {
        quarantine.push(format!(
            "v{}: replayed version (latest v{}); rejected",
            snap.version, latest.version
        ));
        return Err(IngestError::StaleVersion {
            got: snap.version,
            latest: latest.version,
        });
    }
    // Enrollment bounds: nothing outside the program's scope, and no
    // widening of what is already enrolled.
    for t in &snap.targets {
        if let Err(reason) = target_allowed(t, enrollment, store) {
            quarantine.push(format!(
                "v{}: hostile target {} ({reason}); snapshot rejected, feed quarantined",
                snap.version, t.value
            ));
            return Err(IngestError::Feed(FeedError::HostileTarget {
                value: t.value.clone(),
                reason,
            }));
        }
    }
    let version = snap.version;
    store
        .file(snap)
        .map_err(
            |ScopeStoreError::StaleVersion { got, latest }| IngestError::StaleVersion {
                got,
                latest,
            },
        )?;
    Ok(version)
}

/// The fixture enrollment: only example.com, CIDRs /24 or narrower,
/// one pinned feed key.
fn enrollment() -> Enrollment {
    Enrollment {
        program: Program {
            id: "prog-wave26".to_string(),
            name: "wave26 fixture program".to_string(),
            window: TestingWindow {
                start_min: 0,
                end_min: 1440,
            },
            rate_limit: RateLimit {
                requests_per_minute: 60,
                burst: 10,
            },
            allowed_suffixes: vec!["example.com".to_string()],
            max_cidr_prefix: 24,
        },
        feed_key: "feed-key-fixture-001".to_string(),
    }
}

/// Baseline scope already filed: v5 with one domain and one CIDR.
fn baseline_store() -> Result<ScopeStore, TaskDriverError> {
    let mut store = ScopeStore::new();
    let v5 = snapshot(
        5,
        NOW,
        vec![
            target("t-app", TargetKind::Domain, "app.example.com"),
            target("t-net", TargetKind::Cidr, "10.0.0.0/24"),
        ],
    );
    store.file(v5).map_err(|e| TaskDriverError::Fixture {
        what: "baseline".to_string(),
        detail: format!("task-150: baseline v5 failed to file: {e:?}"),
    })?;
    Ok(store)
}

fn scope_version(store: &ScopeStore) -> Option<u64> {
    store.latest().map(|s| s.version)
}

/// V1: the feed adds `*.evil.com` when the enrollment covers only
/// `example.com` → `FeedError::HostileTarget`, snapshot rejected, feed
/// quarantined, scope stays v5. Both refusal paths are exercised: a
/// poll where the feed self-reports hostile, and a snapshot carrying
/// the hostile target that ingestion refuses.
fn case_out_of_bounds_target_refused() -> Result<CaseReport, TaskDriverError> {
    let enrollment = enrollment();
    let mut store = baseline_store()?;
    let mut quarantine: Vec<String> = Vec::new();
    let evil = target("t-evil", TargetKind::Domain, "*.evil.com");
    let v6 = snapshot(
        6,
        NOW + 10,
        vec![
            target("t-app", TargetKind::Domain, "app.example.com"),
            evil.clone(),
        ],
    );
    let mut feed = SignedFeed::new(ScriptedFeed::new(vec![v6.clone()]).with_hostile_at(0, evil));
    feed.tag_version(&enrollment.feed_key, &v6);
    let mut failures = Vec::new();
    // Poll 0: the feed self-reports a hostile target.
    match ingest(&mut feed, &mut store, &enrollment, &mut quarantine) {
        Err(IngestError::Feed(FeedError::HostileTarget { value, .. })) if value == "*.evil.com" => {
        }
        other => failures.push(format!(
            "self-reported hostile poll: wrong outcome: {other:?}"
        )),
    }
    // Poll 1: the snapshot itself carries *.evil.com; ingestion refuses.
    match ingest(&mut feed, &mut store, &enrollment, &mut quarantine) {
        Err(IngestError::Feed(FeedError::HostileTarget { value, reason }))
            if value == "*.evil.com" =>
        {
            if !reason.contains("example.com") {
                failures.push(format!("hostile reason names no enrollment: {reason}"));
            }
        }
        other => failures.push(format!("evil.com snapshot: wrong outcome: {other:?}")),
    }
    if scope_version(&store) != Some(5) {
        failures.push(format!(
            "SCOPE STORE CHANGED under attack: {:?}",
            scope_version(&store)
        ));
    }
    if quarantine.len() != 2 {
        failures.push(format!(
            "quarantine has {} entries, want 2",
            quarantine.len()
        ));
    }
    let mut evidence = quarantine.clone();
    evidence.push(format!(
        "scope retained at v{}",
        scope_version(&store).unwrap_or(0)
    ));
    evidence.push("backend: ScriptedFeed + SignedFeed (MOCK)".to_string());
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "refusal": "HostileTarget",
            "scope_version": scope_version(&store),
            "quarantined": quarantine.len(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: a snapshot with an invalid signature is rejected before any
/// target is parsed. The snapshot deliberately also carries a hostile
/// target — the refusal must be `BadSignature`, proving the targets
/// were never inspected.
fn case_bad_signature_rejected_before_parse() -> Result<CaseReport, TaskDriverError> {
    let enrollment = enrollment();
    let mut store = baseline_store()?;
    let mut quarantine: Vec<String> = Vec::new();
    let v6 = snapshot(
        6,
        NOW + 10,
        vec![
            target("t-app", TargetKind::Domain, "app.example.com"),
            target("t-evil", TargetKind::Domain, "*.evil.com"),
        ],
    );
    let mut feed = SignedFeed::new(ScriptedFeed::new(vec![v6.clone()]));
    feed.tag_with_wrong_key("attacker-key", &v6);
    let mut failures = Vec::new();
    match ingest(&mut feed, &mut store, &enrollment, &mut quarantine) {
        Err(IngestError::BadSignature { version: 6 }) => {}
        other => failures.push(format!("bad signature: wrong outcome: {other:?}")),
    }
    if scope_version(&store) != Some(5) {
        failures.push("scope store changed on bad signature".to_string());
    }
    if quarantine.len() != 1 || !quarantine[0].contains("bad signature") {
        failures.push(format!("quarantine wrong: {quarantine:?}"));
    }
    let mut evidence = quarantine.clone();
    evidence.push(format!(
        "scope retained at v{}",
        scope_version(&store).unwrap_or(0)
    ));
    evidence.push("refusal is BadSignature, not HostileTarget: targets never parsed".to_string());
    evidence.push("backend: ScriptedFeed + SignedFeed (MOCK)".to_string());
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "refusal": "BadSignature",
            "scope_version": scope_version(&store),
            "quarantined": quarantine.len(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1: the feed widens an enrolled target (`10.0.0.0/24` → `10.0.0.0/8`)
/// → refused as hostile. A widened CIDR is a superset of the enrolled
/// scope, never the enrolled scope.
fn case_cidr_widening_refused() -> Result<CaseReport, TaskDriverError> {
    let enrollment = enrollment();
    let mut store = baseline_store()?;
    let mut quarantine: Vec<String> = Vec::new();
    let v6 = snapshot(
        6,
        NOW + 10,
        vec![
            target("t-app", TargetKind::Domain, "app.example.com"),
            target("t-net", TargetKind::Cidr, "10.0.0.0/8"),
        ],
    );
    let mut feed = SignedFeed::new(ScriptedFeed::new(vec![v6.clone()]));
    feed.tag_version(&enrollment.feed_key, &v6);
    let mut failures = Vec::new();
    match ingest(&mut feed, &mut store, &enrollment, &mut quarantine) {
        Err(IngestError::Feed(FeedError::HostileTarget { value, .. })) if value == "10.0.0.0/8" => {
        }
        other => failures.push(format!("CIDR widening: wrong outcome: {other:?}")),
    }
    let joined = quarantine.join("\n");
    if !joined.contains("widens enrolled target 10.0.0.0/24") {
        failures.push(format!("widening reason missing:\n{joined}"));
    }
    if scope_version(&store) != Some(5) {
        failures.push("scope store changed on CIDR widening".to_string());
    }
    let mut evidence = quarantine.clone();
    evidence.push(format!(
        "scope retained at v{}",
        scope_version(&store).unwrap_or(0)
    ));
    evidence.push("backend: ScriptedFeed + SignedFeed (MOCK)".to_string());
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "refusal": "HostileTarget",
            "scope_version": scope_version(&store),
            "quarantined": quarantine.len(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2: the feed replays an old signed snapshot (v3 after v5 was filed)
/// → version check rejects. Monotonic versions only.
fn case_replayed_version_rejected() -> Result<CaseReport, TaskDriverError> {
    let enrollment = enrollment();
    let mut store = baseline_store()?;
    let mut quarantine: Vec<String> = Vec::new();
    // Old, valid, correctly signed — the replay is the only offense.
    let v3 = snapshot(
        3,
        NOW - 100,
        vec![target("t-app", TargetKind::Domain, "app.example.com")],
    );
    let mut feed = SignedFeed::new(ScriptedFeed::new(vec![v3.clone()]));
    feed.tag_version(&enrollment.feed_key, &v3);
    let mut failures = Vec::new();
    match ingest(&mut feed, &mut store, &enrollment, &mut quarantine) {
        Err(IngestError::StaleVersion { got: 3, latest: 5 }) => {}
        other => failures.push(format!("replayed version: wrong outcome: {other:?}")),
    }
    if scope_version(&store) != Some(5) {
        failures.push("scope store changed on version replay".to_string());
    }
    if quarantine.len() != 1 || !quarantine[0].contains("replayed version") {
        failures.push(format!("quarantine wrong: {quarantine:?}"));
    }
    let mut evidence = quarantine.clone();
    evidence.push(format!(
        "scope retained at v{}",
        scope_version(&store).unwrap_or(0)
    ));
    evidence.push("backend: ScriptedFeed + SignedFeed (MOCK)".to_string());
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "refusal": "StaleVersion",
            "scope_version": scope_version(&store),
            "quarantined": quarantine.len(),
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
        "out_of_bounds_target_refused" => case_out_of_bounds_target_refused(),
        "bad_signature_rejected_before_parse" => case_bad_signature_rejected_before_parse(),
        "cidr_widening_refused" => case_cidr_widening_refused(),
        "replayed_version_rejected" => case_replayed_version_rejected(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-150: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// hostile-target refusal itself.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-150".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-150".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
