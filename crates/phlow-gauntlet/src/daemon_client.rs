// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Daemon-link client: the `phlow-cli` ↔ daemon link's client half.
//!
//! Adapted concepts (re-implemented, not ported) from Ghostex
//! `packages/gx-client/src/{client.rs,worker.rs,socket.rs}` @ c911466:
//! a worker running "connect, subscribe, read, reconnect" with a
//! `RECONNECT_LADDER_MS` backoff ladder, subscribe-on-reconnect from
//! client-held subscription state, and snapshot resync after reconnect.
//!
//! The module is split into two layers on purpose:
//!
//! - **Deterministic core** ([`ReconnectEngine`], [`SubscriptionSet`],
//!   [`ClientState`]): pure state machines driven by an explicit
//!   millisecond timeline. The gauntlet drivers (tasks 159–162) drive
//!   these directly against a scripted [`ManualClock`]; nothing here
//!   reads wall time or spawns threads.
//! - **Threaded worker** ([`DaemonClient`]): owns one worker thread
//!   running the connect/subscribe/read/reconnect loop over a [`Link`],
//!   with bounded shutdown ([`SHUTDOWN_TIMEOUT`]) and per-message
//!   drain/drop dispositions. Used by task 165.
//! - **Daemon fixture** ([`DaemonFixture`]): a scripted loopback-TCP
//!   daemon implementing the accept path under test for tasks 163–164:
//!   credential gate, snapshot staleness check, per-connection read
//!   deadline. Real sockets, real fds, in-process.
//!
//! Deliberate deviation from Ghostex, documented because task 162 A2
//! demands it: Ghostex resets its ladder whenever a dropped stream had
//! been acknowledged for at least `HEALTHY_STREAM_DURATION`, however it
//! ended. We reset only on an **orderly** close after a healthy stream.
//! An abrupt drop — even one landing exactly on the healthy boundary —
//! is always a failure for ladder purposes, so a flapping daemon cannot
//! collapse the backoff to the floor interval by syncing drops to the
//! reset point.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Named bounds
// ---------------------------------------------------------------------------

/// Backoff between reconnect attempts after the n-th consecutive failure
/// (1-based); the last rung repeats. Milliseconds. Values are ours; the
/// ladder shape (escalating, capped, reset only on health) is the Ghostex
/// concept.
pub const RECONNECT_LADDER_MS: [u64; 6] = [100, 500, 2000, 4000, 8000, 16000];

/// Consecutive failures after which the worker parks instead of
/// retrying. Bounds the reconnect loop under a flapping daemon.
pub const MAX_RECONNECT_ATTEMPTS: u32 = 10;

/// A stream counts as healthy (ladder-reset eligible) only after being
/// acknowledged for at least this long *and* closing orderly.
pub const HEALTHY_STREAM_MS: u64 = 30_000;

/// How long one daemon-socket frame read may block. Bounds slow-loris
/// slot holds on the accept path.
pub const FRAME_READ_TIMEOUT: Duration = Duration::from_millis(250);

/// Upper bound for [`DaemonClient::shutdown`]: join the worker, drain or
/// drop in-flight messages, release everything.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// One in-flight message gets a single write attempt bounded by this.
/// Keeps the shutdown drain from wedging on an unresponsive daemon.
pub const WRITE_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(10);

/// Largest single frame the fixture daemon will read. Rejects
/// length-prefix lies before any allocation past this.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

/// Cap on distinct subscribed topics: the subscription table is client
/// state and must stay bounded.
pub const MAX_SUBSCRIPTIONS: usize = 256;

/// Cap on host→daemon messages queued while the link is down or slow.
pub const SEND_QUEUE_CAP: usize = 1024;

/// Worker thread name (truncated to 15 chars in `/proc/self/task/*/comm`).
pub const WORKER_THREAD_NAME: &str = "phlow-daemon-link";

/// How long the worker's read and backoff waits block before re-checking
/// for commands/shutdown. Keeps shutdown latency bounded without spinning.
const WAIT_POLL: Duration = Duration::from_millis(50);
/// Accept-loop poll quantum for the fixture daemon's nonblocking listener.
const ACCEPT_POLL: Duration = Duration::from_millis(20);

/// Wall-clock milliseconds. Used only by the threaded worker and the
/// fixture daemon; the deterministic core takes `now_ms` from the driver.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn lock_mutex<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------
// ReconnectEngine — the deterministic backoff core
// ---------------------------------------------------------------------------

/// Why the engine stopped scheduling attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkReason {
    /// [`MAX_RECONNECT_ATTEMPTS`] consecutive failures: parked, no busy loop.
    BackoffExhausted,
}

