//! task-25: backpressure propagation (rust).
//!
//! Drives phlow's real producer/consumer channel: the bounded mpsc channel
//! inside [`phlow_runtime::transport::MsgpackTransport`]
//! (`crates/phlow-runtime/src/transport/msgpack.rs`). One current-thread
//! tokio worker owns the Neovim socket; `exec` calls cross a
//! `WORKER_QUEUE_CAPACITY = 1` mpsc channel, and `exec` takes `&mut self`
//! so at most one request is ever in flight from one transport.
//!
//! Scope, stated plainly: this is the editor-transport channel, not an
//! orchestration-stage event bus — no event bus exists between phlow's
//! orchestration stages (the recon documents this), the experiment
//! Scheduler's queue is admission-only with no consumer, and the tuios
//! accept-loop permit pool throttles TCP connections rather than carrying
//! stage messages. The msgpack channel IS a genuine producer/consumer
//! channel with backpressure semantics, and it is the only one in the
//! codebase: bounded capacity, the producer parks (send-await / reply-await)
//! instead of buffering unboundedly, and a per-request timeout fails
//! explicitly instead of hanging.
//!
//! The driver scripts a fake msgpack-RPC peer (rmpv, the same codec the
//! transport uses) over a Unix socket — real transport code, real channel,
//! controllable slow consumer. Four cases: matched rates, a slow consumer
//! the producer parks behind, a 30-second full stall the producer rides
//! out with flat memory and zero lost messages, and a black-hole peer
//! that proves the timeout is explicit (then recovery via reconnect).
//!
//! [`phlow_runtime::transport::MsgpackTransport`]: https://github.com/qompassai/phlow
//! (local path `crates/phlow-runtime/src/transport/msgpack.rs`)

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_editor::contract::SCHEMAS_LUA;
use phlow_editor::{EditorTransport, TransportError};
use phlow_runtime::transport::MsgpackTransport;
use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

/// Task id.
pub const ID: &str = "task-25";
/// Human-readable name.
pub const NAME: &str = "backpressure propagation";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "matched_rates",
    "slow_consumer_parks",
    "stall_30s_resumes_cleanly",
    "timeout_is_explicit",
];

/// Cap on the scripted peer's read buffer: a request frame is a few hundred
/// bytes; anything past 1 MiB without a decodable frame fails the peer
/// closed rather than accumulating.
const PEER_BUFFER_BYTES_MAX: usize = 1 << 20;
/// Poll interval of the peer's accept/read loops, in milliseconds.
const PEER_POLL_MS: u64 = 10;
/// Socket read timeout so handler threads observe shutdown promptly.
const PEER_READ_TIMEOUT: Duration = Duration::from_millis(200);
/// Slow-consumer delay for the parking case, in seconds.
const SLOW_DELAY_SECS: u64 = 2;
/// Full-stall duration for the adversarial case, in seconds.
const STALL_SECS: u64 = 30;
/// Parallel producers for the stall case.
const STALL_PRODUCERS: usize = 4;
/// Generous RSS-growth bound for the stall case, in MiB. Real growth is
/// ~zero (capacity-1 channel); the bound only catches true OOM-shaped
/// blowups while tolerating parallel-test noise in the shared process.
const RSS_GROWTH_MIB_MAX: f64 = 128.0;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-25 driver itself (not of the transport under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A workspace path was missing or unreadable.
    Path {
        /// Which file was wanted.
        what: String,
        /// Path plus I/O detail.
        detail: String,
    },
    /// The scripted peer failed (bind, accept, I/O).
    Peer {
        /// What the peer was doing.
        detail: String,
    },
    /// The transport returned an unexpected error.
    Transport {
        /// Which case and what happened.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path { what, detail } => write!(f, "task-25: cannot read {what}: {detail}"),
            Self::Peer { detail } => write!(f, "task-25: scripted peer failed: {detail}"),
            Self::Transport { detail } => write!(f, "task-25: transport error: {detail}"),
        }
    }
}

