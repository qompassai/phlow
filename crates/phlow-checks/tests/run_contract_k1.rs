//! K1 — the fleet run-contract test kit, run against phlow first
//! (INDEX §2.1, §3 item 1; phlow plan PH-11 / PH-K1).
//!
//! The kit's shape, which every other server's K1 copies:
//!
//! 1. Start a run: a worker process tree in its **own process group**,
//!    plus a run record naming that group and a deadline.
//! 2. Kill the server: here the "server" is this test process's handle
//!    on the run — dropping it without `wait()` is exactly the state a
//!    dead server leaves behind (an orphaned group, a run directory,
//!    no done-marker).
//! 3. Run one reaping pass and assert: the run is reaped by its
//!    deadline, its done-marker says `abandoned`, and **no process of
//!    its group survives**.
//!
//! A second test covers the heartbeat path: a run whose heartbeats stop
//! is reaped before its (distant) deadline.
//!
//! Unix only: the contract's process-group discipline is a Unix
//! discipline (the check runner has the same split).

#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use phlow_checks::run_contract::{
    DoneState, ReapDecision, RunDir, group_member_pids, now_unix_ms, reap_expired_runs,
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_root(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "phlow-k1-test-{prefix}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).expect("test setup: create temp root");
    dir
}

/// Spawn the run's worker tree: a shell that starts two `sleep`
/// grandchildren and waits — a three-process group, so a leader-only
/// kill provably leaves survivors and the kit would catch it. The
/// child leads its own process group, exactly as the check runner
/// spawns checks. Returns the child handle and the group id.
fn spawn_run_tree() -> (Child, u32) {
    let child = Command::new("/bin/sh")
        .args(["-c", "sleep 300 & sleep 300 & wait"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .expect("test setup: spawn run tree");
    let pgid = child.id();
    (child, pgid)
}

/// Bounded group-emptiness probe for the kit's assertions. This is a
/// test-side assertion with a hard 5 s cap, not a production waiter
/// loop: the contract forbids servers polling peers, and the reaper
/// itself never waits — but the kit must *prove* the group died, and
/// SIGKILL delivery is not instantaneous.
fn assert_group_gone(pgid: u32) {
    for _ in 0..500 {
        if group_member_pids(pgid).is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("process group {pgid} still has members after reaping");
}

/// Bounded probe for the tree being up, with the same hard 5 s cap
/// as `assert_group_gone`: fork scheduling is not instantaneous on a
/// loaded host, so the kit waits for the shell to fork its two sleeps
/// before asserting the tree is alive. The reap assertions below stay
/// exact; only the fixture precondition gets the settle window.
fn assert_group_populated(pgid: u32, want: usize) {
    for _ in 0..500 {
        if group_member_pids(pgid).len() >= want {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("run tree (shell + two sleeps) is alive before the reap");
}

#[test]
fn k1_killed_server_run_is_reaped_by_its_deadline() {
    let root = temp_root("deadline");
    let started = now_unix_ms();
    let deadline = started + 400;
    let (server_handle, pgid) = spawn_run_tree();
    let run = RunDir::create(&root, "k1-deadline", started, deadline, 100, Some(pgid))
        .expect("test setup: create run");
    assert!(run.read_done().unwrap().is_none());
    assert_group_populated(pgid, 3);

    // Kill the server: drop the only handle without waiting. The group
    // is orphaned; nothing in this process will reap it.
    drop(server_handle);

    // One reaping pass before the deadline must leave the run alone
    // while its heartbeat is fresh.
    let pass = reap_expired_runs(&root, started + 50).unwrap();
    assert!(
        pass.decisions
            .contains(&("k1-deadline".to_owned(), ReapDecision::Alive))
    );

    // At the deadline, one pass reaps it.
    let pass = reap_expired_runs(&root, deadline).unwrap();
    assert!(
        pass.decisions
            .contains(&("k1-deadline".to_owned(), ReapDecision::DeadlinePassed))
    );
    let marker = run
        .read_done()
        .unwrap()
        .expect("reaped run has a done-marker");
    assert_eq!(marker.state, DoneState::Abandoned);
    assert_group_gone(pgid);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn k1_stale_heartbeat_run_is_reaped_before_its_deadline() {
    let root = temp_root("stale");
    let started = now_unix_ms();
    let (server_handle, pgid) = spawn_run_tree();
    let run = RunDir::create(
        &root,
        "k1-stale",
        started,
        started + 3_600_000, // deadline an hour out: only staleness can reap
        100,                 // heartbeat interval: stale after 200 ms
        Some(pgid),
    )
    .expect("test setup: create run");
    drop(server_handle);

    // 10 s past the last heartbeat (written at create): stale.
    let pass = reap_expired_runs(&root, started + 10_000).unwrap();
    assert!(
        pass.decisions
            .contains(&("k1-stale".to_owned(), ReapDecision::HeartbeatStale))
    );
    assert_eq!(
        run.read_done().unwrap().unwrap().state,
        DoneState::Abandoned
    );
    assert_group_gone(pgid);
    let _ = std::fs::remove_dir_all(&root);
}
