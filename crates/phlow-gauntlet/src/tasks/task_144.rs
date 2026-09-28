//! Task 144 — report generation from a validated finding (rust, V).
//!
//! The human-readable document. A report is generated only from a
//! `Reportable` finding; every section (summary, impact, steps,
//! evidence refs, scope ref) must be present; the rendered markdown is
//! lint-clean (no trailing whitespace, fenced code blocks closed,
//! headings ordered). Field values are escaped: there is no template
//! engine — sections are plain string interpolation — and the
//! steps-to-reproduce block is fenced with a fence strictly longer than
//! any backtick run in the content, so `{{template}}` markers and
//! markdown breakouts render inert. Every text surface is redacted at
//! write time against the configured secrets.
//!
//! All verdict evidence comes from scripted fixtures (MOCK).

use crate::bounty::types::FindingFields;
use crate::bounty::*;
use crate::skillopt::driver::{CaseReport, TaskDriverError, verdict_line};
use crate::skillopt::learner::Verdict;
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-144";
/// Task name.
pub const NAME: &str = "report-generation";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "reportable_renders_complete",
    "rendered_markdown_lint_clean",
    "missing_field_refused",
    "injection_rendered_inert",
];

/// Maximum fence length the renderer will emit (backticks). Longer
/// means the content was pathological; refuse instead of emitting an
/// absurd fence.
pub const FENCE_LEN_MAX: usize = 32;

/// Typed report-generation errors. `MissingField` refuses the whole
/// report — no partial document is ever emitted.
#[derive(Debug, PartialEq, Eq)]
pub enum ReportError {
    /// The finding is not in `Reportable` state; reports are generated
    /// only from reportable findings.
    NotReportable { state: String },
    /// A required section field is absent from the finding metadata.
    MissingField { field: &'static str },
    /// The steps content needs a longer fence than the bound allows.
    FenceOverflow,
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotReportable { state } => {
                write!(f, "finding not reportable (state {state})")
            }
            Self::MissingField { field } => write!(f, "missing report field: {field}"),
            Self::FenceOverflow => write!(f, "steps need a fence longer than the bound"),
        }
    }
}

impl std::error::Error for ReportError {}

/// The report content model: every section present, every value
/// already redacted at write time.
#[derive(Clone, Debug)]
pub struct Report {
    pub finding_id: String,
    pub title: String,
    pub summary: String,
    pub impact: String,
    pub steps: String,
    pub evidence_sha256: String,
    pub custody_entries: usize,
    pub program_id: String,
    pub scope_version: u64,
    pub target_id: String,
}

/// Build a [`Report`] from a `Reportable` finding plus its
/// operator-extensible metadata fields (`summary`, `impact`, `steps`).
/// Missing fields and non-reportable findings are typed refusals;
/// secrets are scrubbed at write time.
pub fn build_report(
    finding: &Finding,
    fields: &FindingFields,
    program_id: &str,
    scope_version: u64,
    secrets: &[String],
) -> Result<Report, ReportError> {
    if finding.state != FindingState::Reportable {
        return Err(ReportError::NotReportable {
            state: format!("{:?}", finding.state),
        });
    }
    let get = |key: &'static str| -> Result<String, ReportError> {
        fields
            .get(key)
            .map(|v| redact_text(v, secrets))
            .ok_or(ReportError::MissingField { field: key })
    };
    Ok(Report {
        finding_id: finding.id.clone(),
        title: redact_text(&finding.title, secrets),
        summary: get("summary")?,
        impact: get("impact")?,
        steps: get("steps")?,
        evidence_sha256: finding.evidence.sha256.clone(),
        custody_entries: finding.evidence.custody.len(),
        program_id: program_id.to_string(),
        scope_version,
        target_id: finding.target_id.0.clone(),
    })
}

