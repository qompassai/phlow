// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Integration tests for task-186 (scan-once SQLite index).
//!
//! Two driver cases against the scripted doubles (FsLog read counter +
//! synthetic 500-session corpus): a full build indexes all 500 with
//! recorded mtimes, and a rescan after adding 3 / modifying 1 reads
//! exactly 4 files.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_186;

fn check_case(case: &'static str) -> CaseReport {
    let report = task_186::run_case(case)
        .unwrap_or_else(|e| panic!("task-186 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-186 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: 500 files → 500 rows, all 500 queryable by unique token,
/// mtimes recorded for the 5-file sample.
#[test]
fn full_build_queryable() {
    assert_eq!(task_186::ID, "task-186");
    let report = check_case("full_build_queryable");
    let m = &report.metrics;
    assert_eq!(m["files_read"].as_u64().unwrap(), 500);
    assert_eq!(m["rows"].as_u64().unwrap(), 500);
    assert_eq!(
        m["queried_ok"].as_u64().unwrap(),
        500,
        "every session must be queryable by its token"
    );
    assert_eq!(m["mtime_ok"].as_u64().unwrap(), 5);
}

/// V2: add 3 + modify 1 → rescan reads exactly 4 files; the log names
/// exactly the changed set; unchanged files are not re-read.
#[test]
fn incremental_rescan_four_reads() {
    let report = check_case("incremental_rescan_four_reads");
    let m = &report.metrics;
    assert_eq!(m["rescan_files_read"].as_u64().unwrap(), 4);
    assert_eq!(
        m["rescan_fs_reads"].as_u64().unwrap(),
        4,
        "fs-access log must show exactly 4 reads"
    );
    assert_eq!(m["rows"].as_u64().unwrap(), 503);
    let joined = report.evidence.join("\n");
    for name in [
        "new0.session.json",
        "new1.session.json",
        "new2.session.json",
        "s0000.session.json",
    ] {
        assert!(
            joined.contains(name),
            "evidence must name the re-read file {name}:\n{joined}"
        );
    }
}
