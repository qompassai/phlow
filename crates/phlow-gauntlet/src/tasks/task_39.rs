//! task-39: recursive payload bomb (rust).
//!
//! Recon probe: the design asks for structural payload bounds — a small
//! input that expands enormously (billion-laughs style entity
//! expansion, deeply nested arrays) must trip a DEPTH bound and an
//! EXPANSION-RATIO bound, both enforced with typed errors; the flat
//! size cap alone is documented as insufficient. The driver exercises
//! the REAL checked JSON boundary — `phlow_json::parse_limited`
//! (`crates/phlow-json`: input cap + depth bound + typed errors) —
//! with crafted bomb inputs. No mocks.
//!
//! Honest result: HALF the seam exists. The depth bound is real:
//! `JSON_DEPTH_MAX` (64) is enforced with an explicit stack (no
//! recursion over attacker-controlled depth) and reported as the typed
//! `JsonError::DepthExceeded`; deeper nesting trips `serde_json`'s own
//! recursion limit (128) first, also as a typed `JsonError::Parse` —
//! 10,000-deep nesting returns an error, never a stack overflow. The
//! flat input cap (`JSON_INPUT_BYTES_MAX`, 1 MiB) is enforced before
//! parsing with the typed `JsonError::InputTooLarge`.
//!
//! The expansion-RATIO bound does NOT exist: no ratio constant, no
//! ratio error variant (asserted by an exhaustive match over
//! `JsonError` — adding a ratio variant later breaks compilation,
//! fail-closed — and by a runtime scan of the embedded
//! `phlow-json/src/lib.rs` source for "ratio"). The mechanism nuance,
//! documented not hand-waved: phlow has NO recursive-expansion decoder
//! on untrusted input — no entity expansion, no archive extraction (no
//! zip/flate2/tar in the graph), no recursive template expansion — so
//! a JSON document cannot self-expand beyond O(input bytes) (measured:
//! parsed nodes ≤ input bytes on a large payload). The billion-laughs
//! *attack* has no seam; but the ratio *bound with a typed error* the
//! design's conjunctive pass criteria demand is absent, so the honest
//! task verdict is `fail` at `"seam"`.
//!
//! Four cases, all against the real parser (no mocks): two validation,
//! two adversarial. The task-level verdict is `fail` at `"seam"`
//! because the design's pass criteria require BOTH bounds with typed
//! errors, and the ratio bound is missing.
//!
//! Banked for Matt (product decision, NOT auto-implemented): whether a
//! node-count-vs-input-bytes ratio bound belongs in `parse_limited` as
//! defense in depth, given no expansion primitive exists in phlow's
//! parse paths.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_json::{JSON_DEPTH_MAX, JSON_INPUT_BYTES_MAX, JsonError, parse_limited};
use std::fmt;

/// Task id.
pub const ID: &str = "task-39";
/// Human-readable name.
pub const NAME: &str = "recursive payload bomb";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "nested_payload_parses",
    "deep_nesting_rejected_with_typed_error",
    "expansion_ratio_bound_absent",
    "flat_cap_is_the_only_total_bound",
];

/// `phlow-json/src/lib.rs`, embedded at compile time so the probe can
/// assert the absence of a ratio bound against the exact source it
/// drives. If the source moves, compilation fails — fail-closed.
const PHLOW_JSON_SRC: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../phlow-json/src/lib.rs"
));

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-39 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture could not be built.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
    /// A probe step failed.
    Probe {
        /// What was being probed.
        what: String,
        /// The underlying error.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixture { what, detail } => {
                write!(f, "task-39: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-39: cannot probe {what}: {detail}")
            }
        }
    }
}

impl std::error::Error for DriverError {}

fn fixture_error(what: &str, detail: impl fmt::Display) -> DriverError {
    DriverError::Fixture {
        what: what.to_string(),
        detail: detail.to_string(),
    }
}