impl std::error::Error for DriverError {}

// ---------------------------------------------------------------------------
// Recon: re-verify the seam premises against the live sources on every run
// ---------------------------------------------------------------------------

fn workspace_root() -> Result<PathBuf, DriverError> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .ok_or_else(|| DriverError::Path {
            what: "workspace root".to_string(),
            detail: "CARGO_MANIFEST_DIR has fewer than 2 ancestors".to_string(),
        })
}

fn read_file(path: &Path, what: &str) -> Result<String, DriverError> {
    std::fs::read_to_string(path).map_err(|e| DriverError::Path {
        what: what.to_string(),
        detail: format!("{}: {e}", path.display()),
    })
}

/// Verify the seam premises against the live sources. Fails closed if the
/// transport's backpressure shape changes.
fn recon() -> Result<Vec<String>, DriverError> {
    let src = workspace_root()?
        .join("crates")
        .join("phlow-runtime")
        .join("src")
        .join("transport")
        .join("msgpack.rs");
    let text = read_file(&src, "phlow-runtime transport/msgpack.rs")?;
    for needle in [
        "WORKER_QUEUE_CAPACITY",
        "mpsc::channel(WORKER_QUEUE_CAPACITY)",
        "tx.send(request).await",
    ] {
        if !text.contains(needle) {
            return Err(DriverError::Path {
                what: "backpressure premise".to_string(),
                detail: format!("msgpack.rs no longer contains '{needle}'"),
            });
        }
    }
    if !text.contains("const WORKER_QUEUE_CAPACITY: usize = 1;") {
        return Err(DriverError::Path {
            what: "backpressure premise".to_string(),
            detail: "WORKER_QUEUE_CAPACITY is no longer exactly 1".to_string(),
        });
    }
    Ok(vec![
        "recon: phlow-runtime/src/transport/msgpack.rs carries the only producer/consumer channel in the codebase".to_string(),
        "recon: mpsc::channel(WORKER_QUEUE_CAPACITY) with WORKER_QUEUE_CAPACITY = 1 (verified present)".to_string(),
        "recon: exec takes &mut self — at most one request in flight per transport; the channel cannot back up".to_string(),
        "recon: producer parks in tx.send(request).await / reply-await; per-request timeout fails explicitly (TransportError::Timeout)".to_string(),
        "scope: no event bus exists between orchestration stages; the experiment Scheduler queue is admission-only (no consumer); the tuios permit pool throttles TCP connections, not stage messages".to_string(),
    ])
}

// ---------------------------------------------------------------------------
// Scripted msgpack-RPC peer (rmpv — the same codec the transport uses)
// ---------------------------------------------------------------------------

/// How the scripted peer treats each request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeerBehavior {
    /// Reply immediately with the canned result.
    Immediate,
    /// Wait N seconds, then reply.
    DelaySecs(u64),
    /// Read requests but never reply (the black hole).
    BlackHole,
}

/// Try to decode one msgpack value at the head of `buf`. Returns the
/// consumed byte count, or `None` when the buffer does not yet hold a
/// complete frame (the caller keeps reading; the buffer cap fails closed).
fn try_decode_frame(buf: &[u8]) -> Option<usize> {
    let mut slice: &[u8] = buf;
    match rmpv::decode::read_value(&mut slice) {
        Ok(_) => Some(buf.len() - slice.len()),
        Err(_) => None,
    }
}

/// Extract the msgpack-RPC message id from a decoded request frame.
fn frame_id(frame: &[u8]) -> Option<u32> {
    let value = rmpv::decode::read_value(&mut &frame[..]).ok()?;
    let parts = value.as_array()?;
    if parts.len() != 4 {
        return None;
    }
    parts[1].as_u64().map(|id| id as u32)
}

