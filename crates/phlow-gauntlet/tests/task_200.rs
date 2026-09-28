//! Integration tests for task-200 (hostile output sanitization).
//!
//! Both adversarial driver cases: a hostile entity id
//! (`\x1b[2J` clear-screen + newline) through `check --name --json`
//! must stay valid JSON with the id preserved exactly and no raw
//! control bytes on stdout (A1); every `--help` output must be
//! control-byte free, with the build-time gates pinned (A2).

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_200;

fn check_case(case: &str) -> CaseReport {
    let report = task_200::run_case(case)
        .unwrap_or_else(|e| panic!("task-200 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-200 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// A1: the hostile id round-trips through `--json` — stdout parses,
/// the parsed id is byte-identical (escape, not strip), and the raw
/// bytes contain no unescaped control bytes.
#[test]
fn hostile_id_json_stays_valid() {
    assert_eq!(task_200::ID, "task-200");
    let report = check_case("hostile_id_json_stays_valid");
    let m = &report.metrics;
    assert!(
        m["id_preserved"].as_bool().unwrap(),
        "the parsed id must preserve the hostile value exactly"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("stdout parses as JSON"),
        "evidence must show the JSON stayed valid:\n{joined}"
    );
    assert!(
        joined.contains("no unescaped control bytes"),
        "evidence must show the raw bytes stayed inert:\n{joined}"
    );
}

/// A2: `phlow --help` and every `phlow <cmd> --help` are free of raw
/// control bytes, and the build-time gates (the control-byte scan and
/// the clap-renders-verbatim proof) are pinned present in phlow-cli.
#[test]
fn help_has_no_raw_control_bytes() {
    let report = check_case("help_has_no_raw_control_bytes");
    let m = &report.metrics;
    assert!(
        m["help_targets"].as_u64().unwrap() >= 6,
        "expected the root help plus every subcommand, got: {m}"
    );
    assert_eq!(
        m["clean"].as_u64().unwrap(),
        m["help_targets"].as_u64().unwrap(),
        "every help output must be control-byte free"
    );
    assert!(
        m["build_gates_pinned"].as_bool().unwrap(),
        "the build-time gates must be present in phlow-cli"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("hostile_about_is_sanitized"),
        "evidence must pin the clap-sanitization proof:\n{joined}"
    );
}
