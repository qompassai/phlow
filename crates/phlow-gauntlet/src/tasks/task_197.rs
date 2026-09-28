// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 197 — help-first commands (rust, V).
//!
//! The seam is `phlow <cmd> --help`: the Ghostex CLI doctrine's
//! help-first discovery, adapted to phlow-cli. Every subcommand must
//! document itself (`--help` exits 0 with a usage block naming the
//! command), and a static scan of the CLI definition must fail the
//! build when a new subcommand ships without help text.
//!
//! Two cases, both validation: the dynamic subprocess sweep over every
//! subcommand the real binary enumerates (V1), and the static source
//! scan of the `Commands` enum plus the presence of the build-failing
//! clap-level gate `every_subcommand_has_help_string` in
//! `crates/phlow-cli/src/cli.rs` (V2).

use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::tasks::cli_harness;
use crate::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Task id.
pub const ID: &str = "task-197";
/// Task name.
pub const NAME: &str = "help-first commands";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation.
pub const CASES: [&str; 2] = ["all_subcommands_help", "static_help_scan"];

/// Subcommands the doctrine sweep must at least cover. The dynamic
/// enumeration may find more (clap's auto-generated `help`); it must
/// never find fewer than these.
const KNOWN_COMMANDS: &[&str] = &["run", "serve", "check", "status", "tui"];

fn fixture(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Fixture {
        what: what.to_string(),
        detail: format!("task-197: {detail}"),
    }
}

/// V1: enumerate every subcommand from `phlow --help`, then document
/// each: real subcommands via `phlow <cmd> --help`, clap's
/// auto-generated `help` meta-command via `phlow help help` (it takes
/// a command path, not flags, by clap's design). Every invocation
/// must exit 0 with a usage block containing the command name.
fn case_all_subcommands_help() -> Result<CaseReport, TaskDriverError> {
    let top = cli_harness::run_phlow(&["--help"])
        .map_err(|e| fixture("spawn", format!("phlow --help failed to spawn: {e}")))?;
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    if top.code != Some(0) {
        failures.push(format!("phlow --help exited {:?}", top.code));
    }
    let names = cli_harness::parse_help_commands(&top.stdout).map_err(|e| {
        fixture(
            "parse",
            format!("{e} (the help-first doctrine fails closed)"),
        )
    })?;
    evidence.push(format!("enumerated subcommands: {}", names.join(", ")));
    for known in KNOWN_COMMANDS {
        if !names.contains(&known.to_string()) {
            failures.push(format!("known subcommand '{known}' missing from --help"));
        }
    }
    let mut helped = 0;
    for name in &names {
        // clap's auto-generated `help` subcommand is a meta-command: it
        // takes a command path (`phlow help <cmd>`) rather than flags,
        // so `phlow help --help` is a usage error by clap's own design
        // (exit 2, "unrecognized subcommand '--help'"). Its
        // self-documentation is `phlow help help`; every real
        // subcommand uses `<cmd> --help`.
        let args: Vec<&str> = if name == "help" {
            vec!["help", "help"]
        } else {
            vec![name, "--help"]
        };
        let out = cli_harness::run_phlow(&args).map_err(|e| {
            fixture(
                "spawn",
                format!("phlow {} failed to spawn: {e}", args.join(" ")),
            )
        })?;
        if out.code != Some(0) {
            failures.push(format!("phlow {} exited {:?}", args.join(" "), out.code));
            continue;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        if !text.contains("Usage:") {
            failures.push(format!("phlow {} has no Usage: block", args.join(" ")));
        }
        if !text.contains(name.as_str()) {
            failures.push(format!(
                "phlow {} usage block omits the command name",
                args.join(" ")
            ));
        }
        helped += 1;
        evidence.push(format!(
            "{}: exit 0, usage names the command",
            args.join(" ")
        ));
    }
    evidence.push("backend: real phlow binary (subprocess)".to_string());
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "subcommands": names,
            "helped": helped,
            "backend": "real-binary",
        }),
        evidence,
    );
    if !failures.is_empty() {
        report.passed = false;
        report.failures = failures;
    }
    Ok(report)
}

