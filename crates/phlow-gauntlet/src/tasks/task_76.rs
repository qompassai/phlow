//! task-76: tokenizer boundary mismatch (rust).
//!
//! The design asks for the tokenizer binding behind token counting /
//! truncation for budgets and context windows: budget and truncation
//! computed with tokenizer X while the model tokenizes with Y must be
//! measured and surfaced (never hidden), truncation must be
//! re-encoded through the serving tokenizer so the model never
//! receives a split token, and the ledger must never present X-counts
//! as Y-counts.
//!
//! Seam recon (verified, not invented):
//! - No tokenizer binding exists in any phlow crate. An exact-token
//!   source scan for `tokenizer` over every product crate's
//!   `src/**/*.rs` (the gauntlet crate excluded — its own probes use
//!   the design vocabulary) finds zero hits, and no `Cargo.toml` in
//!   the workspace depends on `tiktoken`, `tokenizers`,
//!   `sentencepiece`, or `hf-hub`.
//! - The runtime's context ruler is CHARACTERS: `role_turn`
//!   (`crates/phlow-runtime/src/runtime.rs`) ends the role with
//!   "Context budget exhausted" when `must_dumps(messages).chars()
//!   .count() > budgets.max_context_chars`. Truncation of retrieved
//!   context is char-boundary (`truncate_chars` in
//!   `crates/phlow-agent/src/memory.rs`).
//! - The only token-unit number phlow produces is
//!   `phlow_llm::max_tokens_for(context_length) = min(8192,
//!   context_length / 2)`, sent to Ollama as the `max_tokens` field —
//!   an operator-configured number with no tokenizer behind it,
//!   presented in token units.
//! - The serving model's tokenizer lives inside Ollama and is
//!   invisible to phlow. There is no tokenizer X, so there is nothing
//!   to re-encode truncation through and no X/Y divergence to warn
//!   about.
//!
//! Four cases: two validation, two adversarial. The task-level verdict
//! is `fail` at `"seam"`.
//!
//! Banked for Matt (product decision, NOT auto-implemented on gauntlet
//! authority): whether phlow should gain a tokenizer binding —
//! counting budgets with the serving model's tokenizer, re-encoding
//! truncation through it, and warning on X/Y divergence. That is a new
//! product feature, not a bug fix.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use serde_json::Value;
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-76";
/// Human-readable name.
pub const NAME: &str = "tokenizer boundary mismatch";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "char_budget_is_the_ruler",
    "mock_tokenizers_diverge_and_surface",
    "char_truncation_splits_model_tokens",
    "config_number_presented_as_tokens",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-76 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture was unusable.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
    /// The source probe itself failed.
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
                write!(f, "task-76: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-76: probe {what} failed: {detail}")
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
            metrics: serde_json::json!({}),
            evidence,
            failures: vec![failure],
        }
    }
}

// ---------------------------------------------------------------------------
// Mock tokenizers (design: "two tokenizers with known divergent
// behavior on a small committed fixture corpus; no network")
// ---------------------------------------------------------------------------

/// Mock tokenizer X: cl100k-style — splits on whitespace, separates
/// punctuation into its own tokens.
fn tokenize_x(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for word in text.split_whitespace() {
        let mut buf = String::new();
        for ch in word.chars() {
            if ch.is_alphanumeric() {
                buf.push(ch);
            } else {
                if !buf.is_empty() {
                    tokens.push(std::mem::take(&mut buf));
                }
                tokens.push(ch.to_string());
            }
        }
        if !buf.is_empty() {
            tokens.push(buf);
        }
    }
    tokens
}