fn probe_error(what: &str, detail: impl fmt::Display) -> DriverError {
    DriverError::Probe {
        what: what.to_string(),
        detail: detail.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Fixtures: crafted bomb inputs against the real parser
// ---------------------------------------------------------------------------

/// Build `depth`-deep nested arrays: `[[[ ... ]]]`.
fn nested_arrays(depth: usize) -> Result<String, DriverError> {
    if depth == 0 || depth > 100_000 {
        return Err(fixture_error(
            "nested payload",
            format!("depth {depth} outside (0, 100000]"),
        ));
    }
    Ok(format!("{}{}", "[".repeat(depth), "]".repeat(depth)))
}

/// Count every node in a parsed value with an explicit stack (no
/// recursion over attacker-controlled depth).
fn count_nodes(value: &serde_json::Value) -> usize {
    let mut count = 0usize;
    let mut stack = vec![value];
    while let Some(node) = stack.pop() {
        count += 1;
        match node {
            serde_json::Value::Array(items) => stack.extend(items.iter()),
            serde_json::Value::Object(map) => stack.extend(map.values()),
            _ => {}
        }
    }
    count
}

/// True when `err` is an expansion-ratio rejection. The match is
/// EXHAUSTIVE over `JsonError`: if phlow-json ever gains a ratio
/// variant, this stops compiling — fail-closed, the probe must be
/// revisited instead of silently passing.
fn is_ratio_rejection(err: &JsonError) -> bool {
    match err {
        JsonError::InputTooLarge { .. }
        | JsonError::InvalidUtf8 { .. }
        | JsonError::Parse { .. }
        | JsonError::DepthExceeded { .. }
        | JsonError::NotAnObject
        | JsonError::NotAnArray
        | JsonError::MissingField { .. }
        | JsonError::UnexpectedType { .. }
        | JsonError::NonFiniteNumber { .. } => false,
    }
}

// ---------------------------------------------------------------------------
// Case verdicts
// ---------------------------------------------------------------------------

/// The parsed verdict of one case.
#[derive(Debug, Clone)]
pub struct CaseReport {
    /// Which case ran.
    pub case: String,
    /// Whether the case's own assertions held.
    pub passed: bool,
    /// Measured numbers.
    pub metrics: serde_json::Value,
    /// Diagnostic lines from the case.
    pub evidence: Vec<String>,
    /// Failing assertion details, empty when `passed`.
    pub failures: Vec<String>,
}

impl CaseReport {
    fn pass(case: &'static str, metrics: serde_json::Value, evidence: Vec<String>) -> Self {
        Self {
            case: case.to_string(),
            passed: true,
            metrics,
            evidence,
            failures: Vec::new(),
        }
    }

    fn fail(case: &'static str, failure: String, evidence: Vec<String>) -> Self {
        Self {
            case: case.to_string(),
            passed: false,
            metrics: serde_json::Value::Null,
            evidence,
            failures: vec![failure],
        }
    }
}

/// V1: the design's default scenario — a normal nested payload parses
/// through the checked boundary.
fn case_nested_payload_parses() -> Result<CaseReport, DriverError> {
    const CASE: &str = "nested_payload_parses";
    let mut evidence = Vec::new();
    let input = r#"{"a":[{"b":[1,2,{"c":true}]}],"d":"tail"}"#;
    let value = parse_limited(input).map_err(|e| probe_error("normal payload", e))?;
    let depth_ok = value
        .get("a")
        .and_then(|a| a.get(0))
        .and_then(|o| o.get("b"))
        .is_some();
    if !depth_ok {
        return Ok(CaseReport::fail(
            CASE,
            "parsed value lost its nested shape".to_string(),
            evidence,
        ));
    }
    evidence.push(format!(
        "normal nested payload ({} bytes) parses; nested shape intact",
        input.len()
    ));
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"input_bytes": input.len(), "parsed": true}),
        evidence,
    ))
}

