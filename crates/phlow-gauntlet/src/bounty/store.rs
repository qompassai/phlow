//! In-memory stores: scope history, the target queue, the run ledger,
//! the finding store (dedup by fingerprint), and the append-only audit
//! log with write-time secret redaction.

use crate::bounty::secret::redact_text;
use crate::bounty::types::{Finding, FindingState, Run, RunState, ScopeSnapshot, Target, TargetId};
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

/// Finding storage with cross-cycle dedup. `insert` keys on the
/// fingerprint: a repeat observation bumps `observation_count` on the
/// existing record instead of creating a new one. Rejection is sticky:
/// re-inserting a rejected fingerprint stays rejected.
#[derive(Debug, Default)]
pub struct FindingStore {
    by_fingerprint: HashMap<String, Finding>,
    next_id: u64,
}

impl FindingStore {
    pub fn new() -> Self {
        FindingStore {
            by_fingerprint: HashMap::new(),
            next_id: 1,
        }
    }

    /// Insert a candidate finding. Returns (record id, is_new_record).
    pub fn insert(&mut self, mut f: Finding) -> (String, bool) {
        if let Some(existing) = self.by_fingerprint.get_mut(&f.fingerprint) {
            existing.observation_count += 1;
            return (existing.id.clone(), false);
        }
        let id = format!("f{:06}", self.next_id);
        self.next_id += 1;
        f.id = id.clone();
        if f.observation_count == 0 {
            f.observation_count = 1;
        }
        self.by_fingerprint.insert(f.fingerprint.clone(), f);
        (id, true)
    }

    pub fn get_by_fingerprint(&self, fp: &str) -> Option<&Finding> {
        self.by_fingerprint.get(fp)
    }

    pub fn record_count(&self) -> usize {
        self.by_fingerprint.len()
    }

    /// Transition a stored finding's state through the legal machine.
    pub fn transition(
        &mut self,
        fp: &str,
        next: FindingState,
    ) -> Result<(), crate::bounty::types::IllegalTransition> {
        let f = self
            .by_fingerprint
            .get_mut(fp)
            .expect("bounty: transition of unknown fingerprint");
        f.state = f.state.transition(next)?;
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
