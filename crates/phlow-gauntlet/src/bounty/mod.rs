//! Shared scaffold for gauntlet tasks 131–150: the async bug-bounty
//! cyclical workflow.
//!
//! One lifecycle, many cycles: enroll → scope refresh → queue → probe
//! (bounded concurrency) → findings → dedup → validate → report → operator
//! approval → submit → track → triager feedback → next cycle.
//!
//! The scaffold is deliberately in-memory and deterministic (`ManualClock`)
//! so every task's driver can script exact scenarios. `FakePlatform`
//! models platform *mechanics* (states, 429s, triage events), not any real
//! platform's API — that labeling is load-bearing, see the design doc.

pub mod approve;
pub mod clock;
pub mod diff;
pub mod feed;
pub mod platform;
pub mod sched;
pub mod secret;
pub mod store;
pub mod types;
pub mod validate;

pub use approve::{GateError, Submission, SubmissionGate};
pub use clock::{Clock, ManualClock, SystemClock};
pub use diff::diff_scope;
pub use feed::{FeedError, ScopeFeed, ScriptedFeed};
pub use platform::{FakePlatform, TriageEvent, TriageKind};
pub use sched::{SchedAction, Scheduler};
pub use secret::{SecretVault, redact_text};
pub use store::{
    AuditLog, FindingStore, RunLedger, ScopeStore, StoreTransitionError, TargetQueue, content_hash,
};
pub use types::{
    Approval, CustodyEntry, Evidence, Finding, FindingState, Program, RateLimit, Run, RunState,
    ScopeDiff, ScopeSnapshot, Target, TargetId, TargetKind, TestingWindow,
};
pub use validate::{Check, CheckResult, ValidationPipeline};
