//! In-memory stores: scope history, the target queue, the run ledger,
//! the finding store (dedup by fingerprint + content hash), and the
//! append-only audit log with write-time secret redaction.

use crate::bounty::approve::sha256_hex;
use crate::bounty::secret::redact_text;
use crate::bounty::types::{
    Finding, FindingState, IllegalTransition, Run, RunState, ScopeSnapshot, Target, TargetId,
};
use std::collections::{HashMap, VecDeque};

/// Latest scope plus full version history. Filing is monotonic: a
/// snapshot with version <= latest is rejected (replay protection).
#[derive(Debug, Default)]
pub struct ScopeStore {
    history: Vec<ScopeSnapshot>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ScopeStoreError {
    StaleVersion { got: u64, latest: u64 },
}

impl ScopeStore {
    pub fn new() -> Self {
        ScopeStore {
            history: Vec::new(),
        }
    }

    pub fn file(&mut self, snap: ScopeSnapshot) -> Result<bool, ScopeStoreError> {
        if let Some(latest) = self.history.last() {
            if snap.version < latest.version {
                return Err(ScopeStoreError::StaleVersion {
                    got: snap.version,
                    latest: latest.version,
                });
            }
            if snap.version == latest.version {
                return Ok(false); // idempotent re-file
            }
        }
        self.history.push(snap);
        Ok(true)
    }

    pub fn latest(&self) -> Option<&ScopeSnapshot> {
        self.history.last()
    }

    pub fn version_count(&self) -> usize {
        self.history.len()
    }
}

/// FIFO queue of targets awaiting probes, with cancel-by-id.
#[derive(Debug, Default)]
pub struct TargetQueue {
    queue: VecDeque<Target>,
}

impl TargetQueue {
    pub fn new() -> Self {
        TargetQueue {
            queue: VecDeque::new(),
        }
    }

    pub fn push(&mut self, t: Target) {
        self.queue.push_back(t);
    }

    pub fn pop(&mut self) -> Option<Target> {
        self.queue.pop_front()
    }

    /// Remove a queued target by id. Returns true when something was
    /// actually removed.
    pub fn cancel(&mut self, id: &TargetId) -> bool {
        let before = self.queue.len();
        self.queue.retain(|t| &t.id != id);
        self.queue.len() != before
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn contains(&self, id: &TargetId) -> bool {
        self.queue.iter().any(|t| &t.id == id)
    }
}

/// The crash-resumable record of every run. `pending()` returns the
/// targets that still need probing: queued or running-but-unfinished.
/// Finished/failed/cancelled runs are never re-probed by resume.
#[derive(Debug, Default)]
pub struct RunLedger {
    runs: Vec<Run>,
}

impl RunLedger {
    pub fn new() -> Self {
        RunLedger { runs: Vec::new() }
    }

    pub fn record(&mut self, run: Run) {
        self.runs.push(run);
    }

    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    pub fn set_state(&mut self, run_id: &str, state: RunState, reason: Option<String>) {
        if let Some(r) = self.runs.iter_mut().find(|r| r.id == run_id) {
            r.state = state;
            if reason.is_some() {
                r.cancel_reason = reason;
            }
        }
    }