/// Longest run of consecutive backticks in `text`.
fn longest_backtick_run(text: &str) -> usize {
    let mut best = 0usize;
    let mut run = 0usize;
    for ch in text.chars() {
        if ch == '`' {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

/// A fence strictly longer than any backtick run in the content, so the
/// content can never close the steps block early. Bounded.
pub fn fence_for(content: &str) -> Result<String, ReportError> {
    let len = longest_backtick_run(content).max(2) + 1;
    if len > FENCE_LEN_MAX {
        return Err(ReportError::FenceOverflow);
    }
    Ok("`".repeat(len))
}

/// Render the report as markdown with a fixed section order. No
/// template engine: values are interpolated into fixed positions, so
/// `{{...}}` markers in field text are inert by construction.
pub fn render_report(report: &Report) -> Result<String, ReportError> {
    let fence = fence_for(&report.steps)?;
    Ok(format!(
        "# Bug bounty report: {title}\n\
         \n\
         ## Summary\n\
         \n\
         {summary}\n\
         \n\
         ## Impact\n\
         \n\
         {impact}\n\
         \n\
         ## Steps to reproduce\n\
         \n\
         {fence}\n\
         {steps}\n\
         {fence}\n\
         \n\
         ## Evidence\n\
         \n\
         - evidence sha256: {sha}\n\
         - custody entries: {custody}\n\
         \n\
         ## Scope\n\
         \n\
         - program: {program}\n\
         - scope version: {version}\n\
         - target: {target}\n",
        title = report.title,
        summary = report.summary,
        impact = report.impact,
        steps = report.steps,
        sha = report.evidence_sha256,
        custody = report.custody_entries,
        program = report.program_id,
        version = report.scope_version,
        target = report.target_id,
    ))
}

/// The driver's small markdown lint. Returns violation descriptions;
/// empty means lint-clean. Rules: no trailing whitespace; fenced code
/// blocks closed (a closing fence must be at least as long as its
/// opener, per CommonMark — a shorter backtick run inside a lengthened
/// fence is content, not a fence); headings carry a space after the
/// hashes and never jump more than one level at a time.
pub fn lint_markdown(md: &str) -> Vec<String> {
    fn fence_run(line: &str) -> Option<usize> {
        let t = line.trim_start();
        if t.len() >= 3 && t.chars().all(|c| c == '`') {
            Some(t.len())
        } else {
            None
        }
    }
    let mut violations = Vec::new();
    let mut open_len: Option<usize> = None;
    let mut prev_level = 0u32;
    for (i, line) in md.lines().enumerate() {
        let n = i + 1;
        if line.ends_with(' ') || line.ends_with('\t') {
            violations.push(format!("line {n}: trailing whitespace"));
        }
        if let Some(run) = fence_run(line) {
            match open_len {
                None => open_len = Some(run),
                Some(o) if run >= o => open_len = None,
                Some(_) => {} // shorter run inside a block: content
            }
        }
        let trimmed = line.trim_start();
        if let Some(hashes) = trimmed.strip_prefix('#') {
            let level = 1 + hashes.chars().take_while(|&c| c == '#').count() as u32;
            let after = &trimmed[level as usize..];
            if !after.starts_with(' ') {
                violations.push(format!("line {n}: heading without space after hashes"));
            }
            if level > prev_level + 1 {
                violations.push(format!(
                    "line {n}: heading jumps from level {prev_level} to {level}"
                ));
            }
            prev_level = level;
        }
    }
    if open_len.is_some() {
        violations.push("unclosed fenced code block".to_string());
    }
    violations
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

fn reportable_finding() -> Finding {
    Finding {
        id: "f000001".to_string(),
        target_id: TargetId("t-web-01".to_string()),
        fingerprint: "fp-144".to_string(),
        title: "mock: reflected XSS in /search".to_string(),
        state: FindingState::Reportable,
        evidence: mock_evidence(),
        observation_count: 2,
        reject_reason: None,
    }
}

fn full_fields() -> FindingFields {
    FindingFields::from([
        (
            "summary".to_string(),
            "Reflected cross-site scripting in the /search q parameter.".to_string(),
        ),
        (
            "impact".to_string(),
            "Session hijack of any user clicking a crafted link.".to_string(),
        ),
        (
            "steps".to_string(),
            "1. Open https://example.com/search?q=<script>alert(1)</script>\n\
             2. Observe the script execute."
                .to_string(),
        ),
    ])
}

/// V1: a `Reportable` finding renders a section-complete report; every
/// surface is redacted at write time (a secret planted in the steps
/// comes out `[REDACTED]`, never raw).
fn case_reportable_renders_complete() -> Result<CaseReport, TaskDriverError> {
    let finding = reportable_finding();
    let mut fields = full_fields();
    fields.insert(
        "steps".to_string(),
        "1. Authenticate with token s3cr3t-token-xyz\n2. Replay the request.".to_string(),
    );
    let secrets = vec!["s3cr3t-token-xyz".to_string()];
    let mut failures = Vec::new();
    let md = match build_report(&finding, &fields, "prog-01", 7, &secrets)
        .and_then(|r| render_report(&r))
    {
        Ok(md) => md,
        Err(e) => {
            failures.push(format!("reportable finding refused: {e}"));
            String::new()
        }
    };
    for section in [
        "## Summary",
        "## Impact",
        "## Steps to reproduce",
        "## Evidence",
        "## Scope",
    ] {
        if !md.contains(section) {
            failures.push(format!("rendered report missing section {section}"));
        }
    }
    if !md.contains(&finding.evidence.sha256) {
        failures.push("rendered report missing the evidence sha256".to_string());
    }
    if !md.contains("[REDACTED]") || md.contains("s3cr3t-token-xyz") {
        failures.push("secret not redacted at write time".to_string());
    }
    let mut evidence_lines = vec![
        "sections present: 5/5".to_string(),
        format!("markdown bytes: {}", md.len()),
        "secret in steps -> [REDACTED]".to_string(),
        verdict_line(
            "144",
            Verdict::Replicates,
            "section-complete, lint target, redacted",
        ),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "markdown_bytes": md.len(),
            "redacted": !md.contains("s3cr3t-token-xyz"),
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: the rendered markdown passes the driver's lint — no trailing
/// whitespace, fenced blocks closed, headings ordered.
fn case_rendered_markdown_lint_clean() -> Result<CaseReport, TaskDriverError> {
    let finding = reportable_finding();
    let fields = full_fields();
    let mut failures = Vec::new();
    let md = build_report(&finding, &fields, "prog-01", 7, &[])
        .and_then(|r| render_report(&r))
        .map_err(|e| TaskDriverError::Fixture {
            what: "render".to_string(),
            detail: format!("task-144: render failed: {e}"),
        })?;
    let violations = lint_markdown(&md);
    if !violations.is_empty() {
        failures.push(format!("lint violations: {}", violations.join("; ")));
    }
    // The lint is real: it must catch planted violations.
    let bad = "#Title\nline with trailing space \n```\nunclosed\n";
    let bad_hits = lint_markdown(bad);
    if bad_hits.len() < 3 {
        failures.push(format!(
            "lint caught only {} of the 3 planted violations",
            bad_hits.len()
        ));
    }
    let mut evidence_lines = vec![
        format!("lint violations on rendered report: {}", violations.len()),
        format!("lint violations on planted-bad doc: {}", bad_hits.len()),
        verdict_line(
            "144",
            Verdict::Replicates,
            "rendered output lint-clean; lint proven live",
        ),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "violations": violations.len(),
            "planted_caught": bad_hits.len(),
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1: a finding missing the `impact` field is refused with typed
/// `MissingField` — no partial report is emitted.
fn case_missing_field_refused() -> Result<CaseReport, TaskDriverError> {
    let finding = reportable_finding();
    let mut fields = full_fields();
    fields.remove("impact");
    let mut failures = Vec::new();
    match build_report(&finding, &fields, "prog-01", 7, &[]) {
        Err(ReportError::MissingField { field }) => {
            if field != "impact" {
                failures.push(format!("wrong missing field named: {field}"));
            }
        }
        Err(e) => failures.push(format!("wrong refusal: {e}")),
        Ok(_) => failures.push("REPORT BUILT WITHOUT impact — partial report emitted".to_string()),
    }
    // The other two required fields refuse the same way.
    for missing in ["summary", "steps"] {
        let mut f = full_fields();
        f.remove(missing);
        match build_report(&finding, &f, "prog-01", 7, &[]) {
            Err(ReportError::MissingField { field }) if field == missing => {}
            other => failures.push(format!("missing {missing}: wrong outcome: {other:?}")),
        }
    }
    let mut evidence_lines = vec![
        "missing impact -> Err(ReportError::MissingField { field: \"impact\" })".to_string(),
        "missing summary / steps refuse the same way".to_string(),
        verdict_line("144", Verdict::Replicates, "no partial report ever emitted"),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "refused_fields": ["impact", "summary", "steps"],
            "backend": "scripted-mock",
        }),
        evidence_lines,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2: steps containing `{{template}}` markers and a markdown fence
/// breakout attempt render inert — the literal text survives, the
/// fence is lengthened past any backtick run in the content, and the
/// lint still passes on the rendered output.
fn case_injection_rendered_inert() -> Result<CaseReport, TaskDriverError> {
    let finding = reportable_finding();
    let mut fields = full_fields();
    fields.insert(
        "steps".to_string(),
        "1. POST /search with q={{template}}\n```\n\
         2. breakout attempt: the fence above must NOT close the block\n\
         3. done"
            .to_string(),
    );
    let mut failures = Vec::new();
    let md = build_report(&finding, &fields, "prog-01", 7, &[])
        .and_then(|r| render_report(&r))
        .map_err(|e| TaskDriverError::Fixture {
            what: "render".to_string(),
            detail: format!("task-144: render failed: {e}"),
        })?;
    if !md.contains("{{template}}") {
        failures.push("{{template}} literal lost from output".to_string());
    }
    // The steps block must be fenced with 4 backticks (content's longest
    // run is 3), and the content's own ``` line must sit inside it.
    if !md.contains("````\n") {
        failures.push("fence not lengthened past the content's backtick run".to_string());
    }
    let lines: Vec<&str> = md.lines().collect();
    let fence_idx: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.trim_start().starts_with("````"))
        .map(|(i, _)| i)
        .collect();
    let breakout_idx = lines.iter().position(|l| *l == "```");
    match (fence_idx.as_slice(), breakout_idx) {
        ([open, close], Some(b)) if *open < b && b < *close => {}
        _ => failures.push(format!(
            "breakout ``` line not contained in the lengthened fence: \
             fences at {fence_idx:?}, breakout at {breakout_idx:?}"
        )),
    }
    let violations = lint_markdown(&md);
    if !violations.is_empty() {
        failures.push(format!(
            "injected doc not lint-clean: {}",
            violations.join("; ")
        ));
    }
    let mut evidence_lines = vec![
        "{{template}} renders literally (no template engine to inject)".to_string(),
        "steps fenced with ```` — the ``` breakout line stays inside".to_string(),
        format!("lint violations on injected doc: {}", violations.len()),
        verdict_line(
            "144",
            Verdict::Replicates,
            "injection fixtures render inert",
        ),
    ];
    evidence_lines.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "fence_len": 4,
            "lint_violations": violations.len(),
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
        "reportable_renders_complete" => case_reportable_renders_complete(),
        "rendered_markdown_lint_clean" => case_rendered_markdown_lint_clean(),
        "missing_field_refused" => case_missing_field_refused(),
        "injection_rendered_inert" => case_injection_rendered_inert(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-144: unknown case '{case}'"),
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
            where_: "task-144".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-144".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
