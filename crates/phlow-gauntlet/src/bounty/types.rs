//! Core types for the bug-bounty cyclical workflow, plus the finding
//! state machine. Illegal transitions are rejected with a typed error —
//! never silently mapped.

use std::collections::HashMap;

/// Opaque target identifier, stable across scope snapshots.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TargetId(pub String);

/// What kind of thing a target is. The kind determines which probers may
/// touch it; kinds are never inferred from the value string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetKind {
    Domain,
    Url,
    Cidr,
}

/// A single in-scope testable target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub id: TargetId,
    pub kind: TargetKind,
    pub value: String,
}

/// One polled scope version. Versions are monotonic; a feed that replays
/// an old version is hostile (task 150).
#[derive(Clone, Debug)]
pub struct ScopeSnapshot {
    pub version: u64,
    pub targets: Vec<Target>,
    pub fetched_at: u64,
}

/// The difference between two snapshots, keyed by target id. A value
/// change under a stable id is `changed`, never add+remove.
#[derive(Clone, Debug, Default)]
pub struct ScopeDiff {
    pub added: Vec<Target>,
    pub removed: Vec<Target>,
    pub changed: Vec<(Target, Target)>,
}

/// Program enrollment: the rules of engagement that bind every cycle.
#[derive(Clone, Debug)]
pub struct Program {
    pub id: String,
    pub name: String,
    pub window: TestingWindow,
    pub rate_limit: RateLimit,
    /// Suffixes the program actually covers, e.g. ["example.com"].
    /// A scope feed that names anything outside these is hostile.
    pub allowed_suffixes: Vec<String>,
    /// Maximum CIDR prefix length the feed may widen to (e.g. 24 means
    /// /24 or narrower only). Widening beyond this is hostile.
    pub max_cidr_prefix: u8,
}

/// Daily testing window, minutes since midnight UTC, half-open
/// [start_min, end_min). May wrap midnight (start > end).
#[derive(Clone, Debug)]
pub struct TestingWindow {
    pub start_min: u16,
    pub end_min: u16,
}

impl TestingWindow {
    /// True when `now_secs` (unix epoch) falls inside the window, in UTC.
    pub fn allows(&self, now_secs: u64) -> bool {
        let minute = ((now_secs / 60) % 1440) as u16;
        if self.start_min <= self.end_min {
            minute >= self.start_min && minute < self.end_min
        } else {
            minute >= self.start_min || minute < self.end_min
        }
    }
}

/// Token-bucket parameters. The scheduler enforces these; the platform
/// may additionally answer 429 (task 140).
#[derive(Clone, Debug)]
pub struct RateLimit {
    pub requests_per_minute: u32,
    pub burst: u32,
}

/// Operator approval: a capability token with macaroon-style caveats.
/// Binds a scope version and a time window; the submission gate
/// additionally binds the exact payload hash and a single-use nonce.
#[derive(Clone, Debug)]
pub struct Approval {
    pub program_id: String,
    pub scope_version: u64,
    pub granted_at: u64,
    pub expires_at: u64,
    pub nonce: u64,
    /// Provenance marker: approvals must come from the operator
    /// authority, never from a feed or a target. Forged markers are
    /// refused by shape *and* provenance (task 134).
    pub issuer: String,
}

impl Approval {
    /// Live, for this program, bound to this scope version, right now.
    pub fn valid_for(&self, program_id: &str, scope_version: u64, now: u64) -> bool {
        self.issuer == "operator"
            && self.program_id == program_id
            && self.scope_version == scope_version
            && self.granted_at <= now
            && now < self.expires_at
    }
}

/// One probe execution against one target.
#[derive(Clone, Debug)]
pub struct Run {
    pub id: String,
    pub target_id: TargetId,
    pub state: RunState,
    pub approval_nonce: u64,
    pub cancel_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunState {
    Queued,
    Running,
    Cancelled,
    Finished,
    Failed,
}

/// Raw tool output preserved byte-exact, with a hash and an append-only
/// custody log. Tampering is detectable (task 141).
#[derive(Clone, Debug)]
pub struct Evidence {
    pub raw: Vec<u8>,
    pub sha256: String,
    pub custody: Vec<CustodyEntry>,
    pub truncated: bool,
}

#[derive(Clone, Debug)]
pub struct CustodyEntry {
    pub handler: String,
    pub action: String,
    pub at: u64,
    pub evidence_sha256: String,
}

/// A candidate vulnerability. The fingerprint is the dedup key across
/// cycles (task 139); the state machine below governs its lifecycle.
#[derive(Clone, Debug)]
pub struct Finding {
    pub id: String,
    pub target_id: TargetId,
    pub fingerprint: String,
    pub title: String,
    pub state: FindingState,
    pub evidence: Evidence,
    pub observation_count: u32,
    pub reject_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindingState {
    Candidate,
    Validated,
    Reportable,
    Approved,
    Submitted,
    Triage,
    Accepted,
    Duplicate,
    NeedsMoreInfo,
    Closed,
    Rejected,
}

/// Typed rejection for an illegal finding-state transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IllegalTransition {
    pub from: String,
    pub to: String,
}

impl FindingState {
    fn name(&self) -> &'static str {
        match self {
            FindingState::Candidate => "Candidate",
            FindingState::Validated => "Validated",
            FindingState::Reportable => "Reportable",
            FindingState::Approved => "Approved",
            FindingState::Submitted => "Submitted",
            FindingState::Triage => "Triage",
            FindingState::Accepted => "Accepted",
            FindingState::Duplicate => "Duplicate",
            FindingState::NeedsMoreInfo => "NeedsMoreInfo",
            FindingState::Closed => "Closed",
            FindingState::Rejected => "Rejected",
        }
    }

    /// The legal transitions. Everything else is rejected. Terminal
    /// states (Accepted, Duplicate, Closed, Rejected) have no exits.
    /// NeedsMoreInfo returns to Validated — never to recon.
    pub fn can_transition_to(&self, next: &FindingState) -> bool {
        use FindingState::*;
        matches!(
            (self, next),
            (Candidate, Validated)
                | (Candidate, Rejected)
                | (Validated, Reportable)
                | (Validated, Rejected)
                | (Reportable, Approved)
                | (Reportable, Rejected)
                | (Approved, Submitted)
                | (Submitted, Triage)
                | (Triage, Accepted)
                | (Triage, Duplicate)
                | (Triage, NeedsMoreInfo)
                | (Triage, Closed)
                | (NeedsMoreInfo, Validated)
        )
    }

    pub fn transition(&self, next: FindingState) -> Result<FindingState, IllegalTransition> {
        if self.can_transition_to(&next) {
            Ok(next)
        } else {
            Err(IllegalTransition {
                from: self.name().to_string(),
                to: next.name().to_string(),
            })
        }
    }

    pub fn is_terminal(&self) -> bool {
        use FindingState::*;
        matches!(self, Accepted | Duplicate | Closed | Rejected)
    }
}

/// Operator-extensible per-finding metadata (report fields live here).
pub type FindingFields = HashMap<String, String>;