/// Deterministic reconnect-ladder state machine. The driver supplies
/// `now_ms` (scripted); the engine never reads a clock itself, so a
/// parked engine performs zero wakeups by construction — there is no
/// deadline to poll.
#[derive(Debug)]
pub struct ReconnectEngine {
    consecutive_failures: u32,
    parked: Option<ParkReason>,
    next_attempt_at_ms: Option<u64>,
    attempt_times_ms: Vec<u64>,
}

impl ReconnectEngine {
    /// First attempt is due immediately at t=0 of the driver's timeline.
    pub fn new() -> Self {
        Self {
            consecutive_failures: 0,
            parked: None,
            next_attempt_at_ms: Some(0),
            attempt_times_ms: Vec::new(),
        }
    }

    /// Ladder delay after `failures` consecutive failures (1-based).
    /// The last rung repeats, so the interval never drops below the floor
    /// once escalated.
    pub fn delay_for_failures(failures: u32) -> u64 {
        let idx = (failures.saturating_sub(1) as usize).min(RECONNECT_LADDER_MS.len() - 1);
        RECONNECT_LADDER_MS[idx]
    }

    /// Record a connection attempt at `now_ms`. Returns false when parked:
    /// the driver must not attempt.
    pub fn note_attempt(&mut self, now_ms: u64) -> bool {
        if self.parked.is_some() {
            return false;
        }
        self.attempt_times_ms.push(now_ms);
        self.next_attempt_at_ms = None;
        true
    }

    /// The most recent attempt has ended. `healthy_orderly` must be true
    /// only when the stream was acknowledged, lived at least
    /// [`HEALTHY_STREAM_MS`], and closed orderly — an abrupt drop is
    /// always a failure, even on the healthy boundary (see module docs).
    pub fn note_disconnect(&mut self, now_ms: u64, healthy_orderly: bool) {
        if self.parked.is_some() {
            return;
        }
        if healthy_orderly {
            self.consecutive_failures = 0;
        } else {
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        }
        if self.consecutive_failures >= MAX_RECONNECT_ATTEMPTS {
            self.parked = Some(ParkReason::BackoffExhausted);
            self.next_attempt_at_ms = None;
        } else {
            let delay = Self::delay_for_failures(self.consecutive_failures.max(1));
            self.next_attempt_at_ms = Some(now_ms.saturating_add(delay));
        }
    }

    /// Absolute ms time of the next scheduled attempt; `None` when parked.
    /// A deadline-driven waiter sleeps until this — no spin.
    pub fn next_attempt_at_ms(&self) -> Option<u64> {
        self.next_attempt_at_ms
    }

    /// Why the engine parked, if it did.
    pub fn parked(&self) -> Option<ParkReason> {
        self.parked
    }

    /// Consecutive-failure count driving the ladder.
    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// Ledger of attempt timestamps, for assertions.
    pub fn attempt_times_ms(&self) -> &[u64] {
        &self.attempt_times_ms
    }
}

impl Default for ReconnectEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// SubscriptionSet — subscriptions are client state, not connection state
// ---------------------------------------------------------------------------

/// The client's topic set. Survives reconnects; the worker re-sends the
/// whole set after every reconnect (subscribe-on-reconnect).
#[derive(Debug, Default)]
pub struct SubscriptionSet {
    topics: BTreeSet<String>,
}

impl SubscriptionSet {
    /// Empty set.
    pub fn new() -> Self {
        Self {
            topics: BTreeSet::new(),
        }
    }

    /// Add a topic. Returns true when newly added; false when already
    /// present or the [`MAX_SUBSCRIPTIONS`] bound is hit.
    pub fn subscribe(&mut self, topic: &str) -> bool {
        if self.topics.contains(topic) {
            return false;
        }
        if self.topics.len() >= MAX_SUBSCRIPTIONS {
            return false;
        }
        self.topics.insert(topic.to_string())
    }

    /// Remove a topic. Returns true when it was present.
    pub fn unsubscribe(&mut self, topic: &str) -> bool {
        self.topics.remove(topic)
    }

    /// Topics in sorted order — the deterministic resubscribe order.
    pub fn topics(&self) -> Vec<String> {
        self.topics.iter().cloned().collect()
    }

    /// Number of subscribed topics.
    pub fn len(&self) -> usize {
        self.topics.len()
    }

    /// True when no topics are subscribed.
    pub fn is_empty(&self) -> bool {
        self.topics.is_empty()
    }

    /// True when the topic is in the set.
    pub fn contains(&self, topic: &str) -> bool {
        self.topics.contains(topic)
    }
}

// ---------------------------------------------------------------------------
// Snapshot resync — converge client state after missed events
// ---------------------------------------------------------------------------

/// One daemon state-change event on the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonEvent {
    /// Monotonic per-daemon sequence number.
    pub seq: u64,
    pub key: String,
    pub value: String,
}

