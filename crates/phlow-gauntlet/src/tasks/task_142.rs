//! Task 142 — finding validation pipeline (rust, V/A).
//!
//! A finding becomes reportable only by passing every mechanical check
//! in [`ValidationPipeline`]: in-scope, evidence-present, non-duplicate,
//! and reproducible. The pipeline runs checks in order and short-circuits
//! on the first non-pass, naming the failing check exactly. A check that
//! errors (infrastructure problem, e.g. the scope snapshot is
//! unreachable) fails the finding closed — never "pass on error".
//! Operator-registered checks extend the pipeline without weakening it:
//! adding a check can only make the gate stricter.
//!
//! Note: the scaffold ships three built-in checks; the design's fourth
//! built-in (reproducible) is implemented here as [`ReproducibleCheck`]
//! and registered like an operator check, which exercises the same
//! extension seam the A2 case probes. All verdict evidence comes from
//! scripted fixtures (MOCK).

use crate::bounty::validate::CheckCtx;
use crate::bounty::*;
use crate::skillopt::driver::{CaseReport, TaskDriverError, verdict_line};
use crate::skillopt::learner::Verdict;
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-142";
/// Task name.
pub const NAME: &str = "finding-validation-pipeline";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "all_checks_pass_reportable",
    "each_check_blocks_with_name",
    "validator_error_fails_closed",
    "operator_check_extends_pipeline",
];

/// Reproducibility: a finding observed at least twice counts as
/// reproduced. One observation is a sighting, not a finding.
pub struct ReproducibleCheck;

impl Check for ReproducibleCheck {
    fn name(&self) -> &'static str {
        "reproducible"
    }

    fn check(&self, finding: &Finding, _ctx: &CheckCtx) -> CheckResult {
        if finding.observation_count >= 2 {
            CheckResult::Pass
        } else {
            CheckResult::Fail {
                reason: "observed once: not reproduced".to_string(),
            }
        }
    }
}

/// Operator-registered extension check for the A2 case: titles are
/// required and bounded, so reports are never anonymous or unbounded.
pub struct TitlePresentCheck;

impl Check for TitlePresentCheck {
    fn name(&self) -> &'static str {
        "title-present"
    }

    fn check(&self, finding: &Finding, _ctx: &CheckCtx) -> CheckResult {
        if finding.title.is_empty() {
            CheckResult::Fail {
                reason: "title empty".to_string(),
            }
        } else if finding.title.len() > 200 {
            CheckResult::Fail {
                reason: "title over 200 chars".to_string(),
            }
        } else {
            CheckResult::Pass
        }
    }
}

/// The four-check pipeline under test: the scaffold's three built-ins
/// plus the reproducibility check.
fn four_check_pipeline() -> ValidationPipeline {
    let mut pipeline = ValidationPipeline::with_defaults();
    pipeline.add(ReproducibleCheck);
    pipeline
}

fn scope_v7() -> ScopeSnapshot {
    ScopeSnapshot {
        version: 7,
        targets: vec![Target {
            id: TargetId("t-web-01".to_string()),
            kind: TargetKind::Domain,
            value: "example.com".to_string(),
        }],
        fetched_at: 1_700_000_000,
    }
}

fn mock_evidence() -> Evidence {
    let raw = b"MOCK: nuclei template xss-detect matched".to_vec();
    Evidence {
        sha256: approve::sha256_hex(&raw),
        raw,
        custody: Vec::new(),
        truncated: false,
    }
}

fn empty_evidence() -> Evidence {
    Evidence {
        sha256: approve::sha256_hex(&[]),
        raw: Vec::new(),
        custody: Vec::new(),
        truncated: false,
    }
}

/// A fixture finding that passes every check unless the caller breaks
/// exactly one dimension.
fn passing_finding(fingerprint: &str) -> Finding {
    Finding {
        id: "pending".to_string(),
        target_id: TargetId("t-web-01".to_string()),
        fingerprint: fingerprint.to_string(),
        title: "mock: reflected XSS in /search".to_string(),
        state: FindingState::Candidate,
        evidence: mock_evidence(),
        observation_count: 2,
        reject_reason: None,
    }
}

