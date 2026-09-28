// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 200 — hostile output sanitization (rust, A).
//!
//! The seam is untrusted bytes reaching the terminal, adapted from
//! Ghostex's CLI doctrine: an entity id containing an ESC
//! clear-screen sequence (`\x1b[2J`) plus a newline must round-trip
//! through `--json` as valid JSON (the id preserved exactly, no raw
//! control bytes on stdout), and help text containing terminal
//! escapes must render inert.
//!
//! The declared contract (tested, not wished):
//! * Machine output is JSON-string-escaped by the serializer —
//!   `phlow`'s `python_json_dumps` emits ASCII-only JSON and escapes
//!   every control character, so hostile values stay data, never
//!   terminal commands.
//! * Help text is sanitized twice: clap itself strips ANSI escape
//!   sequences and control bytes from rendered help (verified
//!   empirically — `\x1b[2J` renders as nothing), and the
//!   build-failing scan `all_help_strings_are_control_byte_free` in
//!   `crates/phlow-cli/src/cli.rs` enforces that no help literal
//!   carries raw control bytes at the source. Untrusted data is never
//!   interpolated into help strings.
//!
//! Two adversarial cases: hostile entity id through `--json` (A1)
//! and help-output byte hygiene across every subcommand (A2).

use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::tasks::cli_harness;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-200";
/// Task name.
pub const NAME: &str = "hostile output sanitization";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 adversarial.
pub const CASES: [&str; 2] = [
    "hostile_id_json_stays_valid",
    "help_has_no_raw_control_bytes",
];

/// The hostile entity id: ESC clear-screen plus a newline, the
/// terminal-attack pair from the doctrine.
const HOSTILE_ID: &str = "\u{1b}[2J\nEVIL-ID";

fn fixture(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Fixture {
        what: what.to_string(),
        detail: format!("task-200: {detail}"),
    }
}

/// A1: an entity id containing `\x1b[2J` plus newline goes through
/// `phlow check --name <hostile> --json`. The stdout must parse as
/// JSON, the parsed id must preserve the hostile value exactly, and
/// the raw stdout must contain no unescaped control bytes (no raw
/// ESC, no raw newline except the single trailing line terminator).
fn case_hostile_id_json_stays_valid() -> Result<CaseReport, TaskDriverError> {
    let dir = cli_harness::case_temp_dir("200-a1");
    let out = cli_harness::run_phlow_in(&dir, &["check", "--name", HOSTILE_ID, "--json"])
        .map_err(|e| fixture("spawn", format!("hostile check failed to spawn: {e}")))?;
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    evidence.push(format!("exit code: {:?}", out.code));
    let report = cli_harness::parse_json_stdout(&out, "hostile check --json")
        .map_err(|e| fixture("json", e))?;
    evidence.push("stdout parses as JSON".to_string());
    // The parsed id preserves the hostile value exactly: sanitization
    // is escape, not strip — data is never silently altered.
    let echoed = report
        .get("checks")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
        .and_then(|c| c.get("name"))
        .and_then(|n| n.as_str());
    match echoed {
        Some(id) if id == HOSTILE_ID => {
            evidence.push(
                "parsed id preserves the hostile value exactly (escape, not strip)".to_string(),
            );
        }
        Some(id) => failures.push(format!("parsed id was altered: {id:?}")),
        None => failures.push("report has no checks[0].name to compare".to_string()),
    }
    // The raw bytes stay inert: no raw ESC byte anywhere in stdout.
    if out.stdout.contains(&0x1b) {
        failures.push("raw stdout contains an unescaped ESC byte (0x1b)".to_string());
    } else {
        evidence.push("raw stdout: no ESC byte".to_string());
    }
    match cli_harness::assert_no_raw_control_bytes(&out.stdout, "hostile check --json stdout") {
        Ok(()) => evidence.push("raw stdout: no unescaped control bytes".to_string()),
        Err(e) => failures.push(e),
    }
    evidence.push(
        "backend: real phlow binary (subprocess), serializer = python_json_dumps".to_string(),
    );
    let mut report_out = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "exit_code": out.code,
            "id_preserved": echoed == Some(HOSTILE_ID),
            "backend": "real-binary",
        }),
        evidence,
    );
    if !failures.is_empty() {
        report_out.passed = false;
        report_out.failures = failures;
    }
    Ok(report_out)
}