/// A full state snapshot. `as_of` is the last event sequence number
/// included in `state`: the client's resync marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub version: u64,
    pub as_of: u64,
    pub state: BTreeMap<String, String>,
}

/// Typed resync failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResyncError {
    /// The snapshot predates what the client already applied: a replay.
    StaleSnapshot { as_of: u64, applied_through: u64 },
    /// An event arrived that is neither the next one nor a duplicate.
    Gap { want_seq: u64, got_seq: u64 },
}

/// Client-side stream state with snapshot resync. While a snapshot is in
/// flight, live events are buffered; when the snapshot lands they are
/// applied in sequence order after the `as_of` marker — no gaps, no
/// double-applies.
#[derive(Debug, Default)]
pub struct ClientState {
    applied_through: u64,
    version: u64,
    state: BTreeMap<String, String>,
    awaiting_snapshot: bool,
    buffer: Vec<DaemonEvent>,
    ledger: Vec<(u64, &'static str)>,
}

impl ClientState {
    /// Fresh client: nothing applied.
    pub fn new() -> Self {
        Self::default()
    }

    /// The connection dropped; the next snapshot will resync. Live events
    /// arriving before it are buffered, not applied.
    pub fn begin_resync(&mut self) {
        self.awaiting_snapshot = true;
        self.buffer.clear();
    }

    /// Apply one event to the state map.
    fn apply(&mut self, ev: &DaemonEvent, label: &'static str) {
        self.state.insert(ev.key.clone(), ev.value.clone());
        self.applied_through = ev.seq;
        self.ledger.push((ev.seq, label));
    }

    /// Apply the snapshot, then the buffered live events in sequence
    /// order. Events at or below `as_of` are duplicates of snapshot
    /// content and are dropped; anything past `as_of + 1` with a hole is
    /// a [`ResyncError::Gap`].
    pub fn on_snapshot(&mut self, snap: &Snapshot) -> Result<(), ResyncError> {
        if snap.as_of < self.applied_through {
            return Err(ResyncError::StaleSnapshot {
                as_of: snap.as_of,
                applied_through: self.applied_through,
            });
        }
        self.state = snap.state.clone();
        self.version = snap.version;
        self.applied_through = snap.as_of;
        self.ledger.push((snap.as_of, "snapshot"));
        let mut buffered = std::mem::take(&mut self.buffer);
        buffered.sort_by_key(|e| e.seq);
        for ev in &buffered {
            if ev.seq <= self.applied_through {
                self.ledger.push((ev.seq, "dup-dropped"));
            } else if ev.seq == self.applied_through + 1 {
                self.apply(ev, "applied");
            } else {
                let got = ev.seq;
                let want = self.applied_through + 1;
                self.awaiting_snapshot = false;
                return Err(ResyncError::Gap {
                    want_seq: want,
                    got_seq: got,
                });
            }
        }
        self.awaiting_snapshot = false;
        Ok(())
    }

    /// Handle a live event: buffered while a snapshot is in flight,
    /// duplicate-dropped when already applied, gap-error otherwise.
    pub fn on_live_event(&mut self, ev: &DaemonEvent) -> Result<(), ResyncError> {
        if self.awaiting_snapshot {
            self.buffer.push(ev.clone());
            self.ledger.push((ev.seq, "buffered"));
            return Ok(());
        }
        if ev.seq <= self.applied_through {
            self.ledger.push((ev.seq, "dup-dropped"));
            return Ok(());
        }
        if ev.seq == self.applied_through + 1 {
            self.apply(ev, "applied");
            return Ok(());
        }
        Err(ResyncError::Gap {
            want_seq: self.applied_through + 1,
            got_seq: ev.seq,
        })
    }

    /// Current converged state.
    pub fn state(&self) -> &BTreeMap<String, String> {
        &self.state
    }

    /// Last event sequence number applied.
    pub fn applied_through(&self) -> u64 {
        self.applied_through
    }

    /// Snapshot version applied.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// True while a snapshot is in flight.
    pub fn awaiting_snapshot(&self) -> bool {
        self.awaiting_snapshot
    }