    /// Targets with no terminal run yet.
    pub fn pending_target_ids(&self) -> Vec<TargetId> {
        let mut terminal: HashMap<&TargetId, bool> = HashMap::new();
        for r in &self.runs {
            let done = matches!(
                r.state,
                RunState::Finished | RunState::Failed | RunState::Cancelled
            );
            terminal
                .entry(&r.target_id)
                .and_modify(|d| *d = *d && done)
                .or_insert(done);
        }
        terminal
            .into_iter()
            .filter(|(_, done)| !done)
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub fn count_in_state(&self, state: RunState) -> usize {
        self.runs.iter().filter(|r| r.state == state).count()
    }
}

/// Finding storage with cross-cycle dedup. The dedup key is the
/// composite (fingerprint, content_hash): a repeat observation of
/// identical content bumps `observation_count` on the existing record;
/// different content under the same fingerprint is stored as a
/// distinct record (the collision tiebreak — no silent data loss).
/// Rejection is sticky per composite key: re-inserting identical
/// content of a rejected finding stays rejected.
#[derive(Debug, Default)]
pub struct FindingStore {
    by_fingerprint: HashMap<String, Vec<Finding>>,
    next_id: u64,
}

/// Deterministic content identity for a finding: sha256 over the
/// canonical field sequence in fixed order, each field
/// length-prefixed so field boundaries cannot collide.
///
/// Included:
/// - `target_id`: findings are target-bound; the same fingerprint on
///   different targets is a different finding.
/// - `fingerprint`: the producer's coarse identity claim; keeps the
///   hash meaningful standalone and preserves the fingerprint's role
///   in the composite key.
/// - `title`: the human-readable claim; a retitled finding is
///   different content (this is what splits the task-139 collision
///   arm: same fingerprint, different titles).
/// - `evidence.sha256`: content identity of the raw evidence without
///   re-hashing large byte buffers (sealed as sha256(raw) at capture,
///   task 141).
///
/// Excluded:
/// - `id`: store-assigned (`f{:06}`); hashing it would make every
///   insert unique and destroy dedup. Candidates also arrive with
///   empty/placeholder ids.
/// - `state`: lifecycle — a Candidate -> Validated transition must not
///   fork the record.
/// - `observation_count`: the dedup accumulator itself; including it
///   would prevent merging.
/// - `reject_reason`: post-validation annotation; a rejected finding
///   re-observed must merge back onto its rejected record (stickiness).
/// - `custody` (handler/action/at/evidence_sha256): per-observation
///   metadata. `at` timestamps are volatile — every re-observation
///   would hash differently and cross-cycle dedup would break;
///   handler/action vary by who sealed it; evidence_sha256 duplicates
///   evidence.sha256.
/// - `evidence.truncated`: capture metadata, not content; the bytes it
///   describes are already identified by evidence.sha256.
/// - `evidence.raw`: covered by evidence.sha256 (hash-of-hash avoids
///   O(bytes) work per insert).
///
/// sha256 (via `approve::sha256_hex`, the crate's canonical hash) is
/// used instead of `std`'s `DefaultHasher` deliberately:
/// DefaultHasher (SipHash) is keyed with a RANDOM seed per instance,
/// so equal findings hashed by different instances produce different
/// digests and cross-insert comparisons would silently fail.
pub fn content_hash(f: &Finding) -> String {
    let mut bytes = Vec::new();
    for field in [
        f.target_id.0.as_str(),
        f.fingerprint.as_str(),
        f.title.as_str(),
        f.evidence.sha256.as_str(),
    ] {
        let field_bytes = field.as_bytes();
        bytes.extend_from_slice(&(field_bytes.len() as u64).to_le_bytes());
        bytes.extend_from_slice(field_bytes);
    }
    sha256_hex(&bytes)
}

/// Rejection for `FindingStore::transition`: unknown record ids are a
/// typed error, never a panic; illegal state moves keep the existing
/// `IllegalTransition` shape.
#[derive(Debug, PartialEq, Eq)]
pub enum StoreTransitionError {
    UnknownId { id: String },
    Illegal(IllegalTransition),
}

impl FindingStore {
    pub fn new() -> Self {
        FindingStore {
            by_fingerprint: HashMap::new(),
            next_id: 1,
        }
    }

    /// Insert a candidate finding. Returns (record id, is_new_record).
    /// Same (fingerprint, content_hash): bumps `observation_count` on
    /// the existing record. Same fingerprint, different content hash:
    /// stores a distinct record with a fresh id.
    pub fn insert(&mut self, mut f: Finding) -> (String, bool) {
        let hash = content_hash(&f);
        if let Some(existing) = self
            .by_fingerprint
            .get_mut(&f.fingerprint)
            .and_then(|bucket| bucket.iter_mut().find(|r| content_hash(r) == hash))
        {
            existing.observation_count += 1;
            return (existing.id.clone(), false);
        }
        let id = format!("f{:06}", self.next_id);
        self.next_id += 1;
        f.id = id.clone();
        if f.observation_count == 0 {
            f.observation_count = 1;
        }
        self.by_fingerprint
            .entry(f.fingerprint.clone())
            .or_default()
            .push(f);
        (id, true)
    }

    /// Every record stored under a fingerprint, in insertion order;
    /// empty when the fingerprint is unknown. Never assume one record:
    /// a fingerprint collision stores several — pick explicitly.
    pub fn findings_for(&self, fp: &str) -> &[Finding] {
        self.by_fingerprint
            .get(fp)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Total records across all fingerprints.
    pub fn record_count(&self) -> usize {
        self.by_fingerprint.values().map(Vec::len).sum()
    }

    /// Transition a stored finding's state through the legal machine,
    /// addressed by record id — unambiguous even under fingerprint
    /// collision.
    pub fn transition(&mut self, id: &str, next: FindingState) -> Result<(), StoreTransitionError> {
        let record = self
            .by_fingerprint
            .values_mut()
            .flat_map(|bucket| bucket.iter_mut())
            .find(|r| r.id == id)
            .ok_or_else(|| StoreTransitionError::UnknownId { id: id.to_string() })?;
        record.state = record
            .state
            .transition(next)
            .map_err(StoreTransitionError::Illegal)?;
        Ok(())
    }
}

/// Append-only audit log. Secrets are redacted at write time against the
/// configured secret list — the raw value never reaches the entries.
#[derive(Debug)]
pub struct AuditLog {
    entries: Vec<String>,
    secrets: Vec<String>,
}

impl AuditLog {
    pub fn new(secrets: Vec<String>) -> Self {
        AuditLog {
            entries: Vec::new(),
            secrets,
        }
    }

    pub fn append(&mut self, line: &str) {
        self.entries.push(redact_text(line, &self.secrets));
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// True when no secret substring appears in any entry.
    pub fn leaks_none(&self) -> bool {
        !self
            .entries
            .iter()
            .any(|e| self.secrets.iter().any(|s| !s.is_empty() && e.contains(s)))
    }
}