/// V1: a finding passing all four checks validates clean, and the
/// finding state machine then admits Candidate -> Validated ->
/// Reportable — the pipeline verdict is what gates reportability.
fn case_all_checks_pass_reportable() -> Result<CaseReport, TaskDriverError> {
    let store = FindingStore::new();
    let scope = scope_v7();
    let pipeline = four_check_pipeline();
    let ctx = CheckCtx {
        scope: Some(&scope),
        store: &store,
    };
    let mut finding = passing_finding("fp-142-v1");
    let mut failures = Vec::new();
    match pipeline.validate(&finding, &ctx) {
        Ok(()) => {}
        Err((name, result)) => {
            failures.push(format!("clean fixture failed on {name}: {result:?}"));
        }
    }
    let names = pipeline.check_names();
    for want in [
        "in-scope",
        "evidence-present",
        "non-duplicate",
        "reproducible",
    ] {
        if !names.contains(&want) {
            failures.push(format!("pipeline missing check {want}"));
        }
    }
    for next in [FindingState::Validated, FindingState::Reportable] {
        match finding.state.transition(next) {
            Ok(s) => finding.state = s,
            Err(e) => failures.push(format!("legal transition refused: {} -> {}", e.from, e.to)),
        }
    }
    if finding.state != FindingState::Reportable {
        failures.push(format!("finding stuck at {:?}", finding.state));
    }
    let mut evidence_lines = vec![
        format!("checks run: {}", names.join(", ")),
        format!("pipeline verdict: ok; state now {:?}", finding.state),
        verdict_line("142", Verdict::Replicates, "4/4 pass -> reportable"),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "checks": names,
            "state": format!("{:?}", finding.state),
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: each check individually toggled to fail. The finding stays
/// Candidate, and the pipeline names exactly the failing check — the
/// attribution the rejection path (task 143) depends on.
fn case_each_check_blocks_with_name() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence_lines = Vec::new();

    // in-scope fails: target not in the snapshot. Everything else passes.
    {
        let store = FindingStore::new();
        let scope = scope_v7();
        let pipeline = four_check_pipeline();
        let ctx = CheckCtx {
            scope: Some(&scope),
            store: &store,
        };
        let mut f = passing_finding("fp-142-v2a");
        f.target_id = TargetId("t-evil-99".to_string());
        match pipeline.validate(&f, &ctx) {
            Err((name, CheckResult::Fail { reason })) if name == "in-scope" => {
                evidence_lines.push(format!("out-of-scope target -> {name}: {reason}"));
            }
            other => failures.push(format!("in-scope arm: wrong outcome: {other:?}")),
        }
        if f.state != FindingState::Candidate {
            failures.push("out-of-scope fixture left Candidate".to_string());
        }
    }

    // evidence-present fails: empty evidence. Scope, dedup, repro pass.
    {
        let store = FindingStore::new();
        let scope = scope_v7();
        let pipeline = four_check_pipeline();
        let ctx = CheckCtx {
            scope: Some(&scope),
            store: &store,
        };
        let mut f = passing_finding("fp-142-v2b");
        f.evidence = empty_evidence();
        match pipeline.validate(&f, &ctx) {
            Err((name, CheckResult::Fail { reason })) if name == "evidence-present" => {
                evidence_lines.push(format!("empty evidence -> {name}: {reason}"));
            }
            other => failures.push(format!("evidence-present arm: wrong outcome: {other:?}")),
        }
    }

    // non-duplicate fails: same fingerprint already stored under another id.
    {
        let mut store = FindingStore::new();
        let prior = passing_finding("fp-142-v2c");
        let (prior_id, is_new) = store.insert(prior);
        if !is_new {
            failures.push("fixture setup: prior insert not new".to_string());
        }
        let scope = scope_v7();
        let pipeline = four_check_pipeline();
        let ctx = CheckCtx {
            scope: Some(&scope),
            store: &store,
        };
        let f = passing_finding("fp-142-v2c");
        match pipeline.validate(&f, &ctx) {
            Err((name, CheckResult::Fail { reason })) if name == "non-duplicate" => {
                evidence_lines.push(format!("duplicate fingerprint -> {name}: {reason}"));
                if !reason.contains(&prior_id) {
                    failures.push("non-duplicate reason does not name the existing id".to_string());
                }
            }
            other => failures.push(format!("non-duplicate arm: wrong outcome: {other:?}")),
        }
    }

    // reproducible fails: observed once. Scope, evidence, dedup pass.
    {
        let store = FindingStore::new();
        let scope = scope_v7();
        let pipeline = four_check_pipeline();
        let ctx = CheckCtx {
            scope: Some(&scope),
            store: &store,
        };
        let mut f = passing_finding("fp-142-v2d");
        f.observation_count = 1;
        match pipeline.validate(&f, &ctx) {
            Err((name, CheckResult::Fail { reason })) if name == "reproducible" => {
                evidence_lines.push(format!("single observation -> {name}: {reason}"));
            }
            other => failures.push(format!("reproducible arm: wrong outcome: {other:?}")),
        }
    }

    evidence_lines.push(verdict_line(
        "142",
        Verdict::Replicates,
        "each single failure names its check; finding stays Candidate",
    ));
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "arms": 4,
            "blocked": 4 - failures.len(),
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1: the scope snapshot is unreachable (check errors). The pipeline
/// must fail the finding closed — the outcome is `Error`, never a
/// coerced pass, and the finding is not reportable.
fn case_validator_error_fails_closed() -> Result<CaseReport, TaskDriverError> {
    let store = FindingStore::new();
    let pipeline = four_check_pipeline();
    let ctx = CheckCtx {
        scope: None,
        store: &store,
    };
    let finding = passing_finding("fp-142-a1");
    let mut failures = Vec::new();
    match pipeline.validate(&finding, &ctx) {
        Err((name, CheckResult::Error { reason })) => {
            if name != "in-scope" {
                failures.push(format!("error attributed to wrong check: {name}"));
            }
            if !reason.contains("unreachable") {
                failures.push(format!("error reason uninformative: {reason}"));
            }
        }
        Err((name, other)) => {
            failures.push(format!(
                "validator error misclassified as {other:?} on {name}"
            ));
        }
        Ok(()) => {
            failures.push("VALIDATOR ERROR PASSED THE FINDING — fail-open".to_string());
        }
    }
    let mut evidence_lines = vec![
        "scope: None (store unreachable)".to_string(),
        "pipeline -> Err((\"in-scope\", CheckResult::Error)) — fails closed".to_string(),
        verdict_line(
            "142",
            Verdict::Replicates,
            "validator error never becomes a pass",
        ),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "fail_closed": failures.is_empty(),
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2: the operator registers a custom check. It runs in the pipeline
/// (named in `check_names`), its failure blocks reportability, and
/// previously-failing findings still fail — extension never weakens.
fn case_operator_check_extends_pipeline() -> Result<CaseReport, TaskDriverError> {
    let store = FindingStore::new();
    let scope = scope_v7();
    let mut pipeline = four_check_pipeline();
    pipeline.add(TitlePresentCheck);
    let ctx = CheckCtx {
        scope: Some(&scope),
        store: &store,
    };
    let mut failures = Vec::new();
    if !pipeline.check_names().contains(&"title-present") {
        failures.push("operator check not registered in check_names".to_string());
    }
    let mut untitled = passing_finding("fp-142-a2a");
    untitled.title = String::new();
    match pipeline.validate(&untitled, &ctx) {
        Err((name, CheckResult::Fail { .. })) if name == "title-present" => {}
        other => failures.push(format!("operator check did not block: {other:?}")),
    }
    let titled = passing_finding("fp-142-a2b");
    if pipeline.validate(&titled, &ctx).is_err() {
        failures.push("operator check blocks a clean finding".to_string());
    }
    // Previously-failing findings still fail: adding a check is
    // monotone — it can only turn Ok into Err, never the reverse.
    let mut no_evidence = passing_finding("fp-142-a2c");
    no_evidence.evidence = empty_evidence();
    match pipeline.validate(&no_evidence, &ctx) {
        Err((name, _)) if name == "evidence-present" => {}
        other => failures.push(format!("extension weakened an old verdict: {other:?}")),
    }
    let mut evidence_lines = vec![
        format!("checks: {}", pipeline.check_names().join(", ")),
        "empty title -> title-present Fail; clean title -> Ok".to_string(),
        "old failure (evidence-present) still fails after extension".to_string(),
        verdict_line(
            "142",
            Verdict::Replicates,
            "extension runs and never weakens",
        ),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "checks": pipeline.check_names(),
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "all_checks_pass_reportable" => case_all_checks_pass_reportable(),
        "each_check_blocks_with_name" => case_each_check_blocks_with_name(),
        "validator_error_fails_closed" => case_validator_error_fails_closed(),
        "operator_check_extends_pipeline" => case_operator_check_extends_pipeline(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-142: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[1]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-142".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-142".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
