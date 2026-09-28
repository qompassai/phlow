// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Task 192 — shared index, concurrent readers (rust, V/A).
//!
//! One `index.sqlite` serves the TUI and the daemon concurrently.
//! The index opens in WAL mode: 8 readers × 100 queries run with
//! zero lock errors and every result correct. While a rescan writes,
//! each query's hits carry the rescan generation — a page mixing
//! generations would be a torn read; the rescan commits atomically,
//! so readers see pre- or post-rescan state only.
//!
//! This task also runs the wave-30 license gate: every adapted file
//! must start with the exact maddada attribution header.
//!
//! Primary sources: SQLite WAL-mode documentation
//! (sqlite.org/wal.html — readers do not block writers and writers
//! do not block readers); Ghostex "one shared engine for TUI +
//! daemon" (adaptation map); MIT attribution requirement.

use std::path::PathBuf;
use std::sync::{Arc, Barrier, Mutex, mpsc};

use crate::session_find::{
    ATTRIBUTION_LINES, DB_FILE_NAME, FsLog, IndexError, ScanStats, SessionIndex, SessionSpec,
    SqlLog, fresh_temp_dir, license_audit, pairing_corpus, scan_and_index, write_session,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-192";
/// Task name.
pub const NAME: &str = "shared index, concurrent readers";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 1 validation + 1 adversarial.
pub const CASES: [&str; 2] = [
    "concurrent_readers_no_lock_errors",
    "rescan_during_reads_no_torn_rows",
];
/// Concurrent readers in the harness.
pub const READERS: usize = 8;
/// Queries per reader.
pub const QUERIES_PER_READER: usize = 100;
/// Queries per reader in the rescan race.
pub const RACE_QUERIES: usize = 200;

/// Files adapted from the Ghostex find concept in this wave: the
/// shared engine plus the seven task drivers.
fn adapted_files() -> Vec<PathBuf> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![src.join("session_find.rs")];
    for n in 186..=192 {
        files.push(src.join("tasks").join(format!("task_{n}.rs")));
    }
    files
}

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

fn build_shared_index(tag: &str) -> (PathBuf, PathBuf, String) {
    let tmp = fresh_temp_dir(tag);
    let sessions = tmp.join("sessions");
    std::fs::create_dir_all(&sessions).expect("session dir must be creatable");
    let target_id = pairing_corpus(&sessions);
    let db = tmp.join("index").join(DB_FILE_NAME);
    std::fs::create_dir_all(db.parent().unwrap()).expect("index dir must be creatable");
    let mut fs_log = FsLog::new();
    let mut sql_log = SqlLog::new();
    scan_and_index(&sessions, &db, &mut fs_log, &mut sql_log).expect("scan must succeed");
    (tmp, db, target_id)
}

/// One V1 reader thread: open the shared index, then run
/// `QUERIES_PER_READER` searches, counting correct top hits and
/// errors (lock errors count as errors too).
fn reader_task(
    db: Arc<PathBuf>,
    target_id: Arc<String>,
    barrier: Arc<Barrier>,
    total_ok: Arc<Mutex<usize>>,
    total_err: Arc<Mutex<usize>>,
) -> impl FnOnce() + Send {
    move || {
        let mut index = match SessionIndex::open_strict(&db) {
            Ok(i) => i,
            Err(_) => {
                *total_err.lock().unwrap() += QUERIES_PER_READER;
                return;
            }
        };
        barrier.wait();
        for _ in 0..QUERIES_PER_READER {
            match index.search("pairing brute force", 5) {
                Ok(hits) => {
                    if hits.first().is_some_and(|h| h.id == *target_id) {
                        *total_ok.lock().unwrap() += 1;
                    } else {
                        *total_err.lock().unwrap() += 1;
                    }
                }
                Err(_) => *total_err.lock().unwrap() += 1,
            }
        }
    }
}

/// The wave-30 license gate: every adapted file carries the exact
/// attribution header. Returns the offenders for the report.
fn check_license_audit(failures: &mut Vec<String>) -> Vec<PathBuf> {
    let offenders = license_audit(&adapted_files());
    if !offenders.is_empty() {
        failures.push(format!("license audit offenders: {offenders:?}"));
    }
    offenders
}