/// Serve one accepted connection: read request frames, answer per behavior.
fn serve_connection(
    mut stream: std::os::unix::net::UnixStream,
    behavior: PeerBehavior,
    shutdown: &AtomicBool,
) {
    let _ = stream.set_read_timeout(Some(PEER_READ_TIMEOUT));
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 4096];
    let mut seq: u64 = 0;
    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.len() > PEER_BUFFER_BYTES_MAX {
                    break;
                }
                while let Some(consumed) = try_decode_frame(&buf) {
                    let id = frame_id(&buf[..consumed]);
                    buf.drain(..consumed);
                    let Some(id) = id else { break };
                    match behavior {
                        PeerBehavior::BlackHole => {}
                        PeerBehavior::Immediate => reply(&mut stream, id, seq),
                        PeerBehavior::DelaySecs(s) => {
                            thread::sleep(Duration::from_secs(s));
                            reply(&mut stream, id, seq);
                        }
                    }
                    seq += 1;
                }
            }
            Err(e) if e.kind() == ErrorKind::TimedOut || e.kind() == ErrorKind::WouldBlock => {
                continue;
            }
            Err(_) => break,
        }
    }
}

/// Write one `[1, id, nil, {ok: true, seq}]` response frame.
fn reply(stream: &mut std::os::unix::net::UnixStream, id: u32, seq: u64) {
    let response = rmpv::Value::Array(vec![
        rmpv::Value::from(1),
        rmpv::Value::from(id),
        rmpv::Value::Nil,
        rmpv::Value::Map(vec![
            (rmpv::Value::from("ok"), rmpv::Value::Boolean(true)),
            (rmpv::Value::from("seq"), rmpv::Value::from(seq)),
        ]),
    ]);
    let mut bytes = Vec::new();
    if rmpv::encode::write_value(&mut bytes, &response).is_ok() {
        let _ = stream.write_all(&bytes);
    }
}

/// A scripted peer listening on a Unix socket.
struct FakePeer {
    path: PathBuf,
    shutdown: Arc<AtomicBool>,
    accept_handle: Option<thread::JoinHandle<()>>,
}

impl FakePeer {
    /// Bind `path` and start serving with `behavior`.
    fn start(path: &Path, behavior: PeerBehavior) -> Result<Self, DriverError> {
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| DriverError::Peer {
                detail: format!("cannot remove stale socket {}: {e}", path.display()),
            })?;
        }
        let listener = UnixListener::bind(path).map_err(|e| DriverError::Peer {
            detail: format!("bind {}: {e}", path.display()),
        })?;
        listener
            .set_nonblocking(true)
            .map_err(|e| DriverError::Peer {
                detail: format!("set_nonblocking: {e}"),
            })?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&shutdown);
        let accept_handle = thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let f = Arc::clone(&flag);
                        thread::spawn(move || serve_connection(stream, behavior, &f));
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(PEER_POLL_MS));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            path: path.to_path_buf(),
            shutdown,
            accept_handle: Some(accept_handle),
        })
    }

    /// Signal shutdown, join the accept loop, remove the socket path.
    /// Connection handlers are detached; they observe the flag within
    /// `PEER_READ_TIMEOUT` and exit on their own.
    fn stop(mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.accept_handle.take() {
            let _ = handle.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Resident set size of this process, in MiB. `None` when /proc is
/// unavailable; reported honestly, never invented.
fn rss_mib() -> Option<f64> {
    let text = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = text.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages as f64 * 4.0 / 1024.0)
}

fn socket_path(dir: &Path) -> PathBuf {
    dir.join("peer.sock")
}

fn transport_at(path: &Path) -> Result<MsgpackTransport, DriverError> {
    let path_str = path.to_str().ok_or_else(|| DriverError::Path {
        what: "socket path".to_string(),
        detail: "socket path is not UTF-8".to_string(),
    })?;
    Ok(MsgpackTransport::new(path_str))
}

/// Assert an exec result is the peer's canned `{"ok": true}` reply.
fn expect_ok(
    result: Result<serde_json::Value, TransportError>,
    what: &str,
) -> Result<(), DriverError> {
    match result {
        Ok(value) => {
            if value.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                Ok(())
            } else {
                Err(DriverError::Transport {
                    detail: format!("{what}: unexpected reply {value}"),
                })
            }
        }
        Err(e) => Err(DriverError::Transport {
            detail: format!("{what}: unexpected transport error: {e}"),
        }),
    }
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// The parsed verdict of one case.
#[derive(Debug, Clone)]
pub struct CaseReport {
    /// Which case ran.
    pub case: String,
    /// Whether the case's own assertions held.
    pub passed: bool,
    /// Measured numbers (latencies, counts, RSS).
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

/// V1: matched rates — an immediate peer serves sequential execs; every
/// call succeeds fast with the exact canned reply.
fn case_matched_rates(dir: &Path) -> Result<CaseReport, DriverError> {
    const CASE: &str = "matched_rates";
    let mut evidence = Vec::new();
    std::fs::create_dir_all(dir).map_err(|e| DriverError::Path {
        what: "case dir".to_string(),
        detail: format!("{}: {e}", dir.display()),
    })?;
    let peer = FakePeer::start(&socket_path(dir), PeerBehavior::Immediate)?;
    let mut transport = transport_at(&socket_path(dir))?;
    let mut worst_ms: u128 = 0;
    for i in 0..5 {
        let started = Instant::now();
        let result = transport.exec(SCHEMAS_LUA, &[], Duration::from_secs(10));
        let elapsed_ms = started.elapsed().as_millis();
        worst_ms = worst_ms.max(elapsed_ms);
        if let Err(e) = expect_ok(result, &format!("exec {i}")) {
            peer.stop();
            return Err(e);
        }
    }
    peer.stop();
    evidence.push(format!(
        "5 sequential execs against an immediate peer: all Ok, worst latency {worst_ms} ms"
    ));
    if worst_ms > 5000 {
        return Ok(CaseReport::fail(
            CASE,
            format!("worst latency {worst_ms} ms exceeds 5000 ms"),
            evidence,
        ));
    }
    evidence.push("producer and consumer at matched rates: no parking, no errors".to_string());
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"execs": 5, "worst_latency_ms": worst_ms}),
        evidence,
    ))
}