    /// Ordered (seq, disposition) record for gap/dupe assertions.
    pub fn ledger(&self) -> &[(u64, &'static str)] {
        &self.ledger
    }
}

// ---------------------------------------------------------------------------
// Link — the transport the worker runs over
// ---------------------------------------------------------------------------

/// Typed link failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkError {
    /// `connect` refused / daemon unreachable.
    ConnectFailed,
    /// An established link dropped.
    Dropped,
    /// A bounded write did not complete in time.
    WriteTimeout,
}

/// The worker's transport. Scripted doubles implement this in-process;
/// no real sockets are needed for the protocol half.
pub trait Link: Send {
    /// Open the link. May fail per the double's script.
    fn connect(&mut self) -> Result<(), LinkError>;
    /// Write one frame, bounded by `timeout`.
    fn write_frame(&mut self, frame: &str, timeout: Duration) -> Result<(), LinkError>;
    /// Read one frame, waiting at most `timeout`. `Ok(None)` is a quiet
    /// timeout, not a drop.
    fn read_frame(&mut self, timeout: Duration) -> Result<Option<String>, LinkError>;
    /// Release the link exactly once per successful connect.
    fn close(&mut self);
    /// Currently open link handles (0 or 1): the leak census.
    fn open_handles(&self) -> usize;
}

/// What a scripted read returns.
#[derive(Debug, Clone)]
pub enum ScriptedRead {
    /// A frame arrived.
    Frame(String),
    /// Quiet timeout: the link is idle, not dead.
    Timeout,
    /// The daemon dropped the link.
    Dropped,
}

/// In-process scripted [`Link`] double. Connect outcomes, read outcomes,
/// and write behavior are scripted; everything is recorded for assertions.
#[derive(Debug)]
pub struct ScriptedLink {
    connect_script: VecDeque<bool>,
    connect_default: bool,
    read_script: VecDeque<ScriptedRead>,
    read_default: ScriptedRead,
    write_ok: bool,
    outbound: Vec<String>,
    /// Optional shared probe: every written frame is also pushed here so
    /// a test can observe a link that was moved into a worker thread.
    probe: Option<Arc<Mutex<Vec<String>>>>,
    open: bool,
    connects: usize,
    closes: usize,
}

impl ScriptedLink {
    /// New double. `connect_default`/`read_default` apply once the
    /// scripts are exhausted; `write_ok = false` models an unresponsive
    /// daemon (every bounded write times out).
    pub fn new(connect_default: bool, read_default: ScriptedRead, write_ok: bool) -> Self {
        Self {
            connect_script: VecDeque::new(),
            connect_default,
            read_script: VecDeque::new(),
            read_default,
            write_ok,
            outbound: Vec::new(),
            probe: None,
            open: false,
            connects: 0,
            closes: 0,
        }
    }

    /// Like [`Self::new`], plus a shared outbound probe for links moved
    /// into a worker thread.
    pub fn with_probe(
        connect_default: bool,
        read_default: ScriptedRead,
        write_ok: bool,
        probe: Arc<Mutex<Vec<String>>>,
    ) -> Self {
        let mut link = Self::new(connect_default, read_default, write_ok);
        link.probe = Some(probe);
        link
    }

    /// Queue connect outcomes (`true` = success). Appended in order.
    pub fn script_connects(&mut self, outcomes: &[bool]) {
        self.connect_script.extend(outcomes.iter().copied());
    }

    /// Queue read outcomes. Appended in order.
    pub fn script_reads(&mut self, reads: Vec<ScriptedRead>) {
        self.read_script.extend(reads);
    }

    /// Frames the worker wrote, in order.
    pub fn outbound(&self) -> &[String] {
        &self.outbound
    }

    /// Successful connects so far.
    pub fn connect_count(&self) -> usize {
        self.connects
    }

    /// `close` calls so far.
    pub fn close_count(&self) -> usize {
        self.closes
    }
}

impl Link for ScriptedLink {
    fn connect(&mut self) -> Result<(), LinkError> {
        let ok = self
            .connect_script
            .pop_front()
            .unwrap_or(self.connect_default);
        if ok {
            self.open = true;
            self.connects += 1;
            Ok(())
        } else {
            Err(LinkError::ConnectFailed)
        }
    }

    fn write_frame(&mut self, frame: &str, _timeout: Duration) -> Result<(), LinkError> {
        if !self.write_ok {
            return Err(LinkError::WriteTimeout);
        }
        self.outbound.push(frame.to_string());
        if let Some(probe) = &self.probe {
            lock_mutex(probe).push(frame.to_string());
        }
        Ok(())
    }

    fn read_frame(&mut self, _timeout: Duration) -> Result<Option<String>, LinkError> {
        match self
            .read_script
            .pop_front()
            .unwrap_or_else(|| self.read_default.clone())
        {
            ScriptedRead::Frame(f) => Ok(Some(f)),
            ScriptedRead::Timeout => Ok(None),
            ScriptedRead::Dropped => Err(LinkError::Dropped),
        }
    }

    fn close(&mut self) {
        if self.open {
            self.open = false;
            self.closes += 1;
        }
    }

    fn open_handles(&self) -> usize {
        usize::from(self.open)
    }
}

// ---------------------------------------------------------------------------
// Process census helpers (Linux)
// ---------------------------------------------------------------------------

/// Open fd count for this process, via `/proc/self/fd`.
pub fn fd_count() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .map(|r| r.count())
        .unwrap_or(0)
}