/// Mock tokenizer Y: SentencePiece-style — greedy fixed-width char
/// chunks, no word awareness.
fn tokenize_y(text: &str) -> Vec<String> {
    const Y_TOKEN_CHARS: usize = 4;
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(Y_TOKEN_CHARS)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

/// Re-encode through Y: the design's remedy — the truncated text is
/// encoded with the serving tokenizer and decoded back, so the model
/// receives exactly Y's canonical token sequence.
fn reencode_y(text: &str) -> String {
    tokenize_y(text).concat()
}

/// The committed fixture corpus: (text, x_count, y_count).
/// Counts are asserted below, so a drift in either mock fails loudly.
const CORPUS: [(&str, usize, usize); 4] = [
    ("hello world", 2, 3),
    ("Supercalifragilistic", 1, 5),
    ("a b c", 3, 2),
    ("", 0, 0),
];

fn check_corpus(evidence: &mut Vec<String>) -> Result<(), String> {
    for (text, want_x, want_y) in CORPUS {
        let got_x = tokenize_x(text).len();
        let got_y = tokenize_y(text).len();
        if got_x != want_x || got_y != want_y {
            return Err(format!(
                "mock drift on {text:?}: X {got_x} (want {want_x}), Y {got_y} (want {want_y})"
            ));
        }
        evidence.push(format!(
            "corpus {text:?}: X={got_x} tokens, Y={got_y} tokens{}",
            if got_x == got_y {
                " (agree)"
            } else {
                " (DIVERGE)"
            }
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Source probe: locate the tokenizer binding (task_48 scan pattern)
// ---------------------------------------------------------------------------

/// Maximum source files the tokenizer probe may read.
const SOURCE_FILES_MAX: usize = 4000;
/// Maximum bytes per source file the probe reads.
const SOURCE_BYTES_MAX: usize = 512 * 1024;

/// Workspace root: two levels above this crate's manifest directory.
fn workspace_root() -> Result<PathBuf, DriverError> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| fixture_error("workspace root", "manifest dir has no grandparent"))?;
    if !root.join("Cargo.lock").is_file() {
        return Err(fixture_error(
            "workspace root",
            format!("no Cargo.lock under {}", root.display()),
        ));
    }
    Ok(root.to_path_buf())
}

/// The gauntlet's own crate root, excluded from the product scan: the
/// gauntlet's drivers are test probes, not product code, and this
/// wave's own files (task_76.rs's doc comments, task_77.rs's design
/// notes) use the design vocabulary. Scanning the harness would
/// self-match; the probe covers product crates only.
fn excluded_crate_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Exact-token (case-insensitive) hits for `token` over every product
/// crate's `src/**/*.rs` — the gauntlet crate itself excluded (see
/// [`excluded_crate_root`]). Bounded like task_48.
fn scan_sources(root: &Path, token: &str) -> Result<Vec<String>, DriverError> {
    let excluded = excluded_crate_root();
    let wanted = token.to_lowercase();
    let crates_dir = root.join("crates");
    let mut hits = Vec::new();
    let mut files_seen = 0usize;
    let mut stack = vec![crates_dir];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| fixture_error("source walk", format!("{}: {e}", dir.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| fixture_error("source walk", e))?;
            let path = entry.path();
            if path.is_dir() {
                if path != excluded {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.components().any(|c| c.as_os_str() == "src")
            {
                files_seen += 1;
                if files_seen > SOURCE_FILES_MAX {
                    return Err(probe_error(
                        "source scan",
                        format!("file budget {SOURCE_FILES_MAX} exhausted"),
                    ));
                }
                let bytes = std::fs::read(&path).map_err(|e| {
                    fixture_error("source read", format!("{}: {e}", path.display()))
                })?;
                if bytes.len() > SOURCE_BYTES_MAX {
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes);
                for (lineno, line) in text.lines().enumerate() {
                    let found = line
                        .split(|c: char| !c.is_alphanumeric())
                        .any(|tok| tok.eq_ignore_ascii_case(&wanted));
                    if found {
                        hits.push(format!("{}:{}", path.display(), lineno + 1));
                    }
                }
            }
        }
    }
    Ok(hits)
}

/// V1: the runtime's budget ruler is characters, not tokens. The case
/// replicates the exact budget expression from `role_turn`
/// (`crates/phlow-runtime/src/runtime.rs`): the message JSON dump's
/// `.chars().count()` against `max_context_chars`. It also checks the
/// design's default scenario — the same tokenizer on both sides counts
/// the corpus identically.
fn case_char_budget_is_the_ruler() -> Result<CaseReport, DriverError> {
    const CASE: &str = "char_budget_is_the_ruler";
    let mut evidence = Vec::new();
    check_corpus(&mut evidence).map_err(|e| fixture_error("corpus", e))?;
    // The budget expression from role_turn, replicated on ASCII
    // fixtures (must_dumps is ensure_ascii — identical on ASCII).
    let messages = serde_json::json!([
        {"role": "user", "content": "hello world"},
        {"role": "assistant", "content": "Supercalifragilistic"},
    ]);
    let dumped = serde_json::to_string(&messages).expect("fixture is finite JSON");
    let char_count = dumped.chars().count();
    evidence.push(format!(
        "budget expression must_dumps(messages).chars().count() = {char_count} chars \
         (runtime.rs role_turn compares this against max_context_chars)"
    ));
    // The same text counted by the mock tokenizers is NOT the char
    // count: chars are not tokens.
    let text: String = ["hello world", "Supercalifragilistic"].join(" ");
    let x = tokenize_x(&text).len();
    let y = tokenize_y(&text).len();
    evidence.push(format!(
        "same text: {char_count} chars (dump) vs X={x} vs Y={y} tokens — \
         the char ruler and any token ruler disagree"
    ));
    // Design default: same tokenizer both sides — the count side and
    // the serve side both run X, so counts match exactly.
    let count_side: Vec<usize> = CORPUS
        .iter()
        .map(|(text, _, _)| tokenize_x(text).len())
        .collect();
    let serve_side: Vec<usize> = CORPUS
        .iter()
        .map(|(text, _, _)| tokenize_x(text).len())
        .collect();
    if count_side != serve_side {
        return Ok(CaseReport::fail(
            CASE,
            "identical tokenizers disagreed: mock broken".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "X-vs-X on the corpus: counts match exactly (the design's default \
         scenario holds for the mocks; the runtime itself has no tokenizer)"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"char_count": char_count, "x_tokens": x, "y_tokens": y}),
        evidence,
    ))
}

/// V2: the design's mismatch scenario — X and Y diverge on the corpus,
/// and the divergence is MEASURED and SURFACED (both numbers recorded,
/// the X number flagged as an estimate), not hidden. The real-code
/// anchor: phlow already runs two rulers — `max_context_chars`
/// (chars) and `max_tokens_for(context_length)` (token units) —
/// neither of which is the serving model's tokenizer.
fn case_mock_tokenizers_diverge_and_surface() -> Result<CaseReport, DriverError> {
    const CASE: &str = "mock_tokenizers_diverge_and_surface";
    let mut evidence = Vec::new();
    check_corpus(&mut evidence).map_err(|e| fixture_error("corpus", e))?;
    let mut divergent = 0usize;
    for (text, _, _) in CORPUS {
        let x = tokenize_x(text).len();
        let y = tokenize_y(text).len();
        let is_divergent = x != y;
        if is_divergent {
            divergent += 1;
        }
        // The mock ledger: both numbers, and the X number flagged as an
        // estimate — exactly what the design demands.
        let entry = serde_json::json!({
            "text": text,
            "x_count": x,
            "y_count": y,
            "divergent": is_divergent,
            "x_is_estimate": true,
            "note": "x_count counted with tokenizer X; provider bills in Y",
        });
        evidence.push(format!("ledger entry: {entry}"));
    }
    if divergent == 0 {
        return Ok(CaseReport::fail(
            CASE,
            "mock tokenizers never diverged: fixture broken".to_string(),
            evidence,
        ));
    }
    evidence.push(format!(
        "divergence measured on {divergent}/{} fixtures and surfaced in the ledger \
         (both counts + estimate flag); phlow's real rulers are max_context_chars \
         (chars) and max_tokens_for (token units) — neither is the model's tokenizer",
        CORPUS.len()
    ));
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"fixtures": CORPUS.len(), "divergent": divergent}),
        evidence,
    ))
}

