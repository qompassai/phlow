//! The finding validation pipeline: mechanical checks that must all
//! pass before a finding becomes reportable. A check that errors fails
//! closed — the finding is NOT reportable.

use crate::bounty::store::{FindingStore, content_hash};
use crate::bounty::types::{Finding, ScopeSnapshot};

/// Per-check context: what the check may consult. Nothing else.
pub struct CheckCtx<'a> {
    pub scope: Option<&'a ScopeSnapshot>,
    pub store: &'a FindingStore,
}

/// A check returns Pass, Fail (with reason), or Error (infrastructure
/// problem — fails the finding closed).
#[derive(Debug, PartialEq, Eq)]
pub enum CheckResult {
    Pass,
    Fail { reason: String },
    Error { reason: String },
}

/// One mechanical validation check.
pub trait Check {
    fn name(&self) -> &'static str;
    fn check(&self, finding: &Finding, ctx: &CheckCtx) -> CheckResult;
}

/// The finding's target is in the current scope snapshot.
pub struct InScopeCheck;
impl Check for InScopeCheck {
    fn name(&self) -> &'static str {
        "in-scope"
    }
    fn check(&self, finding: &Finding, ctx: &CheckCtx) -> CheckResult {
        match ctx.scope {
            None => CheckResult::Error {
                reason: "scope store unreachable".to_string(),
            },
            Some(snap) => {
                if snap.targets.iter().any(|t| t.id == finding.target_id) {
                    CheckResult::Pass
                } else {
                    CheckResult::Fail {
                        reason: "target not in current scope".to_string(),
                    }
                }
            }
        }
    }
}

/// The finding carries non-empty evidence.
pub struct EvidencePresentCheck;
impl Check for EvidencePresentCheck {
    fn name(&self) -> &'static str {
        "evidence-present"
    }
    fn check(&self, finding: &Finding, _ctx: &CheckCtx) -> CheckResult {
        if finding.evidence.raw.is_empty() {
            CheckResult::Fail {
                reason: "evidence empty".to_string(),
            }
        } else {
            CheckResult::Pass
        }
    }
}

/// No other record carries the same (fingerprint, content-hash)
/// under a different record id (fresh work, not a re-report of a known
/// finding). A fingerprint collision with different content is NOT a
/// duplicate — the store keeps both records.
pub struct NonDuplicateCheck;
impl Check for NonDuplicateCheck {
    fn name(&self) -> &'static str {
        "non-duplicate"
    }
    fn check(&self, finding: &Finding, ctx: &CheckCtx) -> CheckResult {
        let hash = content_hash(finding);
        for existing in ctx.store.findings_for(&finding.fingerprint) {
            if content_hash(existing) == hash && existing.id != finding.id {
                return CheckResult::Fail {
                    reason: format!("duplicate of {}", existing.id),
                };
            }
        }
        CheckResult::Pass
    }
}

/// The pipeline: every check must Pass. The first non-Pass decides the
/// outcome; an Error fails closed (never "pass on error").
pub struct ValidationPipeline {
    checks: Vec<Box<dyn Check>>,
}

impl ValidationPipeline {
    pub fn new() -> Self {
        ValidationPipeline { checks: Vec::new() }
    }

    pub fn with_defaults() -> Self {
        let mut p = ValidationPipeline::new();
        p.add(InScopeCheck);
        p.add(EvidencePresentCheck);
        p.add(NonDuplicateCheck);
        p
    }

    pub fn add<C: Check + 'static>(&mut self, c: C) {
        self.checks.push(Box::new(c));
    }

    /// Run all checks. Returns Ok(()) only when every check passes;
    /// otherwise the failing check's name and result.
    pub fn validate(&self, finding: &Finding, ctx: &CheckCtx) -> Result<(), (String, CheckResult)> {
        for c in &self.checks {
            match c.check(finding, ctx) {
                CheckResult::Pass => {}
                other => return Err((c.name().to_string(), other)),
            }
        }
        Ok(())
    }

    pub fn check_names(&self) -> Vec<&'static str> {
        self.checks.iter().map(|c| c.name()).collect()
    }
}

impl Default for ValidationPipeline {
    fn default() -> Self {
        ValidationPipeline::with_defaults()
    }
}
