//! Integration tests for task-197 (help-first commands).
//!
//! Both driver cases against the real `phlow` binary (subprocess):
//! the dynamic `--help` sweep over every enumerated subcommand (V1)
//! and the static source scan of the `Commands` enum plus the
//! build-failing help gate (V2).

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_197;

fn check_case(case: &str) -> CaseReport {
    let report = task_197::run_case(case)
        .unwrap_or_else(|e| panic!("task-197 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-197 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: every subcommand enumerated from `phlow --help` answers
/// `phlow <cmd> --help` with exit 0 and a usage block naming the
/// command. The enumeration must cover at least the five known
/// commands (run, serve, check, status, tui).
#[test]
fn all_subcommands_help() {
    assert_eq!(task_197::ID, "task-197");
    let report = check_case("all_subcommands_help");
    let m = &report.metrics;
    let subs: Vec<&str> = m["subcommands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for known in ["run", "serve", "check", "status", "tui"] {
        assert!(
            subs.contains(&known),
            "known subcommand {known} missing from enumeration: {subs:?}"
        );
    }
    assert_eq!(
        m["helped"].as_u64().unwrap() as usize,
        subs.len(),
        "every enumerated subcommand must pass --help"
    );
    assert_eq!(m["backend"].as_str().unwrap(), "real-binary");
}

/// V2: the static scan finds all five `Commands` variants with doc
/// comments, and pins the build gate
/// `every_subcommand_has_help_string` — the test that fails
/// `cargo test -p phlow-cli` when a new variant ships without help.
#[test]
fn static_help_scan() {
    let report = check_case("static_help_scan");
    let m = &report.metrics;
    let variants: Vec<&str> = m["variants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for want in ["Run", "Serve", "Check", "Status", "Tui"] {
        assert!(
            variants.contains(&want),
            "variant {want} missing from static scan: {variants:?}"
        );
    }
    assert!(
        m["build_gate_present"].as_bool().unwrap(),
        "the build-failing help gate must be present in phlow-cli"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("every_subcommand_has_help_string"),
        "evidence must name the build gate:\n{joined}"
    );
}