/// V1: 8 readers × 100 queries on one WAL index → all correct, zero
/// lock errors. Also runs the wave-30 license audit.
fn case_concurrent_readers_no_lock_errors() -> Result<CaseReport, TaskDriverError> {
    let (_tmp, db, target_id) = build_shared_index("t192-v1");
    let db = Arc::new(db);
    let target_id = Arc::new(target_id);
    let barrier = Arc::new(Barrier::new(READERS));
    let mut failures = Vec::new();
    let total_ok = Arc::new(Mutex::new(0usize));
    let total_err = Arc::new(Mutex::new(0usize));
    std::thread::scope(|scope| {
        for _ in 0..READERS {
            scope.spawn(reader_task(
                Arc::clone(&db),
                Arc::clone(&target_id),
                Arc::clone(&barrier),
                Arc::clone(&total_ok),
                Arc::clone(&total_err),
            ));
        }
    });
    let ok = *total_ok.lock().unwrap();
    let err = *total_err.lock().unwrap();
    if err != 0 {
        failures.push(format!("{err} query failures (lock errors or wrong hits)"));
    }
    if ok != READERS * QUERIES_PER_READER {
        failures.push(format!(
            "{ok} correct queries, want {}",
            READERS * QUERIES_PER_READER
        ));
    }
    let offenders = check_license_audit(&mut failures);
    let evidence = vec![
        format!(
            "{READERS} readers × {QUERIES_PER_READER} queries: {ok} correct, {err} errors (WAL mode)"
        ),
        format!(
            "license audit over {} adapted files: {} offenders (header: {})",
            adapted_files().len(),
            offenders.len(),
            ATTRIBUTION_LINES[0],
        ),
    ];
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "readers": READERS,
            "queries_per_reader": QUERIES_PER_READER,
            "correct": ok,
            "errors": err,
            "license_offenders": offenders.len(),
            "backend": "sqlite-wal",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// One A1 reader thread: open the shared index, run `RACE_QUERIES`
/// searches, and check every hit page for single-generation
/// consistency (a torn page mixes generations) plus the right top hit.
fn race_reader_task(
    db: Arc<PathBuf>,
    target_id: Arc<String>,
    barrier: Arc<Barrier>,
    torn: Arc<Mutex<usize>>,
    wrong: Arc<Mutex<usize>>,
    errors: Arc<Mutex<usize>>,
    generations: Arc<Mutex<Vec<u64>>>,
) -> impl FnOnce() + Send {
    move || {
        let mut index = match SessionIndex::open_strict(&db) {
            Ok(i) => i,
            Err(_) => {
                *errors.lock().unwrap() += RACE_QUERIES;
                return;
            }
        };
        barrier.wait();
        for _ in 0..RACE_QUERIES {
            match index.search("pairing brute force", 10) {
                Ok(hits) => {
                    let mut gens: Vec<u64> = hits.iter().map(|h| h.generation).collect();
                    gens.sort();
                    gens.dedup();
                    if gens.len() > 1 {
                        *torn.lock().unwrap() += 1;
                    }
                    generations.lock().unwrap().extend(gens);
                    if !hits.first().is_some_and(|h| h.id == *target_id) {
                        *wrong.lock().unwrap() += 1;
                    }
                }
                Err(_) => *errors.lock().unwrap() += 1,
            }
        }
    }
}

/// The A1 writer thread: two rescans (each bumps the generation)
/// while readers are mid-flight. Each rescan's `Result` goes back
/// over the channel — a discarded error would make the whole
/// scenario vacuous.
fn race_writer_task(
    sessions: PathBuf,
    db: Arc<PathBuf>,
    barrier: Arc<Barrier>,
    done: mpsc::Sender<Result<ScanStats, IndexError>>,
) -> impl FnOnce() + Send {
    move || {
        barrier.wait();
        for round in 0..2 {
            let spec = SessionSpec::new(
                &format!("race-{round}"),
                &format!("race session {round} pairing notes"),
                "brute force attempts seen in logs",
            );
            write_session(&sessions, &format!("race{round}"), &spec);
            let mut fs_log = FsLog::new();
            let mut sql_log = SqlLog::new();
            let result = scan_and_index(&sessions, &db, &mut fs_log, &mut sql_log);
            let _ = done.send(result);
        }
    }
}

/// A1: rescan writes while 8 readers query → every returned page is
/// single-generation (never torn); readers see only committed states.
fn case_rescan_during_reads_no_torn_rows() -> Result<CaseReport, TaskDriverError> {
    let (_tmp, db, target_id) = build_shared_index("t192-a1");
    let sessions = db.parent().unwrap().parent().unwrap().join("sessions");
    let db = Arc::new(db);
    let target_id = Arc::new(target_id);
    let barrier = Arc::new(Barrier::new(READERS + 1));
    let torn = Arc::new(Mutex::new(0usize));
    let wrong = Arc::new(Mutex::new(0usize));
    let errors = Arc::new(Mutex::new(0usize));
    let generations = Arc::new(Mutex::new(Vec::new()));
    let (write_tx, write_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..READERS {
            scope.spawn(race_reader_task(
                Arc::clone(&db),
                Arc::clone(&target_id),
                Arc::clone(&barrier),
                Arc::clone(&torn),
                Arc::clone(&wrong),
                Arc::clone(&errors),
                Arc::clone(&generations),
            ));
        }
        scope.spawn(race_writer_task(
            sessions,
            Arc::clone(&db),
            Arc::clone(&barrier),
            write_tx,
        ));
    });
    let mut failures = Vec::new();
    let mut rescans_ok = 0usize;
    for result in write_rx {
        match result {
            Ok(_) => rescans_ok += 1,
            Err(e) => failures.push(format!("writer rescan failed: {e}")),
        }
    }
    if rescans_ok != 2 {
        failures.push(format!("writer completed {rescans_ok}/2 rescans"));
    }
    let outcome = RaceOutcome {
        torn_pages: *torn.lock().unwrap(),
        wrong_hits: *wrong.lock().unwrap(),
        err_count: *errors.lock().unwrap(),
        rescans_ok,
        gens: sorted_generations(&generations),
    };
    Ok(build_race_report(outcome, failures))
}