/// A1: truncation at a char boundary (what phlow's `truncate_chars`
/// does) lands mid-token in Y. The design's remedy — truncate, then
/// re-encode through the serving tokenizer — has nothing to re-encode
/// through in phlow: no tokenizer exists.
fn case_char_truncation_splits_model_tokens() -> Result<CaseReport, DriverError> {
    const CASE: &str = "char_truncation_splits_model_tokens";
    let mut evidence = Vec::new();
    let full = "abcdefghij";
    // phlow-agent's truncate_chars: char-boundary truncation.
    let truncated: String = full.chars().take(9).collect();
    assert_eq!(truncated, "abcdefghi");
    let y_full = tokenize_y(full);
    let y_trunc = tokenize_y(&truncated);
    evidence.push(format!(
        "char-truncate {full:?} to 9 chars -> {truncated:?}; \
         Y(full) = {y_full:?}, Y(truncated) = {y_trunc:?}"
    ));
    let last = y_trunc.last().cloned().unwrap_or_default();
    if last.chars().count() == 4 {
        return Ok(CaseReport::fail(
            CASE,
            "truncation did not split a Y token: fixture broken".to_string(),
            evidence,
        ));
    }
    evidence.push(format!(
        "the truncation split Y's token \"ij\" -> trailing partial {last:?}: \
         a char-boundary cut is a mid-token cut in Y"
    ));
    // The design's remedy, demonstrated on the mocks: re-encoding
    // through Y yields Y's canonical token sequence.
    let reencoded = reencode_y(&truncated);
    let recanonical = tokenize_y(&reencoded);
    evidence.push(format!(
        "mock remedy: re-encode through Y -> {reencoded:?}, Y(re-encoded) = {recanonical:?} \
         (canonical — the model would receive whole Y-tokens)"
    ));
    if recanonical != y_trunc {
        return Ok(CaseReport::fail(
            CASE,
            "Y re-encode was not canonical: mock broken".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "phlow has no serving tokenizer to re-encode through (zero tokenizer \
         bindings — see the task-level scan): char truncation can split the \
         model's tokens and nothing detects or repairs it"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"truncated": truncated, "split_y_token": true}),
        evidence,
    ))
}

