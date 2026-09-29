//! The approval queue: request -> pending -> human decision.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::delta::{PermissionDelta, PermissionSet};
use crate::error::Error;
use crate::scope::{Request, Scope, check_plain};

/// Maximum records one queue holds. History is append-only, so a full queue
/// rejects new requests instead of evicting old decisions.
pub const QUEUE_RECORDS_MAX: usize = 1024;
/// Maximum configured operators per queue.
pub const OPERATORS_MAX: usize = 16;
/// Suggested request lifetime.
pub const DEFAULT_TTL: Duration = Duration::from_secs(300);
/// Longest accepted request lifetime.
pub const TTL_MAX: Duration = Duration::from_secs(24 * 60 * 60);
/// Actor names that denote a model or the harness itself. They can never be
/// configured as operators, compared case-insensitively.
pub const RESERVED_ACTORS: [&str; 5] = ["agent", "model", "assistant", "system", "tool"];

/// Source of queue serials; each queue's IDs embed its serial, so an ID
/// issued by one queue is unknown to every other queue.
static QUEUE_SERIAL_NEXT: AtomicU64 = AtomicU64::new(1);

/// Lifecycle of one approval record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Pending,
    Approved,
    Denied,
    Expired,
    Revoked,
}

impl State {
    /// The wire name of this state.
    pub fn as_str(self) -> &'static str {
        match self {
            State::Pending => "pending",
            State::Approved => "approved",
            State::Denied => "denied",
            State::Expired => "expired",
            State::Revoked => "revoked",
        }
    }
}

/// A human operator's answer to a pending request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HumanVerdict {
    Approve,
    Deny,
}

/// A snapshot of one approval record.
///
/// Values returned by [`ApprovalQueue::get`] and [`ApprovalQueue::pending`]
/// are owned copies: editing them changes nothing in the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub id: String,
    pub run_id: String,
    pub scope: Scope,
    pub permissions_before: PermissionSet,
    pub permissions_after: PermissionSet,
    /// Computed from the two sets at admission; never model-supplied.
    pub permission_delta: PermissionDelta,
    /// Model-authored display text; never consulted for any decision.
    pub summary: Option<String>,
    pub state: State,
    pub created_at: Instant,
    pub deadline: Instant,
    /// The operator who approved or denied, kept after revocation.
    pub decided_by: Option<String>,
    pub decided_at: Option<Instant>,
    pub revoked_by: Option<String>,
    pub revoked_at: Option<Instant>,
}

/// In-memory approval queue bound to an explicit operator allowlist.
///
/// Deliberately not `Clone`: a copied queue would let one consumed ID be
/// decided twice. Every transition takes `&mut self`, so a shared `&` read
/// handle cannot approve, deny or revoke.
#[derive(Debug)]
pub struct ApprovalQueue {
    serial: u64,
    operators: BTreeSet<String>,
    records: Vec<Record>,
}

impl ApprovalQueue {
    /// Create a queue whose decisions only the listed operators may make.
    ///
    /// Rejects an empty list, more than [`OPERATORS_MAX`] names, invalid
    /// names, and any name in [`RESERVED_ACTORS`].
    pub fn new(operators: &[&str]) -> Result<Self, Error> {
        if operators.is_empty() {
            return Err(Error::InvalidValue {
                field: "operators",
                reason: "at least one operator is required",
            });
        }
        if operators.len() > OPERATORS_MAX {
            return Err(Error::TooMany {
                field: "operators",
                max: OPERATORS_MAX,
            });
        }
        for name in operators {
            check_plain("operators", name)?;
            if is_reserved(name) {
                return Err(Error::InvalidValue {
                    field: "operators",
                    reason: "reserved non-human actor",
                });
            }
        }
        let serial = QUEUE_SERIAL_NEXT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .map_err(|_| Error::InvalidValue {
                field: "queue",
                reason: "queue serials exhausted",
            })?;
        Ok(ApprovalQueue {
            serial,
            operators: operators.iter().map(|name| (*name).to_owned()).collect(),
            records: Vec::new(),
        })
    }