/// V2: slow consumer — the peer delays each reply 2s; the producer must
/// park (elapsed >= delay) rather than fail, and the reply stays intact.
fn case_slow_consumer_parks(dir: &Path) -> Result<CaseReport, DriverError> {
    const CASE: &str = "slow_consumer_parks";
    let mut evidence = Vec::new();
    std::fs::create_dir_all(dir).map_err(|e| DriverError::Path {
        what: "case dir".to_string(),
        detail: format!("{}: {e}", dir.display()),
    })?;
    let peer = FakePeer::start(&socket_path(dir), PeerBehavior::DelaySecs(SLOW_DELAY_SECS))?;
    let mut transport = transport_at(&socket_path(dir))?;
    let started = Instant::now();
    let result = transport.exec(SCHEMAS_LUA, &[], Duration::from_secs(30));
    let elapsed = started.elapsed();
    peer.stop();
    if let Err(e) = expect_ok(result, "slow exec") {
        return Ok(CaseReport::fail(CASE, e.to_string(), evidence));
    }
    let elapsed_secs = elapsed.as_secs_f64();
    evidence.push(format!(
        "peer delayed the reply {SLOW_DELAY_SECS}s; producer parked {elapsed_secs:.2}s then got the intact reply"
    ));
    if elapsed_secs < SLOW_DELAY_SECS as f64 {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "producer returned in {elapsed_secs:.2}s without waiting out the {SLOW_DELAY_SECS}s consumer delay"
            ),
            evidence,
        ));
    }
    if elapsed_secs > 25.0 {
        return Ok(CaseReport::fail(
            CASE,
            format!("producer took {elapsed_secs:.2}s for a {SLOW_DELAY_SECS}s delay"),
            evidence,
        ));
    }
    evidence.push(
        "producer throughput tracks the consumer: it blocked instead of failing or buffering"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"delay_secs": SLOW_DELAY_SECS, "parked_secs": elapsed_secs}),
        evidence,
    ))
}