/// Thread names for this process, via `/proc/self/task/*/comm`
/// (kernel-truncated to 15 chars).
pub fn thread_names() -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(tasks) = std::fs::read_dir("/proc/self/task") {
        for task in tasks.flatten() {
            let comm = task.path().join("comm");
            if let Ok(text) = std::fs::read_to_string(&comm) {
                names.push(text.trim().to_string());
            }
        }
    }
    names
}

/// How many of this process's threads have `substr` in their name.
pub fn threads_named(substr: &str) -> usize {
    thread_names().iter().filter(|n| n.contains(substr)).count()
}

// ---------------------------------------------------------------------------
// DaemonFixture — scripted loopback daemon for the accept path (163, 164)
// ---------------------------------------------------------------------------

/// Shared mutable state behind the fixture's connection handlers.
struct DaemonShared {
    token: String,
    version: AtomicU64,
    as_of: AtomicU64,
    state: Mutex<BTreeMap<String, String>>,
    subscribes: Mutex<Vec<String>>,
    audit: Mutex<Vec<String>>,
    effects: Mutex<Vec<String>>,
    slots: AtomicUsize,
}

impl DaemonShared {
    fn audit(&self, line: String) {
        lock_mutex(&self.audit).push(line);
    }
}

/// Releases one accept-path slot when the handler returns, on every path.
struct SlotGuard<'a> {
    slots: &'a AtomicUsize,
}

impl<'a> SlotGuard<'a> {
    fn new(slots: &'a AtomicUsize) -> Self {
        slots.fetch_add(1, Ordering::AcqRel);
        Self { slots }
    }
}

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        self.slots.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A scripted daemon on loopback TCP: credential gate, snapshot
/// staleness check, per-connection read deadline. One frame per
/// connection keeps the fixture small; the tests open a fresh
/// connection per hostile probe.
pub struct DaemonFixture {
    shared: Arc<DaemonShared>,
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    accept_handle: Option<JoinHandle<()>>,
}

fn err_json(kind: &str) -> Vec<u8> {
    serde_json::json!({"err": kind}).to_string().into_bytes()
}

fn ok_json(kind: &str) -> Vec<u8> {
    serde_json::json!({"ok": kind}).to_string().into_bytes()
}

/// Read one frame from a peer: 4-byte big-endian length, then the body.
/// Any read stall trips [`FRAME_READ_TIMEOUT`]: the slow-loris defense.
fn serve_peer(stream: TcpStream, peer: SocketAddr, shared: &DaemonShared) {
    let _slot = SlotGuard::new(&shared.slots);
    shared.audit(format!("accept {peer}"));
    if stream.set_read_timeout(Some(FRAME_READ_TIMEOUT)).is_err() {
        return;
    }
    let mut header = [0u8; 4];
    if (&stream).read_exact(&mut header).is_err() {
        shared.audit(format!("read timeout from {peer}: slot released"));
        return;
    }
    let len = u32::from_be_bytes(header) as usize;
    if len == 0 || len > MAX_FRAME_BYTES {
        shared.audit(format!("bad frame length {len} from {peer}"));
        return;
    }
    let mut body = vec![0u8; len];
    if (&stream).read_exact(&mut body).is_err() {
        shared.audit(format!("read timeout (body) from {peer}: slot released"));
        return;
    }
    let reply = dispatch_frame(&body, peer, shared);
    let _ = write_frame_bytes(&stream, &reply);
}