/// Path to the CLI definition under test, resolved from this crate's
/// manifest dir (the workspace layout is fixed: crates side by side).
fn cli_rs_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("phlow-cli")
        .join("src")
        .join("cli.rs")
}

/// Extract `(variant name, source line index)` pairs from the `Commands`
/// enum body. Variant declarations sit at exactly one indent level
/// (`    Run {`, `    Serve,`); field lines sit deeper.
fn enum_variants(body: &str) -> Vec<(String, usize)> {
    let mut variants = Vec::new();
    for (index, line) in body.lines().enumerate() {
        if !line.starts_with("    ") || line.starts_with("     ") {
            continue;
        }
        let token: String = line
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric())
            .collect();
        if token.chars().next().is_some_and(|c| c.is_uppercase()) {
            variants.push((token, index));
        }
    }
    variants
}

/// True when the variant at `line_index` is preceded (skipping blank
/// lines and attributes) by a `///` doc comment — the doc comment
/// clap turns into the variant's help text.
fn has_doc_comment(lines: &[&str], line_index: usize) -> bool {
    let mut index = line_index;
    while index > 0 {
        index -= 1;
        let trimmed = lines[index].trim_start();
        if trimmed.is_empty() || trimmed.starts_with("#[") {
            continue;
        }
        return trimmed.starts_with("///");
    }
    false
}

/// V2: static scan of the CLI definition. Every `Commands` variant must
/// carry a doc comment (clap renders it as the help text), and the
/// clap-level gate `every_subcommand_has_help_string` — the test that
/// fails `cargo test -p phlow-cli` when a variant lacks help — must be
/// present in the crate. The gate itself is executed by the build, not
/// re-run here; this case pins that it exists and that the definition
/// it guards is clean.
fn case_static_help_scan() -> Result<CaseReport, TaskDriverError> {
    let path = cli_rs_path();
    let source = std::fs::read_to_string(&path)
        .map_err(|e| fixture("read", format!("cannot read {}: {e}", path.display())))?;
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let enum_start = source
        .find("pub enum Commands")
        .ok_or_else(|| fixture("scan", "pub enum Commands not found in cli.rs".to_string()))?;
    let body_start = source[enum_start..]
        .find('{')
        .map(|i| enum_start + i + 1)
        .ok_or_else(|| fixture("scan", "Commands enum has no body".to_string()))?;
    // The enum ends at the first column-0 `}` after its opening brace.
    let body_end = source[body_start..]
        .find("\n}")
        .map(|i| body_start + i)
        .ok_or_else(|| fixture("scan", "Commands enum end not found".to_string()))?;
    let body = &source[body_start..body_end];
    let lines: Vec<&str> = body.lines().collect();
    let variants = enum_variants(body);
    if variants.is_empty() {
        failures.push("static scan found zero Commands variants".to_string());
    }
    for (name, index) in &variants {
        if has_doc_comment(&lines, *index) {
            evidence.push(format!("variant {name}: doc comment present"));
        } else {
            failures.push(format!("variant {name} has no doc comment (no help text)"));
        }
    }
    // The build-failing gate must exist in the crate: without it the
    // scan above is advisory, not a gate.
    if source.contains("fn every_subcommand_has_help_string") {
        evidence.push(
            "build gate present: every_subcommand_has_help_string in cli.rs \
             (fails `cargo test -p phlow-cli` when a variant lacks help)"
                .to_string(),
        );
    } else {
        failures.push(
            "build gate every_subcommand_has_help_string missing from cli.rs \
             (V2 requires a test that fails the build)"
                .to_string(),
        );
    }
    evidence.push(format!("scanned {}", path.display()));
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "variants": variants.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            "build_gate_present": failures.is_empty(),
            "backend": "static-scan",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "all_subcommands_help" => case_all_subcommands_help(),
        "static_help_scan" => case_static_help_scan(),
        _ => Err(fixture("case", format!("unknown case '{case}'"))),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — every
/// subcommand answers `--help`.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-197".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-197".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