/// Counters collected from the racing readers and writer.
struct RaceOutcome {
    torn_pages: usize,
    wrong_hits: usize,
    err_count: usize,
    rescans_ok: usize,
    gens: Vec<u64>,
}

/// Distinct generations observed, sorted, for the verdict.
fn sorted_generations(generations: &Mutex<Vec<u64>>) -> Vec<u64> {
    let mut gens = generations.lock().unwrap().clone();
    gens.sort();
    gens.dedup();
    gens
}

/// Verdict over the race counters: no torn pages, no wrong top hits,
/// no errors, and only pre/post generations observed.
fn build_race_report(mut outcome: RaceOutcome, mut failures: Vec<String>) -> CaseReport {
    if outcome.torn_pages != 0 {
        failures.push(format!(
            "{} torn pages (mixed generations in one result)",
            outcome.torn_pages
        ));
    }
    if outcome.wrong_hits != 0 {
        failures.push(format!("{} queries with wrong top hit", outcome.wrong_hits));
    }
    if outcome.err_count != 0 {
        failures.push(format!("{} query errors during rescan", outcome.err_count));
    }
    // Two rescans happened: readers may only ever see generations
    // from the pre/post states, at most 3 distinct values.
    if outcome.gens.len() > 3 {
        failures.push(format!(
            "saw {} distinct generations, want <= 3: {:?}",
            outcome.gens.len(),
            outcome.gens
        ));
    }
    let evidence = vec![
        format!(
            "{READERS} readers × {RACE_QUERIES} queries during 2 rescans: torn pages={}, wrong top={}, errors={}",
            outcome.torn_pages, outcome.wrong_hits, outcome.err_count,
        ),
        format!(
            "generations observed across all pages: {:?} (pre/post states only)",
            outcome.gens
        ),
    ];
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "readers": READERS,
            "race_queries": RACE_QUERIES,
            "torn_pages": outcome.torn_pages,
            "wrong_top": outcome.wrong_hits,
            "errors": outcome.err_count,
            "rescans_ok": outcome.rescans_ok,
            "generations_seen": std::mem::take(&mut outcome.gens),
            "backend": "sqlite-wal",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    report
}

/// Run one driver case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "concurrent_readers_no_lock_errors" => case_concurrent_readers_no_lock_errors(),
        "rescan_during_reads_no_torn_rows" => case_rescan_during_reads_no_torn_rows(),
        _ => Err(arm_error(
            "case",
            format!("task-192: unknown case '{case}'"),
        )),
    }
}

/// Task-level entry for the gauntlet runner: the headline case.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-192".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-192".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