fn write_frame_bytes(mut stream: &TcpStream, body: &[u8]) -> std::io::Result<()> {
    stream.write_all(&(body.len() as u32).to_be_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

/// The accept-path contract under test: authenticate first, then
/// authorize; staleness before application. Unauthenticated peers get
/// [`AuthError`](str) and a dropped connection; stale snapshots get a
/// typed rejection with daemon state untouched.
fn dispatch_frame(body: &[u8], peer: SocketAddr, shared: &DaemonShared) -> Vec<u8> {
    let frame: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return err_json("MalformedFrame"),
    };
    // Authenticate before parsing any op: loopback is not trust.
    let cred = frame.get("cred").and_then(|c| c.as_str()).unwrap_or("");
    if cred != shared.token {
        shared.audit(format!(
            "AuthError from {peer}: missing or wrong credential"
        ));
        return err_json("AuthError");
    }
    match frame.get("op").and_then(|o| o.as_str()) {
        Some("subscribe") => {
            if let Some(topic) = frame.get("topic").and_then(|t| t.as_str()) {
                lock_mutex(&shared.subscribes).push(topic.to_string());
            }
            ok_json("subscribed")
        }
        Some("ping") => ok_json("pong"),
        // Privileged ops: reachable only past the credential gate.
        Some("daemon-shutdown") => {
            lock_mutex(&shared.effects).push("daemon-shutdown".to_string());
            ok_json("ok")
        }
        Some("exec") => {
            let argv = frame.get("argv").map(|a| a.to_string()).unwrap_or_default();
            lock_mutex(&shared.effects).push(format!("exec:{argv}"));
            ok_json("ok")
        }
        Some("snapshot") => {
            let version = frame.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
            let as_of = frame.get("as_of").and_then(|v| v.as_u64()).unwrap_or(0);
            let cur_v = shared.version.load(Ordering::Acquire);
            let cur_a = shared.as_of.load(Ordering::Acquire);
            if version < cur_v || (version == cur_v && as_of < cur_a) {
                shared.audit(format!(
                    "StaleSnapshot from {peer}: version {version} as_of {as_of} \
                     behind current {cur_v}/{cur_a}; state unchanged"
                ));
                return serde_json::json!({
                    "err": "StaleSnapshot",
                    "current_version": cur_v,
                    "current_as_of": cur_a,
                })
                .to_string()
                .into_bytes();
            }
            if let Some(map) = frame.get("state").and_then(|s| s.as_object()) {
                let mut state = lock_mutex(&shared.state);
                state.clear();
                for (k, v) in map {
                    if let Some(s) = v.as_str() {
                        state.insert(k.clone(), s.to_string());
                    }
                }
            }
            shared.version.store(version, Ordering::Release);
            shared.as_of.store(as_of, Ordering::Release);
            ok_json("snapshot-applied")
        }
        _ => err_json("UnknownOp"),
    }
}

impl DaemonFixture {
    /// Start the fixture on 127.0.0.1 (ephemeral port) with the given
    /// credential token and epoch.
    pub fn start(token: &str, version: u64, as_of: u64) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(DaemonShared {
            token: token.to_string(),
            version: AtomicU64::new(version),
            as_of: AtomicU64::new(as_of),
            state: Mutex::new(BTreeMap::new()),
            subscribes: Mutex::new(Vec::new()),
            audit: Mutex::new(Vec::new()),
            effects: Mutex::new(Vec::new()),
            slots: AtomicUsize::new(0),
        });
        let shutdown = Arc::new(AtomicBool::new(false));
        let accept_shared = Arc::clone(&shared);
        let accept_shutdown = Arc::clone(&shutdown);
        let accept_handle = thread::Builder::new()
            .name("phlow-daemon-accept".to_string())
            .spawn(move || {
                while !accept_shutdown.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, peer)) => {
                            let peer_shared = Arc::clone(&accept_shared);
                            let _ = thread::Builder::new()
                                .name("phlow-daemon-conn".to_string())
                                .spawn(move || serve_peer(stream, peer, &peer_shared));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(ACCEPT_POLL);
                        }
                        Err(_) => thread::sleep(ACCEPT_POLL),
                    }
                }
            })?;
        Ok(Self {
            shared,
            addr,
            shutdown,
            accept_handle: Some(accept_handle),
        })
    }

    /// Where the fixture listens.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Currently held accept-path slots.
    pub fn slots(&self) -> usize {
        self.shared.slots.load(Ordering::Acquire)
    }

    /// Seed the daemon's state map (test setup).
    pub fn seed_state(&self, entries: &[(&str, &str)]) {
        let mut state = lock_mutex(&self.shared.state);
        for (k, v) in entries {
            state.insert((*k).to_string(), (*v).to_string());
        }
    }

    /// Copy of the daemon's state map.
    pub fn state_snapshot(&self) -> BTreeMap<String, String> {
        lock_mutex(&self.shared.state).clone()
    }

    /// Topics the fixture saw subscribed (authenticated).
    pub fn subscribes(&self) -> Vec<String> {
        lock_mutex(&self.shared.subscribes).clone()
    }

    /// Audit log lines (accepts, AuthErrors, timeouts, staleness).
    pub fn audit(&self) -> Vec<String> {
        lock_mutex(&self.shared.audit).clone()
    }

    /// Privileged effects that actually executed.
    pub fn effects(&self) -> Vec<String> {
        lock_mutex(&self.shared.effects).clone()
    }

    /// Stop the accept loop and join it. Connection handlers are
    /// short-lived by construction (read deadline); the tests assert
    /// they are gone via the thread census.
    pub fn stop(mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.accept_handle.take() {
            let _ = handle.join();
        }
    }
}

/// Test helper: open one connection, send one length-prefixed frame,
/// read one length-prefixed reply.
pub fn send_frame(addr: SocketAddr, payload: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut stream = TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write_frame_bytes(&stream, payload)?;
    let mut header = [0u8; 4];
    stream.read_exact(&mut header)?;
    let len = u32::from_be_bytes(header) as usize;
    let mut body = vec![0u8; len.min(MAX_FRAME_BYTES)];
    stream.read_exact(&mut body)?;
    Ok(body)
}

// ---------------------------------------------------------------------------
// DaemonClient — the threaded worker (task 165)
// ---------------------------------------------------------------------------