/// V2: deeply nested arrays are rejected with TYPED errors, never a
/// stack overflow. 100-deep (past JSON_DEPTH_MAX=64, under serde_json's
/// own 128) trips the crate's explicit bound; 10,000-deep trips the
/// parser's recursion limit first — both typed, and the probe returning
/// at all proves no stack overflow.
fn case_deep_nesting_rejected() -> Result<CaseReport, DriverError> {
    const CASE: &str = "deep_nesting_rejected_with_typed_error";
    let mut evidence = Vec::new();
    let over_explicit = nested_arrays(JSON_DEPTH_MAX + 36)?;
    match parse_limited(&over_explicit) {
        Err(JsonError::DepthExceeded { depth, max }) => {
            evidence.push(format!(
                "100-deep nesting -> JsonError::DepthExceeded {{ depth: {depth}, max: {max} }}: the explicit named bound fires with a typed error"
            ));
            if max != JSON_DEPTH_MAX {
                return Ok(CaseReport::fail(
                    CASE,
                    format!("DepthExceeded max {max} != JSON_DEPTH_MAX {JSON_DEPTH_MAX}"),
                    evidence,
                ));
            }
        }
        Err(other) => {
            return Ok(CaseReport::fail(
                CASE,
                format!("100-deep nesting gave '{other}', want DepthExceeded"),
                evidence,
            ));
        }
        Ok(_) => {
            return Ok(CaseReport::fail(
                CASE,
                format!(
                    "{}-deep nesting parsed: the explicit depth bound did not fire",
                    JSON_DEPTH_MAX + 36
                ),
                evidence,
            ));
        }
    }
    let bomb = nested_arrays(10_000)?;
    let started = std::time::Instant::now();
    match parse_limited(&bomb) {
        Err(err) => {
            evidence.push(format!(
                "10,000-deep nesting -> typed error '{err}' in {:?} (no stack overflow: the probe returned)",
                started.elapsed()
            ));
            if is_ratio_rejection(&err) {
                return Ok(CaseReport::fail(
                    CASE,
                    "10,000-deep nesting was rejected as a ratio violation, not a depth violation"
                        .to_string(),
                    evidence,
                ));
            }
        }
        Ok(_) => {
            return Ok(CaseReport::fail(
                CASE,
                "10,000-deep nesting parsed: no depth bound fired".to_string(),
                evidence,
            ));
        }
    }
    evidence.push(
        "depth is bounded twice: serde_json's own recursion limit (128) at parse time, then JSON_DEPTH_MAX (64) with an explicit stack — no recursion ever runs over attacker-controlled depth"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({
            "explicit_bound": JSON_DEPTH_MAX,
            "deep_input_depth": 10_000,
            "rejected_typed": true,
        }),
        evidence,
    ))
}

/// A1: the expansion-RATIO bound is absent. The probe measures that a
/// large flat payload cannot self-amplify (parsed nodes ≤ input bytes:
/// JSON has no expansion primitive) AND asserts the negative two ways:
/// the embedded phlow-json source contains no "ratio", and an
/// exhaustive match over `JsonError` shows no ratio variant. The case
/// passes as a probe documenting the absence; the absence itself is
/// what fails the task.
fn case_expansion_ratio_bound_absent() -> Result<CaseReport, DriverError> {
    const CASE: &str = "expansion_ratio_bound_absent";
    let mut evidence = Vec::new();
    // A large flat payload: 100,000-element array (~488 KiB, under the
    // 1 MiB flat cap so the ratio dimension is isolated). The elements
    // are joined with commas — no trailing comma, valid JSON.
    let payload = format!(
        "[{}]",
        (0..100_000).map(|_| "0").collect::<Vec<_>>().join(",")
    );
    let input_bytes = payload.len();
    let value = parse_limited(&payload).map_err(|e| probe_error("flat payload", e))?;
    let nodes = count_nodes(&value);
    #[allow(clippy::cast_precision_loss)]
    let ratio = nodes as f64 / input_bytes as f64;
    evidence.push(format!(
        "flat payload: {input_bytes} input bytes -> {nodes} parsed nodes (ratio {ratio:.3} nodes/byte): no self-amplification — JSON cannot expand beyond O(input bytes)"
    ));
    if nodes > input_bytes {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "node count {nodes} exceeds input bytes {input_bytes}: unexpected amplification"
            ),
            evidence,
        ));
    }
    // The negative, asserted twice against the exact source under test.
    if PHLOW_JSON_SRC.to_lowercase().contains("ratio") {
        return Ok(CaseReport::fail(
            CASE,
            "phlow-json source mentions 'ratio': a ratio bound may exist; probe premise changed"
                .to_string(),
            evidence,
        ));
    }
    evidence.push(
        "embedded phlow-json/src/lib.rs contains no 'ratio': no ratio constant and no ratio error variant exist".to_string(),
    );
    // Exhaustive match (is_ratio_rejection) doubles as the compile-time
    // half: a future ratio variant breaks the build instead of passing
    // silently.
    evidence.push(
        "no recursive-expansion decoder exists in phlow's parse paths (source scans: no entity expansion, no archive extraction — no zip/flate2/tar in the graph, no recursive template expansion of untrusted input) — the billion-laughs attack has no seam"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({
            "input_bytes": input_bytes,
            "parsed_nodes": nodes,
            "nodes_per_byte": ratio,
            "ratio_bound_in_source": false,
        }),
        evidence,
    ))
}