    /// Admit `request` as a pending record that expires after `ttl`.
    ///
    /// Takes ownership: the caller keeps no alias to the stored scope or
    /// permission sets. Returns the new record's queue-bound ID.
    pub fn request(
        &mut self,
        run_id: &str,
        request: Request,
        ttl: Duration,
    ) -> Result<String, Error> {
        check_plain("run_id", run_id)?;
        if ttl.is_zero() || ttl > TTL_MAX {
            return Err(Error::InvalidValue {
                field: "ttl",
                reason: "must be positive and at most TTL_MAX",
            });
        }
        if self.records.len() >= QUEUE_RECORDS_MAX {
            return Err(Error::TooMany {
                field: "queue",
                max: QUEUE_RECORDS_MAX,
            });
        }
        let created_at = Instant::now();
        let deadline = created_at.checked_add(ttl).ok_or(Error::InvalidValue {
            field: "ttl",
            reason: "deadline overflows the clock",
        })?;
        let id = format!("apr-{}-{}", self.serial, self.records.len());
        let Request {
            scope,
            permissions_before,
            permissions_after,
            summary,
        } = request;
        self.records.push(Record {
            id: id.clone(),
            run_id: run_id.to_owned(),
            permission_delta: PermissionDelta::between(&permissions_before, &permissions_after),
            scope,
            permissions_before,
            permissions_after,
            summary,
            state: State::Pending,
            created_at,
            deadline,
            decided_by: None,
            decided_at: None,
            revoked_by: None,
            revoked_at: None,
        });
        Ok(id)
    }

    /// Snapshot of one record, or `None` for an ID this queue never issued.
    pub fn get(&self, id: &str) -> Option<Record> {
        self.records.iter().find(|record| record.id == id).cloned()
    }

    /// Snapshots of records still awaiting a decision and not yet past their
    /// deadline, in admission order. Bounded by [`QUEUE_RECORDS_MAX`].
    pub fn pending(&self) -> Vec<Record> {
        let now = Instant::now();
        self.records
            .iter()
            .filter(|record| record.state == State::Pending && now < record.deadline)
            .cloned()
            .collect()
    }

    /// Record a human operator's decision on a pending request.
    ///
    /// Refuses unknown IDs, missing or unlisted actors, and non-pending
    /// records; all leave the record unchanged. A request past its deadline
    /// is marked `Expired` (even if no sweep ran) and refused.
    pub fn decide(
        &mut self,
        id: &str,
        verdict: HumanVerdict,
        actor: Option<&str>,
    ) -> Result<(), Error> {
        let actor = self.operator(actor)?;
        let record = self.record_mut(id)?;
        if record.state != State::Pending {
            return Err(Error::WrongState {
                state: record.state,
            });
        }
        let now = Instant::now();
        if now >= record.deadline {
            record.state = State::Expired;
            return Err(Error::Expired);
        }
        record.state = match verdict {
            HumanVerdict::Approve => State::Approved,
            HumanVerdict::Deny => State::Denied,
        };
        record.decided_by = Some(actor);
        record.decided_at = Some(now);
        Ok(())
    }

    /// Withdraw an approval. Only `Approved` records can be revoked; the
    /// original decision's attribution is kept alongside the revocation.
    pub fn revoke(&mut self, id: &str, actor: Option<&str>) -> Result<(), Error> {
        let actor = self.operator(actor)?;
        let record = self.record_mut(id)?;
        if record.state != State::Approved {
            return Err(Error::WrongState {
                state: record.state,
            });
        }
        record.state = State::Revoked;
        record.revoked_by = Some(actor);
        record.revoked_at = Some(Instant::now());
        Ok(())
    }

    /// Mark every pending record whose deadline is at or before `now` as
    /// `Expired`. Returns how many records changed.
    pub fn sweep_expired(&mut self, now: Instant) -> usize {
        let mut expired_count = 0;
        for record in &mut self.records {
            if record.state == State::Pending && now >= record.deadline {
                record.state = State::Expired;
                expired_count += 1;
            }
        }
        expired_count
    }

    fn operator(&self, actor: Option<&str>) -> Result<String, Error> {
        match actor {
            Some(name) if self.operators.contains(name) => Ok(name.to_owned()),
            _ => Err(Error::ActorRefused),
        }
    }

    fn record_mut(&mut self, id: &str) -> Result<&mut Record, Error> {
        self.records
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or(Error::UnknownId)
    }
}

fn is_reserved(name: &str) -> bool {
    RESERVED_ACTORS
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(name))
}