/// A1: the consumer stalls completely for 30s with several producers in
/// flight. Every producer must park, memory must stay flat (bounded by the
/// explicit capacity-1 channel), and every message must arrive intact when
/// the consumer resumes — no loss, no OOM.
fn case_stall_30s_resumes_cleanly(dir: &Path) -> Result<CaseReport, DriverError> {
    const CASE: &str = "stall_30s_resumes_cleanly";
    let mut evidence = Vec::new();
    std::fs::create_dir_all(dir).map_err(|e| DriverError::Path {
        what: "case dir".to_string(),
        detail: format!("{}: {e}", dir.display()),
    })?;
    let peer = FakePeer::start(&socket_path(dir), PeerBehavior::DelaySecs(STALL_SECS))?;
    let path = socket_path(dir);
    let rss_before = rss_mib();
    let started = Instant::now();
    let mut handles = Vec::new();
    for producer in 0..STALL_PRODUCERS {
        let path = path.clone();
        handles.push(thread::spawn(move || {
            let mut transport = MsgpackTransport::new(path.to_str().expect("socket path is UTF-8"));
            let call_started = Instant::now();
            let result = transport.exec(SCHEMAS_LUA, &[], Duration::from_secs(90));
            (producer, call_started.elapsed(), result)
        }));
    }
    let mut outcomes = Vec::new();
    for handle in handles {
        outcomes.push(handle.join().map_err(|_| DriverError::Peer {
            detail: "producer thread panicked".to_string(),
        })?);
    }
    let total_elapsed = started.elapsed();
    let rss_after = rss_mib();
    peer.stop();
    let mut failures = Vec::new();
    for (producer, elapsed, result) in &outcomes {
        if let Err(e) = expect_ok(result.clone(), &format!("producer {producer}")) {
            failures.push(e.to_string());
        }
        if elapsed.as_secs_f64() < STALL_SECS as f64 {
            failures.push(format!(
                "producer {producer} returned in {:.2}s without riding out the {STALL_SECS}s stall",
                elapsed.as_secs_f64()
            ));
        }
    }
    let total_secs = total_elapsed.as_secs_f64();
    evidence.push(format!(
        "{STALL_PRODUCERS} producers stalled {STALL_SECS}s; all parked, total wall {total_secs:.1}s"
    ));
    match (rss_before, rss_after) {
        (Some(before), Some(after)) => {
            let growth = after - before;
            evidence.push(format!(
                "RSS {before:.1} MiB -> {after:.1} MiB across the stall (growth {growth:.1} MiB)"
            ));
            if growth > RSS_GROWTH_MIB_MAX {
                failures.push(format!(
                    "RSS grew {growth:.1} MiB during the stall (bound {RSS_GROWTH_MIB_MAX})"
                ));
            } else {
                evidence.push(
                    "memory flat: the capacity-1 channel bounds buffering even with producers parked"
                        .to_string(),
                );
            }
        }
        _ => evidence
            .push("RSS unavailable (/proc/self/statm unreadable); memory not measured".to_string()),
    }
    if total_secs > 75.0 {
        failures.push(format!(
            "total wall {total_secs:.1}s far exceeds the {STALL_SECS}s stall"
        ));
    }
    if failures.is_empty() {
        evidence.push(format!(
            "no messages lost: all {STALL_PRODUCERS} parked producers received their intact replies when the consumer resumed"
        ));
        Ok(CaseReport::pass(
            CASE,
            serde_json::json!({
                "producers": STALL_PRODUCERS,
                "stall_secs": STALL_SECS,
                "total_wall_secs": total_secs,
                "rss_growth_mib": rss_after.unwrap_or(-1.0) - rss_before.unwrap_or(0.0),
            }),
            evidence,
        ))
    } else {
        Ok(CaseReport {
            case: CASE.to_string(),
            passed: false,
            metrics: serde_json::Value::Null,
            evidence,
            failures,
        })
    }
}