/// A2: sweep `phlow --help` and every `phlow <cmd> --help`: the raw
/// stdout must contain no raw control bytes. This is the runtime
/// half of the declared contract; the build half is the
/// `all_help_strings_are_control_byte_free` unit test in
/// `crates/phlow-cli/src/cli.rs`, which fails the build when any
/// help literal carries a control byte, plus
/// `hostile_about_is_sanitized_by_clap`, which pins clap's own
/// stripping of ANSI escapes and control bytes from rendered help.
fn case_help_has_no_raw_control_bytes() -> Result<CaseReport, TaskDriverError> {
    let top = cli_harness::run_phlow(&["--help"])
        .map_err(|e| fixture("spawn", format!("phlow --help failed to spawn: {e}")))?;
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let names = cli_harness::parse_help_commands(&top.stdout).map_err(|e| fixture("parse", e))?;
    let mut targets: Vec<Vec<String>> = vec![vec!["--help".to_string()]];
    for name in &names {
        // Same clap meta-command carve-out as task-197: `phlow help
        // --help` is a usage error by clap's design; the help
        // subcommand documents itself via `phlow help help`.
        if name == "help" {
            targets.push(vec!["help".to_string(), "help".to_string()]);
        } else {
            targets.push(vec![name.clone(), "--help".to_string()]);
        }
    }
    let mut clean = 0;
    for target in &targets {
        let label = format!("phlow {}", target.join(" "));
        let arg_refs: Vec<&str> = target.iter().map(String::as_str).collect();
        let out = cli_harness::run_phlow(&arg_refs)
            .map_err(|e| fixture("spawn", format!("{label} failed to spawn: {e}")))?;
        match cli_harness::assert_no_raw_control_bytes(&out.stdout, &label) {
            Ok(()) => {
                clean += 1;
                evidence.push(format!(
                    "{label}: {} bytes, no raw control bytes",
                    out.stdout.len()
                ));
            }
            Err(e) => failures.push(e),
        }
    }
    // Pin the build-time half of the contract: the unit test that
    // fails `cargo test -p phlow-cli` on a hostile help literal must
    // exist in the crate.
    let cli_rs = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("phlow-cli")
        .join("src")
        .join("cli.rs");
    let source = std::fs::read_to_string(&cli_rs)
        .map_err(|e| fixture("read", format!("cannot read {}: {e}", cli_rs.display())))?;
    for gate in [
        "fn all_help_strings_are_control_byte_free",
        "fn hostile_about_is_sanitized_by_clap",
    ] {
        if source.contains(gate) {
            evidence.push(format!("build gate present: {gate} in cli.rs"));
        } else {
            failures.push(format!("build gate {gate} missing from cli.rs"));
        }
    }
    let mut report_out = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "help_targets": targets.len(),
            "clean": clean,
            "build_gates_pinned": failures.is_empty(),
            "backend": "real-binary+static-scan",
        }),
        evidence,
    );
    if !failures.is_empty() {
        report_out.passed = false;
        report_out.failures = failures;
    }
    Ok(report_out)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "hostile_id_json_stays_valid" => case_hostile_id_json_stays_valid(),
        "help_has_no_raw_control_bytes" => case_help_has_no_raw_control_bytes(),
        _ => Err(fixture("case", format!("unknown case '{case}'"))),
    }
}

/// Task-level entry for the gauntlet runner: the headline case is the
/// hostile-id-through-JSON case.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-200".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-200".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