/// Host → worker commands.
enum WorkerCommand {
    Subscribe(String),
    Outbox(String),
    Shutdown,
}

/// How the read loop ended.
enum LoopEnd {
    Shutdown,
    Lost { healthy_orderly: bool },
}

/// The worker thread: connect, subscribe, read, reconnect — the Ghostex
/// `worker.rs` loop shape, re-implemented over [`Link`] with the
/// deterministic [`ReconnectEngine`].
pub struct DaemonClient {
    worker: Option<JoinHandle<()>>,
    tx: Sender<WorkerCommand>,
    shutdown: Arc<AtomicBool>,
    dispositions: Arc<Mutex<Vec<String>>>,
    fd_before: usize,
}

/// What `shutdown` did.
#[derive(Debug)]
pub struct ShutdownReport {
    /// The worker thread was joined (never detached, never abandoned).
    pub joined: bool,
    /// Wall time from shutdown request to worker exit.
    pub elapsed: Duration,
    /// Per-message drain/drop dispositions, in queue order.
    pub dispositions: Vec<String>,
    /// fd count after minus before: must be 0.
    pub fd_delta: i64,
}

/// Apply one host command. Returns true when the worker must stop.
fn apply_cmd(
    cmd: WorkerCommand,
    subs: &mut SubscriptionSet,
    pending: &mut Vec<String>,
    shutdown: &AtomicBool,
    dispositions: &Arc<Mutex<Vec<String>>>,
) -> bool {
    match cmd {
        WorkerCommand::Subscribe(topic) => {
            subs.subscribe(&topic);
            false
        }
        WorkerCommand::Outbox(msg) => {
            if pending.len() < SEND_QUEUE_CAP {
                pending.push(msg);
            } else {
                lock_mutex(dispositions).push(format!("dropped {msg}: outbox full"));
            }
            false
        }
        WorkerCommand::Shutdown => {
            shutdown.store(true, Ordering::Release);
            true
        }
    }
}

/// Non-blocking drain of queued host commands.
fn drain_commands(
    rx: &Receiver<WorkerCommand>,
    subs: &mut SubscriptionSet,
    pending: &mut Vec<String>,
    shutdown: &AtomicBool,
    dispositions: &Arc<Mutex<Vec<String>>>,
) -> bool {
    for cmd in rx.try_iter() {
        if apply_cmd(cmd, subs, pending, shutdown, dispositions) {
            return true;
        }
    }
    false
}