/// A2: the consumer never answers (black hole). The producer must fail
/// EXPLICITLY with `TransportError::Timeout` — promptly, not a hang — and
/// the same transport must recover by reconnecting once a healthy peer is
/// back on the socket path.
fn case_timeout_is_explicit(dir: &Path) -> Result<CaseReport, DriverError> {
    const CASE: &str = "timeout_is_explicit";
    let mut evidence = Vec::new();
    std::fs::create_dir_all(dir).map_err(|e| DriverError::Path {
        what: "case dir".to_string(),
        detail: format!("{}: {e}", dir.display()),
    })?;
    let path = socket_path(dir);
    let blackhole = FakePeer::start(&path, PeerBehavior::BlackHole)?;
    let mut transport = transport_at(&path)?;
    let started = Instant::now();
    let result = transport.exec(SCHEMAS_LUA, &[], Duration::from_secs(2));
    let elapsed = started.elapsed();
    let elapsed_secs = elapsed.as_secs_f64();
    match result {
        Err(TransportError::Timeout) => {
            evidence.push(format!(
                "black-hole peer: exec failed explicitly with TransportError::Timeout after {elapsed_secs:.2}s"
            ));
        }
        Err(other) => {
            blackhole.stop();
            return Ok(CaseReport::fail(
                CASE,
                format!("black-hole peer gave {other}, want TransportError::Timeout"),
                evidence,
            ));
        }
        Ok(value) => {
            blackhole.stop();
            return Ok(CaseReport::fail(
                CASE,
                format!("black-hole peer unexpectedly succeeded: {value}"),
                evidence,
            ));
        }
    }
    if !(2.0..=10.0).contains(&elapsed_secs) {
        blackhole.stop();
        return Ok(CaseReport::fail(
            CASE,
            format!("timeout took {elapsed_secs:.2}s; want prompt failure near the 2s deadline"),
            evidence,
        ));
    }
    evidence
        .push("no hang: the timeout fired at the deadline instead of parking forever".to_string());
    // Recovery: the timed-out worker dropped its half-dead stream; a
    // healthy peer on the same path must serve the next exec via reconnect.
    blackhole.stop();
    let healthy = FakePeer::start(&path, PeerBehavior::Immediate)?;
    let started = Instant::now();
    let result = transport.exec(SCHEMAS_LUA, &[], Duration::from_secs(10));
    let recover_secs = started.elapsed().as_secs_f64();
    healthy.stop();
    if let Err(e) = expect_ok(result, "post-timeout exec") {
        return Ok(CaseReport::fail(
            CASE,
            format!("transport did not recover after the timeout: {e}"),
            evidence,
        ));
    }
    evidence.push(format!(
        "same transport, healthy peer: exec succeeded in {recover_secs:.2}s — the worker reconnected fresh after dropping the stalled stream"
    ));
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"timeout_secs": elapsed_secs, "recover_secs": recover_secs}),
        evidence,
    ))
}

/// Run one case by name in its own subdirectory of `socket_dir`.
pub fn run_case(socket_dir: &Path, case: &str) -> Result<CaseReport, DriverError> {
    let dir = socket_dir.join(case);
    match case {
        "matched_rates" => case_matched_rates(&dir),
        "slow_consumer_parks" => case_slow_consumer_parks(&dir),
        "stall_30s_resumes_cleanly" => case_stall_30s_resumes_cleanly(&dir),
        "timeout_is_explicit" => case_timeout_is_explicit(&dir),
        _ => Err(DriverError::Peer {
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

fn run_inner(ctx: &Ctx) -> Result<Vec<String>, TaskFailure> {
    let mut evidence = recon().map_err(|e| TaskFailure {
        where_: "recon".to_string(),
        how: e.to_string(),
        evidence: Vec::new(),
    })?;
    let socket_dir = ctx.work_dir.join("task-25");
    for case in CASES {
        let report = run_case(&socket_dir, case).map_err(|e| TaskFailure {
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
    Ok(evidence)
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
