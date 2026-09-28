//! The operator-approval boundary on submission. No auto-submit, ever:
//! `submit` requires a live operator approval bound to the exact payload
//! bytes, and each approval nonce is single-use (replay refused).

use crate::bounty::types::Approval;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// A submitted report: the finding, the exact bytes sent, and the
/// approval nonce that authorized them.
#[derive(Clone, Debug)]
pub struct Submission {
    pub finding_id: String,
    pub payload_hash: String,
    pub approval_nonce: u64,
    pub submitted_at: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum GateError {
    NoApproval,
    Expired,
    ScopeVersionMismatch,
    HashMismatch { expected: String, got: String },
    ReplayNonce { nonce: u64 },
    FindingNotApproved,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// The last human checkpoint. Holds the spent-nonce set; every
/// submission is validated for liveness, scope binding, exact
/// payload-hash binding, and single nonce use.
pub struct SubmissionGate {
    spent_nonces: HashSet<u64>,
}

impl SubmissionGate {
    pub fn new() -> Self {
        SubmissionGate {
            spent_nonces: HashSet::new(),
        }
    }

    /// Authorize submitting `payload_bytes` for an Approved finding.
    /// `approved_hash` is the sha256 the operator approved: any byte
    /// difference refuses the submission. Fails closed on every
    /// mismatch; the nonce is spent exactly once.
    #[allow(clippy::too_many_arguments)]
    pub fn submit(
        &mut self,
        finding_id: &str,
        finding_approved: bool,
        payload_bytes: &[u8],
        approved_hash: &str,
        approval: Option<&Approval>,
        program_id: &str,
        scope_version: u64,
        now: u64,
    ) -> Result<Submission, GateError> {
        if !finding_approved {
            return Err(GateError::FindingNotApproved);
        }
        let a = approval.ok_or(GateError::NoApproval)?;
        if a.issuer != "operator" {
            return Err(GateError::NoApproval);
        }
        if a.program_id != program_id || a.scope_version != scope_version {
            return Err(GateError::ScopeVersionMismatch);
        }
        if !(a.granted_at <= now && now < a.expires_at) {
            return Err(GateError::Expired);
        }
        let got = sha256_hex(payload_bytes);
        if got != approved_hash {
            return Err(GateError::HashMismatch {
                expected: approved_hash.to_string(),
                got,
            });
        }
        if !self.spent_nonces.insert(a.nonce) {
            return Err(GateError::ReplayNonce { nonce: a.nonce });
        }
        Ok(Submission {
            finding_id: finding_id.to_string(),
            payload_hash: got,
            approval_nonce: a.nonce,
            submitted_at: now,
        })
    }

    pub fn spent_nonce_count(&self) -> usize {
        self.spent_nonces.len()
    }
}

impl Default for SubmissionGate {
    fn default() -> Self {
        Self::new()
    }
}