/// A2: budget accounting uses a tokenizer-free number while the
/// provider bills in tokens — and the ledger presents the number in
/// the token-unit field with no estimate flag. Real code:
/// `max_tokens_for` feeds the Ollama payload's `max_tokens`.
fn case_config_number_presented_as_tokens() -> Result<CaseReport, DriverError> {
    const CASE: &str = "config_number_presented_as_tokens";
    let mut evidence = Vec::new();
    let cfg = phlow_config::OllamaConfig::default();
    let max_tokens = phlow_llm::payload::max_tokens_for(cfg.context_length());
    evidence.push(format!(
        "max_tokens_for(context_length = {}) = {max_tokens} \
         (min(8192, context_length / 2); no tokenizer involved)",
        cfg.context_length()
    ));
    if max_tokens != 8192 {
        return Ok(CaseReport::fail(
            CASE,
            format!("expected 8192 for the default config, got {max_tokens}"),
            evidence,
        ));
    }
    let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];
    let payload = phlow_llm::build_chat_payload(&cfg, &messages, &[], None)
        .map_err(|e| fixture_error("chat payload", e))?;
    let field = payload.get("max_tokens").cloned().unwrap_or(Value::Null);
    evidence.push(format!(
        "real Ollama payload carries max_tokens = {field} — a token-unit field \
         fed by an operator-configured number with no tokenizer behind it"
    ));
    if field != serde_json::json!(8192) {
        return Ok(CaseReport::fail(
            CASE,
            format!("payload max_tokens was {field}, not 8192"),
            evidence,
        ));
    }
    evidence.push(
        "the run report records model_calls (a counter) but no token counts: \
         the X-number is presented as Y with no estimate flag and no divergence \
         warning — the design's ledger discipline does not exist"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"max_tokens": max_tokens, "presented_as_y": true}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "char_budget_is_the_ruler" => case_char_budget_is_the_ruler(),
        "mock_tokenizers_diverge_and_surface" => case_mock_tokenizers_diverge_and_surface(),
        "char_truncation_splits_model_tokens" => case_char_truncation_splits_model_tokens(),
        "config_number_presented_as_tokens" => case_config_number_presented_as_tokens(),
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
    // Locate the tokenizer binding first: an exact-token scan for
    // `tokenizer` over every crates/*/src/**/*.rs.
    let root = workspace_root().map_err(|e| TaskFailure {
        where_: "probe".to_string(),
        how: e.to_string(),
        evidence: vec![],
    })?;
    let hits = scan_sources(&root, "tokenizer").map_err(|e| TaskFailure {
        where_: "probe".to_string(),
        how: e.to_string(),
        evidence: vec![],
    })?;
    let mut evidence = vec![
        format!(
            "recon: exact-token source scan for 'tokenizer' over product crates' \
             src/**/*.rs: \
             {} hits (phlow-gauntlet excluded — the harness uses the design vocabulary)",
            hits.len()
        ),
        "recon: no Cargo.toml in the workspace depends on tiktoken, tokenizers, \
         sentencepiece, or hf-hub — there is no tokenizer crate to bind"
            .to_string(),
        "recon: the runtime's context ruler is chars — role_turn \
         (crates/phlow-runtime/src/runtime.rs) ends the role with \"Context budget \
         exhausted\" when must_dumps(messages).chars().count() > \
         budgets.max_context_chars"
            .to_string(),
        "recon: truncation is char-boundary — truncate_chars \
         (crates/phlow-agent/src/memory.rs); the only token-unit number phlow \
         produces is max_tokens_for(context_length) = min(8192, context_length / 2), \
         sent to Ollama as max_tokens"
            .to_string(),
    ];
    if !hits.is_empty() {
        return Err(TaskFailure {
            where_: "probe".to_string(),
            how: format!(
                "a tokenizer binding EXISTS: {} hit(s) — the absence finding is refuted: {}",
                hits.len(),
                hits.join("; ")
            ),
            evidence,
        });
    }
    evidence.push(
        "recon: zero tokenizer hits — the binding the design asks to locate does \
         not exist; the serving model's tokenizer lives inside Ollama, invisible \
         to phlow"
            .to_string(),
    );
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
        "finding: there is no tokenizer X and no tokenizer Y in phlow — budgets \
         are char-counted, truncation is char-boundary, and max_tokens is an \
         operator-configured number presented in token units; none of the design's \
         pass criteria (re-encode truncation through the serving tokenizer, never \
         present X-counts as Y-counts, warn on divergence) can be met"
            .to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam cannot meet the criteria: no tokenizer binding exists in any phlow crate (exact-token source scan for 'tokenizer' over product crates' src/**/*.rs: zero hits (phlow-gauntlet excluded — the harness uses the design vocabulary in its own probes); zero tokenizer dependencies in any Cargo.toml); the runtime counts context in characters (role_turn: must_dumps(messages).chars().count() > max_context_chars, \"Context budget exhausted\"); truncation is char-boundary (truncate_chars, crates/phlow-agent/src/memory.rs) and can split the serving model's tokens with no detection or repair; the max_tokens sent to Ollama is min(8192, context_length / 2) — an operator-configured number with no tokenizer behind it, presented in the token-unit payload field with no estimate flag; the serving model's tokenizer lives inside Ollama and is invisible to phlow. Whether phlow should gain a tokenizer binding (budgeting with the serving model's tokenizer, re-encoding truncation through it, warning on X/Y divergence) is a product decision for Matt, not a gauntlet-authorized change.".to_string(),
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
