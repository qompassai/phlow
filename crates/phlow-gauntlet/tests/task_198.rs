//! Integration tests for task-198 (JSON is a contract).
//!
//! Three validation driver cases against the real `phlow` binary
//! (subprocess): read-twice JSON stability with the TUI `--json`
//! refusal (V1), entity-id stability across runs on a seeded config
//! (V2), and schema snapshots for the `status`/`check`/`run` report
//! shapes (V3).

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_198;

fn check_case(case: &str) -> CaseReport {
    let report = task_198::run_case(case)
        .unwrap_or_else(|e| panic!("task-198 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-198 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: `status --json` twice — both parse, field-name sets identical;
/// `--json` before the subcommand behaves the same; `tui --json` is a
/// usage error (exit 2), never a silent human dump.
#[test]
fn read_twice_json_stable() {
    assert_eq!(task_198::ID, "task-198");
    let report = check_case("read_twice_json_stable");
    let m = &report.metrics;
    assert!(
        m["field_paths"].as_u64().unwrap() > 10,
        "expected a real status report shape, got: {m}"
    );
    assert_eq!(
        m["tui_json_refused_with"].as_i64().unwrap(),
        2,
        "tui --json must be a usage error (exit 2)"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("usage error on stderr"),
        "evidence must show the TUI refusal:\n{joined}"
    );
}

/// V2: the same seeded config read twice — entity ids (check names)
/// identical across runs, and the config seeded exactly two of them.
#[test]
fn ids_stable_across_runs() {
    let report = check_case("ids_stable_across_runs");
    let m = &report.metrics;
    let ids: Vec<&str> = m["entity_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["alpha", "beta"],
        "the seeded entity ids must be stable and exact"
    );
    assert_eq!(m["rounds"].as_u64().unwrap(), 2);
}

/// V3: the `status`, `check`, and `run` report schemas match the
/// checked-in snapshots — no renamed field, dropped key, or retyped
/// value slips past.
#[test]
fn schema_snapshots() {
    let report = check_case("schema_snapshots");
    let m = &report.metrics;
    let snapshots: Vec<&str> = m["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(snapshots, ["status", "check", "run"]);
    assert_eq!(
        m["matched"].as_u64().unwrap(),
        3,
        "all three schemas must match their snapshots"
    );
}