/// Wait until `deadline` (or forever when `None`, i.e. parked), staying
/// responsive to commands. Returns true when the worker must stop.
fn wait_for_deadline(
    deadline: Option<u64>,
    rx: &Receiver<WorkerCommand>,
    subs: &mut SubscriptionSet,
    pending: &mut Vec<String>,
    shutdown: &AtomicBool,
    dispositions: &Arc<Mutex<Vec<String>>>,
) -> bool {
    loop {
        if shutdown.load(Ordering::Acquire) {
            return true;
        }
        let quantum = match deadline {
            Some(at) => {
                let now = now_ms();
                if now >= at {
                    return false;
                }
                WAIT_POLL.min(Duration::from_millis(at - now))
            }
            None => WAIT_POLL,
        };
        match rx.recv_timeout(quantum) {
            Ok(cmd) => {
                if apply_cmd(cmd, subs, pending, shutdown, dispositions) {
                    return true;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                shutdown.store(true, Ordering::Release);
                return true;
            }
        }
    }
}

/// Read frames until the link drops or the host stops the worker.
/// Tracks the subscribe ack so the ladder can tell a healthy stream
/// from a flap.
fn read_loop(
    link: &mut dyn Link,
    rx: &Receiver<WorkerCommand>,
    subs: &mut SubscriptionSet,
    pending: &mut Vec<String>,
    shutdown: &AtomicBool,
    dispositions: &Arc<Mutex<Vec<String>>>,
) -> LoopEnd {
    let mut acked_at_ms: Option<u64> = None;
    loop {
        if drain_commands(rx, subs, pending, shutdown, dispositions) {
            return LoopEnd::Shutdown;
        }
        if shutdown.load(Ordering::Acquire) {
            return LoopEnd::Shutdown;
        }
        match link.read_frame(WAIT_POLL) {
            Err(_) => {
                return LoopEnd::Lost {
                    healthy_orderly: false,
                };
            }
            Ok(None) => {}
            Ok(Some(frame)) => {
                if frame == "ack" {
                    acked_at_ms = Some(now_ms());
                } else if frame == "bye" {
                    let healthy = acked_at_ms
                        .is_some_and(|t| now_ms().saturating_sub(t) >= HEALTHY_STREAM_MS);
                    return LoopEnd::Lost {
                        healthy_orderly: healthy,
                    };
                }
            }
        }
    }
}

fn worker_main(
    mut link: Box<dyn Link>,
    mut subs: SubscriptionSet,
    rx: Receiver<WorkerCommand>,
    shutdown: Arc<AtomicBool>,
    dispositions: Arc<Mutex<Vec<String>>>,
) {
    let mut engine = ReconnectEngine::new();
    let mut pending: Vec<String> = Vec::new();
    'run: loop {
        if shutdown.load(Ordering::Acquire) {
            break 'run;
        }
        if drain_commands(&rx, &mut subs, &mut pending, &shutdown, &dispositions) {
            break 'run;
        }
        let mut deadline: Option<u64> = None;
        if engine.note_attempt(now_ms()) {
            match link.connect() {
                Ok(()) => {
                    for topic in subs.topics() {
                        let _ = link.write_frame(&format!("sub:{topic}"), WRITE_ATTEMPT_TIMEOUT);
                    }
                    match read_loop(
                        &mut *link,
                        &rx,
                        &mut subs,
                        &mut pending,
                        &shutdown,
                        &dispositions,
                    ) {
                        LoopEnd::Shutdown => break 'run,
                        LoopEnd::Lost { healthy_orderly } => {
                            engine.note_disconnect(now_ms(), healthy_orderly);
                            link.close();
                        }
                    }
                }
                Err(_) => {
                    engine.note_disconnect(now_ms(), false);
                }
            }
            deadline = engine.next_attempt_at_ms();
        }
        if wait_for_deadline(
            deadline,
            &rx,
            &mut subs,
            &mut pending,
            &shutdown,
            &dispositions,
        ) {
            break 'run;
        }
    }
    // Late commands that arrived after the last poll still count as
    // in-flight: drain them before deciding dispositions.
    drain_commands(&rx, &mut subs, &mut pending, &shutdown, &dispositions);
    // Drain-or-drop: one bounded write attempt per message; the whole
    // pass is bounded by SHUTDOWN_TIMEOUT so an unresponsive daemon
    // cannot wedge shutdown. Every message's disposition is logged.
    let drain_start = Instant::now();
    {
        let mut disp = lock_mutex(&dispositions);
        for msg in pending {
            if drain_start.elapsed() >= SHUTDOWN_TIMEOUT {
                disp.push(format!("dropped {msg}: shutdown timeout"));
            } else {
                match link.write_frame(&msg, WRITE_ATTEMPT_TIMEOUT) {
                    Ok(()) => disp.push(format!("drained {msg}")),
                    Err(e) => disp.push(format!("dropped {msg}: {e:?}")),
                }
            }
        }
    }
    link.close();
}

impl DaemonClient {
    /// Spawn the worker thread over `link`, initially subscribed to
    /// `initial` topics.
    pub fn spawn(link: Box<dyn Link>, initial: &[&str]) -> std::io::Result<Self> {
        let mut subs = SubscriptionSet::new();
        for topic in initial {
            subs.subscribe(topic);
        }
        let (tx, rx) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let dispositions = Arc::new(Mutex::new(Vec::new()));
        let worker_shutdown = Arc::clone(&shutdown);
        let worker_dispositions = Arc::clone(&dispositions);
        let fd_before = fd_count();
        let worker = thread::Builder::new()
            .name(WORKER_THREAD_NAME.to_string())
            .spawn(move || worker_main(link, subs, rx, worker_shutdown, worker_dispositions))?;
        Ok(Self {
            worker: Some(worker),
            tx,
            shutdown,
            dispositions,
            fd_before,
        })
    }

    /// Subscribe to a topic (client state; resent on next reconnect).
    pub fn subscribe(&self, topic: &str) {
        let _ = self.tx.send(WorkerCommand::Subscribe(topic.to_string()));
    }

    /// Queue a host→daemon message. False when the worker is gone.
    pub fn send(&self, msg: String) -> bool {
        self.tx.send(WorkerCommand::Outbox(msg)).is_ok()
    }

    /// Stop the worker: signal, bound the join by [`SHUTDOWN_TIMEOUT`],
    /// drain or drop in-flight messages with per-message dispositions.
    /// The thread is always joined, never detached.
    pub fn shutdown(mut self) -> ShutdownReport {
        let start = Instant::now();
        self.shutdown.store(true, Ordering::Release);
        let _ = self.tx.send(WorkerCommand::Shutdown);
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        let handle = self.worker.take().expect("worker thread missing");
        while !handle.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let joined = handle.is_finished() && handle.join().is_ok();
        let elapsed = start.elapsed();
        let dispositions = lock_mutex(&self.dispositions).clone();
        let fd_delta = fd_count() as i64 - self.fd_before as i64;
        ShutdownReport {
            joined,
            elapsed,
            dispositions,
            fd_delta,
        }
    }
}