/// A2: the flat size cap is the ONLY total bound — and it is enforced
/// with a typed error before any parsing. The design deems the flat
/// cap insufficient for expansion attacks; the probe documents that it
/// is the only total bound present, which is exactly the gap.
fn case_flat_cap_is_the_only_total_bound() -> Result<CaseReport, DriverError> {
    const CASE: &str = "flat_cap_is_the_only_total_bound";
    let mut evidence = Vec::new();
    let oversize = " ".repeat(JSON_INPUT_BYTES_MAX + 1);
    match parse_limited(&oversize) {
        Err(JsonError::InputTooLarge { bytes, max }) => {
            evidence.push(format!(
                "input of {bytes} bytes -> JsonError::InputTooLarge {{ max: {max} }}: rejected before parsing, typed"
            ));
        }
        Err(other) => {
            return Ok(CaseReport::fail(
                CASE,
                format!("oversize input gave '{other}', want InputTooLarge"),
                evidence,
            ));
        }
        Ok(_) => {
            return Ok(CaseReport::fail(
                CASE,
                "oversize input parsed: the flat cap did not fire".to_string(),
                evidence,
            ));
        }
    }
    evidence.push(
        "the flat cap (1 MiB) is the only total-size bound; with no expansion primitive in phlow's parse paths it bounds total memory — but the design's required expansion-ratio bound with its own typed error is still absent"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({
            "input_bytes_max": JSON_INPUT_BYTES_MAX,
            "oversize_bytes": JSON_INPUT_BYTES_MAX + 1,
            "rejected_typed": true,
        }),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "nested_payload_parses" => case_nested_payload_parses(),
        "deep_nesting_rejected_with_typed_error" => case_deep_nesting_rejected(),
        "expansion_ratio_bound_absent" => case_expansion_ratio_bound_absent(),
        "flat_cap_is_the_only_total_bound" => case_flat_cap_is_the_only_total_bound(),
        _ => Err(DriverError::Fixture {
            what: "case".to_string(),
            detail: format!("unknown case '{case}'"),
        }),
    }
}

// ---------------------------------------------------------------------------
// Task entry point
// ---------------------------------------------------------------------------

struct TaskFailure {
    where_: String,
    how: String,
    evidence: Vec<String>,
}

fn run_inner(_ctx: &Ctx) -> Result<Vec<String>, TaskFailure> {
    let mut evidence = vec![
        "recon: depth bound is real — JSON_DEPTH_MAX (64) enforced with an explicit stack, typed JsonError::DepthExceeded; serde_json's own recursion limit (128) fires first at parse time".to_string(),
        "recon: flat input cap JSON_INPUT_BYTES_MAX (1 MiB) enforced before parsing, typed JsonError::InputTooLarge".to_string(),
        "recon: no expansion-ratio bound — no ratio constant, no ratio JsonError variant (exhaustive match + embedded-source scan agree)".to_string(),
        "recon: no recursive-expansion decoder in phlow's parse paths — no entity expansion, no archive extraction, no recursive template expansion of untrusted input".to_string(),
    ];
    for case in CASES {
        let report = run_case(case).map_err(|e| TaskFailure {
            where_: case.to_string(),
            how: e.to_string(),
            evidence: evidence.clone(),
        })?;
        evidence.push(format!("case {case}: passed={}", report.passed));
        evidence.push(format!("case {case} metrics: {}", report.metrics));
        for line in &report.evidence {
            evidence.push(format!("case {case}: {line}"));
        }
        if !report.passed {
            return Err(TaskFailure {
                where_: case.to_string(),
                how: report.failures.join("; "),
                evidence,
            });
        }
    }
    evidence.push(
        "finding: 10,000-deep nesting returns a typed error, never a stack overflow; 100-deep trips the explicit JsonError::DepthExceeded".to_string(),
    );
    evidence.push(
        "finding: a large flat payload shows no self-amplification (parsed nodes <= input bytes) — JSON cannot expand beyond O(input)".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: the design's pass criteria demand BOTH a depth bound and an expansion-ratio bound with typed errors — the depth half exists (JSON_DEPTH_MAX=64, typed DepthExceeded; serde_json's own 128 recursion limit at parse time) but no expansion-ratio bound exists anywhere (no constant, no JsonError variant, verified by exhaustive match and embedded-source scan). The billion-laughs attack itself has no seam (no recursive-expansion decoder in phlow's parse paths), so the missing bound is a defense-in-depth product decision, not a demonstrated vulnerability — but the conjunctive criteria are unmet.".to_string(),
        evidence,
    })
}

/// Attempt the task.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    match run_inner(ctx) {
        Ok(evidence) => TaskOutcome::Pass {
            evidence: bound_evidence(evidence),
        },
        Err(failure) => TaskOutcome::Fail {
            where_: failure.where_,
            how: failure.how,
            evidence: bound_evidence(failure.evidence),
        },
    }
}
