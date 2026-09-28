// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Wave 29 shared harness: the ephemeral browser bridge pattern.
//!
//! Adapted pattern (Ghostex `skills/ghostex-embedded-browser-use/SKILL.md`
//! @ c911466): a CLI-launched, **ephemeral** per-task stdio MCP bridge to
//! a browser DevTools port. The bridge is born with the task and dies with
//! it; it never writes persistent MCP config. Ghostex's CEF-inside-GPUI
//! implementation is entangled and was NOT lifted — only the pattern,
//! re-implemented here in Tiger Style Rust against Chromium remote
//! debugging (scripted endpoint for the protocol half, real `chromium`
//! on primo for the integration half).
//!
//! # Declared contracts
//!
//! - **Stdio framing:** newline-delimited JSON-RPC 2.0. One message per
//!   line, `\n` terminator, UTF-8, at most [`MAX_MESSAGE_BYTES`] per line.
//!   Partial reads buffer until the newline; a peer that closes stdout
//!   mid-message yields [`BridgeError::Eof`], never a silent desync.
//! - **Result ingestion:** `Runtime.evaluate` remote objects are mapped to
//!   MCP content with [`MAX_RESULT_BYTES`] / [`MAX_RESULT_DEPTH`] bounds.
//!   Strings are prefix-truncated (never silently — `truncated: true` is
//!   always set). Cyclic values are **rejected** per
//!   [`CYCLIC_CONTRACT`]; the walk is iterative, so no stack overflow.
//! - **Navigation:** only `http`/`https` to declared hosts (exact or
//!   dot-subdomain). Everything else → [`BridgeError::NavigationDenied`].
//! - **Targets:** one target per bridge unless the task declares
//!   multi-target; external `Target.created` events (e.g. `window.open`)
//!   are refused and closed. Target ids are namespaced per bridge as
//!   `<task-id>:<raw-id>` — disjoint by construction.
//! - **Teardown:** SIGTERM, then SIGKILL after [`SIGTERM_GRACE`], hard cap
//!   [`BRIDGE_KILL_TIMEOUT`]. Own-and-release-exactly-once on every path.
//!
//! # What is scripted vs real
//!
//! The bridge **process** is a real OS child (a `sleep`/`python3` fixture
//! stub, honestly labeled MOCK, standing in for the real bridge binary)
//! so launch/kill/reap/zombie semantics are genuine. The DevTools port has
//! two backends: [`ScriptedPort`] (deterministic, in-process) and
//! [`ChromiumPort`] (real chromium over HTTP, no websocket — so
//! `Runtime.evaluate` itself is covered by the scripted half, citing the
//! CDP docs).

use crate::bounty::approve::sha256_hex;
use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Max bytes of page-result text admitted into agent context.
pub const MAX_RESULT_BYTES: usize = 65_536;
/// Max nesting depth admitted during result deserialization.
pub const MAX_RESULT_DEPTH: u32 = 32;
/// Max bytes of one newline-delimited stdio message.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
/// Default target bound: one page per bridge unless declared otherwise.
pub const MAX_TARGETS_DEFAULT: usize = 1;
/// Hard cap for bridge teardown (SIGTERM → SIGKILL escalation inside).
pub const BRIDGE_KILL_TIMEOUT: Duration = Duration::from_secs(5);
/// Grace after SIGTERM before SIGKILL escalation.
pub const SIGTERM_GRACE: Duration = Duration::from_millis(500);
/// Headroom over [`MAX_RESULT_BYTES`] the allocation meter tolerates
/// (structural JSON punctuation around the bounded payload).
pub const METER_HEADROOM_BYTES: usize = 8 * 1024;
/// Declared cyclic-value contract: reject with a typed error.
pub const CYCLIC_CONTRACT: &str = "reject";
/// Marker the scripted endpoint uses to encode a cyclic remote object.
pub const CYCLIC_MARKER_KEY: &str = "$cyclic";
/// Marker substituted for subtrees deeper than [`MAX_RESULT_DEPTH`].
pub const DEPTH_MARKER: &str = "$truncated-max-depth";
/// Chromium binary used for the real-browser integration half.
pub const CHROMIUM_BIN: &str = "/usr/bin/chromium";
/// How long launch waits for chromium's debugging port to answer.
pub const CHROMIUM_READY_TIMEOUT: Duration = Duration::from_secs(15);
/// Poll interval while waiting for the debugging port.
pub const CHROMIUM_READY_POLL: Duration = Duration::from_millis(50);
/// Bounded wait for the SIGTERM-ignoring stub's readiness line (it
/// prints only after installing SIG_IGN). A fixed sleep raced handler
/// installation under load: SIGTERM landing during interpreter
/// startup killed the child the ordinary way and the SIGKILL-
/// escalation assertion went flaky.
pub const IGNORE_SIGTERM_READY_TIMEOUT: Duration = Duration::from_secs(10);
/// Slow-evaluate spin budget: polls x interval = 30s max. The
/// task-183 driver always cancels mid-spin; the budget only bounds a
/// driver that never cancels.
const SLOW_EVALUATE_POLLS: u64 = 6_000;
const SLOW_EVALUATE_POLL: Duration = Duration::from_millis(5);
/// Bounded reap budget for `Drop`'s best-effort cleanup (panic path
/// only; normal teardown reaps fully via [`terminate_child`]).
pub const DROP_REAP_TIMEOUT: Duration = Duration::from_millis(200);
/// Poll interval inside `Drop`'s bounded reap.
pub const DROP_REAP_POLL: Duration = Duration::from_millis(10);

/// Typed bridge failures. Every refusal the bridge can produce.
#[derive(Debug, PartialEq, Eq)]
pub enum BridgeError {
    /// Peer closed stdout with a partial message buffered.
    Eof,
    /// A stdio line exceeded [`MAX_MESSAGE_BYTES`] before its newline.
    FramingTooLarge { bytes: usize },
    /// A complete line was not valid JSON.
    FramingNotJson { detail: String },
    /// The DevTools port failed or refused an operation.
    DevTools { detail: String },
    /// Navigation outside the task's declared scope.
    NavigationDenied { url: String, reason: String },
    /// A target id outside this bridge's namespace.
    UnknownTarget { id: String },
    /// An externally created target the task did not declare.
    TargetRefused { raw_id: String },
    /// More targets than the task declared.
    TargetLimitExceeded { limit: usize },
    /// The bridge has no live target (used before launch / after close).
    NoTarget,
    /// The bridge child process could not be spawned.
    SpawnFailed { detail: String },
    /// The bridge child could not be reaped in time.
    KillFailed { detail: String },
    /// An in-flight evaluate was cancelled.
    Cancelled,
    /// A cyclic remote-object value (rejected per [`CYCLIC_CONTRACT`]).
    CyclicValue,
    /// A value nested deeper than [`MAX_RESULT_DEPTH`].
    DepthExceeded { depth: u32 },
    /// The MCP config surface changed across a bridge run.
    ConfigDirty { detail: String },
    /// The real-chromium backend is unavailable.
    ChromiumMissing,
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::Eof => write!(f, "peer closed stdout mid-message"),
            BridgeError::FramingTooLarge { bytes } => {
                write!(
                    f,
                    "stdio line exceeded {MAX_MESSAGE_BYTES} bytes ({bytes} buffered)"
                )
            }
            BridgeError::FramingNotJson { detail } => {
                write!(f, "stdio line is not valid JSON: {detail}")
            }
            BridgeError::DevTools { detail } => write!(f, "devtools port error: {detail}"),
            BridgeError::NavigationDenied { url, reason } => {
                write!(f, "navigation to {url} denied: {reason}")
            }
            BridgeError::UnknownTarget { id } => {
                write!(f, "unknown target '{id}' in this bridge's namespace")
            }
            BridgeError::TargetRefused { raw_id } => {
                write!(
                    f,
                    "externally created target '{raw_id}' refused: not declared"
                )
            }
            BridgeError::TargetLimitExceeded { limit } => {
                write!(f, "target count would exceed declared bound {limit}")
            }
            BridgeError::NoTarget => write!(f, "bridge has no live target"),
            BridgeError::SpawnFailed { detail } => {
                write!(f, "bridge process spawn failed: {detail}")
            }
            BridgeError::KillFailed { detail } => {
                write!(f, "bridge process kill failed: {detail}")
            }
            BridgeError::Cancelled => write!(f, "cancelled mid-evaluate"),
            BridgeError::CyclicValue => write!(
                f,
                "cyclic remote-object value rejected (contract: {CYCLIC_CONTRACT})"
            ),
            BridgeError::DepthExceeded { depth } => {
                write!(f, "value nested {depth} deep, bound is {MAX_RESULT_DEPTH}")
            }
            BridgeError::ConfigDirty { detail } => {
                write!(f, "MCP config surface changed across bridge run: {detail}")
            }
            BridgeError::ChromiumMissing => write!(f, "real chromium backend unavailable"),
        }
    }
}

impl std::error::Error for BridgeError {}

// ---------------------------------------------------------------------------
// Stdio framing: newline-delimited JSON-RPC 2.0 (declared contract).
// ---------------------------------------------------------------------------

/// Newline-delimited JSON message framer over a byte stream.
///
/// `push_chunk` buffers partial reads until a `\n` completes a message;
/// `finish` models the peer closing stdout. A line longer than
/// [`MAX_MESSAGE_BYTES`] is a typed error, never a silent desync.
pub struct StdioFramer {
    buf: Vec<u8>,
}

impl StdioFramer {
    /// Empty framer.
    pub fn new() -> Self {
        StdioFramer { buf: Vec::new() }
    }

    /// Bytes still buffered without a terminating newline.
    pub fn pending_bytes(&self) -> usize {
        self.buf.len()
    }

    /// Feed newly read bytes; returns every complete message, in order.
    /// A message is handed to the caller exactly once.
    pub fn push_chunk(&mut self, chunk: &[u8]) -> Result<Vec<serde_json::Value>, BridgeError> {
        self.buf.extend_from_slice(chunk);
        if !self.buf.contains(&b'\n') && self.buf.len() > MAX_MESSAGE_BYTES {
            let bytes = self.buf.len();
            self.buf.clear();
            return Err(BridgeError::FramingTooLarge { bytes });
        }
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = &line[..line.len() - 1];
            if line.is_empty() {
                continue;
            }
            let text =
                String::from_utf8(line.to_vec()).map_err(|e| BridgeError::FramingNotJson {
                    detail: e.to_string(),
                })?;
            let value: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| BridgeError::FramingNotJson {
                    detail: e.to_string(),
                })?;
            out.push(value);
        }
        Ok(out)
    }

    /// The peer closed stdout. A buffered partial message is
    /// [`BridgeError::Eof`]; an empty buffer is a clean shutdown.
    pub fn finish(&mut self) -> Result<(), BridgeError> {
        if self.buf.is_empty() {
            Ok(())
        } else {
            self.buf.clear();
            Err(BridgeError::Eof)
        }
    }
}

impl Default for StdioFramer {
    fn default() -> Self {
        Self::new()
    }
}

/// One event from the scripted stdio peer.
pub enum PeerEvent {
    /// Bytes the peer wrote.
    Chunk(Vec<u8>),
    /// The peer closed its stdout.
    Eof,
}

/// Outcome of driving the bridge's stdio loop against scripted events.
pub struct LoopReport {
    /// Process exit code the bridge would report (0 = clean).
    pub exit_code: i32,
    /// Complete messages dispatched to the handler, each exactly once.
    pub handled: u64,
    /// In-flight request ids failed with the typed EOF error.
    pub inflight_failed: Vec<u64>,
    /// True when EOF arrived with no partial message pending.
    pub eof_clean: bool,
}

/// Drive the bridge stdio loop against a scripted peer.
///
/// One request (id 1) is in flight when the loop starts. A mid-message
/// EOF fails it with the typed [`BridgeError::Eof`] and the loop exits
/// 0 — clean shutdown, not a crash.
pub fn run_bridge_loop(events: Vec<PeerEvent>) -> LoopReport {
    let mut framer = StdioFramer::new();
    let mut handled: u64 = 0;
    let mut inflight = vec![1u64];
    for event in events {
        match event {
            PeerEvent::Chunk(bytes) => match framer.push_chunk(&bytes) {
                Ok(messages) => handled += messages.len() as u64,
                Err(_) => {
                    return LoopReport {
                        exit_code: 1,
                        handled,
                        inflight_failed: std::mem::take(&mut inflight),
                        eof_clean: false,
                    };
                }
            },
            PeerEvent::Eof => match framer.finish() {
                Ok(()) => {
                    return LoopReport {
                        exit_code: 0,
                        handled,
                        inflight_failed: Vec::new(),
                        eof_clean: true,
                    };
                }
                Err(BridgeError::Eof) => {
                    return LoopReport {
                        exit_code: 0,
                        handled,
                        inflight_failed: std::mem::take(&mut inflight),
                        eof_clean: false,
                    };
                }
                Err(_) => {
                    return LoopReport {
                        exit_code: 1,
                        handled,
                        inflight_failed: std::mem::take(&mut inflight),
                        eof_clean: false,
                    };
                }
            },
        }
    }
    LoopReport {
        exit_code: 0,
        handled,
        inflight_failed: Vec::new(),
        eof_clean: true,
    }
}

// ---------------------------------------------------------------------------
// DevTools port backends.
// ---------------------------------------------------------------------------

/// One browser target as seen on the debugging port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetInfo {
    /// Port-native id (before per-bridge namespacing).
    pub raw_id: String,
    /// Current URL.
    pub url: String,
    /// Current title.
    pub title: String,
}

/// The debugging-port surface the bridge drives.
///
/// Two backends: [`ScriptedPort`] (deterministic, in-process — the
/// protocol half) and [`ChromiumPort`] (real chromium over HTTP — the
/// integration half; it has no websocket transport, so `evaluate` and
/// `navigate` there report [`BridgeError::DevTools`] and the scripted
/// half covers `Runtime.evaluate` semantics against the CDP docs).
///
/// The trait is `Send` because ports are routinely shared across the
/// watchdog and task threads (`SharedScriptedPort`).
pub trait DevToolsPort: Send {
    /// Current targets on the port.
    fn targets(&self) -> Vec<TargetInfo>;
    /// Open a target; the port-native `Target.created` equivalent.
    fn create_target(&mut self, url: &str) -> Result<TargetInfo, BridgeError>;
    /// Close a target by port-native id.
    fn close_target(&mut self, raw_id: &str) -> Result<(), BridgeError>;
    /// `Runtime.evaluate` on a target; returns the CDP `result` object
    /// (`{"type": ..., "value": ...}`, `{"type": "undefined"}`, ...).
    fn evaluate(&mut self, raw_id: &str, expr: &str) -> Result<serde_json::Value, BridgeError>;
    /// `Page.navigate` on a target.
    fn navigate(&mut self, raw_id: &str, url: &str) -> Result<(), BridgeError>;
    /// URLs this port was asked to navigate to (scripted observation).
    fn navigations_observed(&self) -> Vec<String>;
    /// A target created *outside* the bridge (e.g. `window.open`); the
    /// bridge must refuse or adopt it per its declaration.
    fn take_external_target(&mut self) -> Option<TargetInfo>;
    /// Signal cancellation to a slow in-flight evaluate.
    fn set_cancel(&mut self);
    /// How many `Target.created`-equivalent events this port emitted.
    fn created_events(&self) -> u64;
}

/// Deterministic in-process DevTools endpoint.
///
/// Evaluate responses are canned `(expression-substring, CDP-result)`
/// rules. `slow_evaluate` makes `evaluate` spin on the shared cancel
/// flag so task 183 can cancel mid-evaluate without threads racing the
/// real browser.
pub struct ScriptedPort {
    targets: Vec<TargetInfo>,
    next_id: u64,
    created_events: u64,
    evaluate_rules: Vec<(String, serde_json::Value)>,
    slow_evaluate: bool,
    cancel: Arc<AtomicBool>,
    navigations: Vec<String>,
    external_pending: Option<TargetInfo>,
}

impl ScriptedPort {
    /// Empty port: no targets, no rules.
    pub fn new() -> Self {
        ScriptedPort {
            targets: Vec::new(),
            next_id: 0,
            created_events: 0,
            evaluate_rules: Vec::new(),
            slow_evaluate: false,
            cancel: Arc::new(AtomicBool::new(false)),
            navigations: Vec::new(),
            external_pending: None,
        }
    }

    /// Add a canned evaluate rule: when `expr` contains `pattern`,
    /// return `result` (a CDP `Runtime.evaluate` result object).
    pub fn on_evaluate(&mut self, pattern: &str, result: serde_json::Value) {
        self.evaluate_rules.push((pattern.to_string(), result));
    }

    /// Make `evaluate` block on the cancel flag (task 183 fixture).
    pub fn set_slow_evaluate(&mut self, slow: bool) {
        self.slow_evaluate = slow;
    }

    /// Simulate a page-side `window.open`: a target the bridge did not
    /// create appears on the port.
    pub fn simulate_external_target(&mut self, url: &str) {
        let raw_id = format!("ext-{:04}", self.next_id);
        self.next_id += 1;
        let info = TargetInfo {
            raw_id: raw_id.clone(),
            url: url.to_string(),
            title: String::new(),
        };
        self.targets.push(info.clone());
        self.external_pending = Some(info);
    }

    /// How many `Target.created`-equivalent events this port emitted.
    pub fn created_events(&self) -> u64 {
        self.created_events
    }

    fn find_target(&self, raw_id: &str) -> Option<usize> {
        self.targets.iter().position(|t| t.raw_id == raw_id)
    }

    /// Target lookup + canned rules without the slow spin. The shared
    /// port runs the spin outside its mutex, then delegates here.
    fn evaluate_inner(
        &mut self,
        raw_id: &str,
        expr: &str,
    ) -> Result<serde_json::Value, BridgeError> {
        if self.find_target(raw_id).is_none() {
            return Err(BridgeError::UnknownTarget {
                id: raw_id.to_string(),
            });
        }
        for (pattern, result) in &self.evaluate_rules {
            if expr.contains(pattern.as_str()) {
                // Fixture edge: cloning the canned rule materializes
                // the simulated wire bytes (what a real browser would
                // send over the CDP websocket). The ingestion path
                // (`ingest_remote`) must not add unbounded copies of
                // its own — that is what the AllocMeter polices.
                return Ok(result.clone());
            }
        }
        Ok(serde_json::json!({"type": "undefined"}))
    }
}

impl Default for ScriptedPort {
    fn default() -> Self {
        Self::new()
    }
}

impl DevToolsPort for ScriptedPort {
    fn targets(&self) -> Vec<TargetInfo> {
        self.targets.clone()
    }

    fn create_target(&mut self, url: &str) -> Result<TargetInfo, BridgeError> {
        let raw_id = format!("scripted-{:04}", self.next_id);
        self.next_id += 1;
        self.created_events += 1;
        let info = TargetInfo {
            raw_id,
            url: url.to_string(),
            title: String::new(),
        };
        self.targets.push(info.clone());
        Ok(info)
    }

    fn close_target(&mut self, raw_id: &str) -> Result<(), BridgeError> {
        match self.find_target(raw_id) {
            Some(idx) => {
                self.targets.remove(idx);
                Ok(())
            }
            None => Err(BridgeError::DevTools {
                detail: format!("close of unknown target {raw_id}"),
            }),
        }
    }

    fn evaluate(&mut self, raw_id: &str, expr: &str) -> Result<serde_json::Value, BridgeError> {
        if self.slow_evaluate {
            // Long-running page script: poll the cancel flag so the
            // task-183 cancellation has something to interrupt.
            slow_spin(&self.cancel)?;
        }
        self.evaluate_inner(raw_id, expr)
    }

    fn navigate(&mut self, raw_id: &str, url: &str) -> Result<(), BridgeError> {
        match self.find_target(raw_id) {
            Some(idx) => {
                self.targets[idx].url = url.to_string();
                self.navigations.push(url.to_string());
                Ok(())
            }
            None => Err(BridgeError::UnknownTarget {
                id: raw_id.to_string(),
            }),
        }
    }

    fn navigations_observed(&self) -> Vec<String> {
        self.navigations.clone()
    }

    fn take_external_target(&mut self) -> Option<TargetInfo> {
        self.external_pending.take()
    }

    fn set_cancel(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    fn created_events(&self) -> u64 {
        self.created_events
    }
}

/// Poll `cancel` up to [`SLOW_EVALUATE_POLLS`] times: the task-183
/// long-running page script. Returns typed [`BridgeError::Cancelled`]
/// the moment the flag fires.
fn slow_spin(cancel: &AtomicBool) -> Result<(), BridgeError> {
    for _ in 0..SLOW_EVALUATE_POLLS {
        if cancel.load(Ordering::SeqCst) {
            return Err(BridgeError::Cancelled);
        }
        std::thread::sleep(SLOW_EVALUATE_POLL);
    }
    Ok(())
}

/// Build a CDP `Runtime.evaluate` result object for a string value.
pub fn cdp_string(value: &str) -> serde_json::Value {
    serde_json::json!({"type": "string", "value": value})
}

/// Build a CDP `Runtime.evaluate` result object for an object value.
pub fn cdp_object(value: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"type": "object", "value": value, "description": "Object"})
}

/// A `ScriptedPort` behind a shared handle, so a driver can inject
/// port-side events (an external `Target.created`, cancellation) while
/// the bridge owns the port — the honest model of asynchronous
/// page-side behavior.
#[derive(Clone, Default)]
pub struct SharedScriptedPort {
    inner: Arc<Mutex<ScriptedPort>>,
}

impl SharedScriptedPort {
    /// A cloneable handle to the underlying scripted port.
    pub fn handle(&self) -> Arc<Mutex<ScriptedPort>> {
        Arc::clone(&self.inner)
    }
}

impl DevToolsPort for SharedScriptedPort {
    fn targets(&self) -> Vec<TargetInfo> {
        self.inner.lock().unwrap().targets()
    }
    fn create_target(&mut self, url: &str) -> Result<TargetInfo, BridgeError> {
        self.inner.lock().unwrap().create_target(url)
    }
    fn close_target(&mut self, raw_id: &str) -> Result<(), BridgeError> {
        self.inner.lock().unwrap().close_target(raw_id)
    }
    fn evaluate(&mut self, raw_id: &str, expr: &str) -> Result<serde_json::Value, BridgeError> {
        // Snapshot the spin inputs under a short lock. The spin must
        // NOT run under the inner mutex: the driver sets the cancel
        // flag through this same mutex, and a 30s locked spin would
        // serialize the cancel after it — the typed Cancelled is lost
        // and mid-evaluate cancellation degrades to Ok(Null).
        let (slow, cancel) = {
            let inner = self.inner.lock().unwrap();
            (inner.slow_evaluate, Arc::clone(&inner.cancel))
        };
        if slow {
            slow_spin(&cancel)?;
        }
        self.inner.lock().unwrap().evaluate_inner(raw_id, expr)
    }
    fn navigate(&mut self, raw_id: &str, url: &str) -> Result<(), BridgeError> {
        self.inner.lock().unwrap().navigate(raw_id, url)
    }
    fn navigations_observed(&self) -> Vec<String> {
        self.inner.lock().unwrap().navigations_observed()
    }
    fn take_external_target(&mut self) -> Option<TargetInfo> {
        self.inner.lock().unwrap().take_external_target()
    }
    fn set_cancel(&mut self) {
        self.inner.lock().unwrap().set_cancel();
    }
    fn created_events(&self) -> u64 {
        self.inner.lock().unwrap().created_events()
    }
}

/// Real chromium over its HTTP debugging endpoints (`/json/list`,
/// `/json/new`, `/json/close`). No websocket transport: `evaluate` and
/// `navigate` report [`BridgeError::DevTools`]; use [`ScriptedPort`]
/// for `Runtime.evaluate` semantics.
pub struct ChromiumPort {
    base: String,
    client: reqwest::blocking::Client,
}

/// Build the `PUT /json/new?{url}` URL. Per the DevTools HTTP docs
/// (https://chromedevtools.github.io/devtools-protocol/, "PUT /json/new
/// or PUT /json/new?{url}"), the raw query component — not a `url=`
/// parameter — is parsed and URL-unescaped as the initial navigation
/// URL; a `url=` parameter is treated as a literal (invalid) URL and
/// silently falls back to `about:blank`.
fn new_target_url(base: &str, url: &str) -> String {
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    let encoded: String = utf8_percent_encode(url, NON_ALPHANUMERIC).collect();
    format!("{base}/json/new?{encoded}")
}

impl ChromiumPort {
    /// Attach to a debugging port, e.g. `ChromiumPort::new(9333)`.
    pub fn new(port: u16) -> Result<Self, BridgeError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| BridgeError::DevTools {
                detail: format!("http client build failed: {e}"),
            })?;
        Ok(ChromiumPort {
            base: format!("http://127.0.0.1:{port}"),
            client,
        })
    }

    /// Fallible target list: unlike the trait's `targets()` (which
    /// swallows transport errors into an empty vec), this
    /// distinguishes "zero targets" from "the port is dead" — the
    /// distinction the task-178 teardown proof needs.
    pub fn probe_targets(&self) -> Result<Vec<TargetInfo>, BridgeError> {
        self.list_targets()
    }

    fn list_targets(&self) -> Result<Vec<TargetInfo>, BridgeError> {
        let body = self
            .client
            .get(format!("{}/json/list", self.base))
            .send()
            .map_err(|e| BridgeError::DevTools {
                detail: format!("/json/list failed: {e}"),
            })?
            .text()
            .map_err(|e| BridgeError::DevTools {
                detail: format!("/json/list body failed: {e}"),
            })?;
        let items: Vec<serde_json::Value> =
            serde_json::from_str(&body).map_err(|e| BridgeError::DevTools {
                detail: format!("/json/list is not JSON: {e}"),
            })?;
        let mut out = Vec::new();
        for item in items {
            out.push(TargetInfo {
                raw_id: item
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                url: item
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                title: item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
        Ok(out)
    }

    /// Open `url` in a new target and wait (bounded) for its title to
    /// become `want_title`. This is the integration-half round trip:
    /// the bridge drives a real browser to a URL and observes the page.
    pub fn open_url_and_wait_title(
        &self,
        url: &str,
        want_title: &str,
        timeout: Duration,
    ) -> Result<TargetInfo, BridgeError> {
        // Raw query component per the DevTools HTTP docs (see
        // `new_target_url`): `?url=` would silently open about:blank.
        let created: serde_json::Value = self
            .client
            .put(new_target_url(&self.base, url))
            .send()
            .map_err(|e| BridgeError::DevTools {
                detail: format!("/json/new failed: {e}"),
            })?
            .json()
            .map_err(|e| BridgeError::DevTools {
                detail: format!("/json/new is not JSON: {e}"),
            })?;
        let raw_id = created
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            for target in self.list_targets()? {
                if target.raw_id == raw_id && target.title == want_title {
                    return Ok(target);
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Err(BridgeError::DevTools {
            detail: format!("title '{want_title}' never appeared for target {raw_id}"),
        })
    }
}

impl DevToolsPort for ChromiumPort {
    fn targets(&self) -> Vec<TargetInfo> {
        self.list_targets().unwrap_or_default()
    }

    fn create_target(&mut self, url: &str) -> Result<TargetInfo, BridgeError> {
        // Raw query component per the DevTools HTTP docs (see
        // `new_target_url`): `?url=` would silently open about:blank.
        let created: serde_json::Value = self
            .client
            .put(new_target_url(&self.base, url))
            .send()
            .map_err(|e| BridgeError::DevTools {
                detail: format!("/json/new failed: {e}"),
            })?
            .json()
            .map_err(|e| BridgeError::DevTools {
                detail: format!("/json/new is not JSON: {e}"),
            })?;
        Ok(TargetInfo {
            raw_id: created
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            url: created
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            title: String::new(),
        })
    }

    fn close_target(&mut self, raw_id: &str) -> Result<(), BridgeError> {
        // GET per the DevTools HTTP docs ("GET /json/close/{targetId}").
        self.client
            .get(format!("{}/json/close/{raw_id}", self.base))
            .send()
            .map_err(|e| BridgeError::DevTools {
                detail: format!("/json/close failed: {e}"),
            })?;
        Ok(())
    }

    fn evaluate(&mut self, _raw_id: &str, _expr: &str) -> Result<serde_json::Value, BridgeError> {
        Err(BridgeError::DevTools {
            detail: "ChromiumPort has no websocket transport; Runtime.evaluate is covered by the scripted half (see CDP docs)".to_string(),
        })
    }

    fn navigate(&mut self, _raw_id: &str, _url: &str) -> Result<(), BridgeError> {
        Err(BridgeError::DevTools {
            detail: "ChromiumPort has no websocket transport; navigation is covered by the scripted half".to_string(),
        })
    }

    fn navigations_observed(&self) -> Vec<String> {
        Vec::new()
    }

    fn take_external_target(&mut self) -> Option<TargetInfo> {
        None
    }

    fn set_cancel(&mut self) {}

    fn created_events(&self) -> u64 {
        0
    }
}

/// True when a `chromium` binary answers `--version`.
pub fn chromium_available() -> bool {
    Command::new("chromium")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A free loopback TCP port for a test chromium instance.
pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(19_299)
}

// ---------------------------------------------------------------------------
// Bounded result ingestion: CDP remote object -> MCP content.
// ---------------------------------------------------------------------------

/// Counts every byte the ingestion path copies into agent context.
/// The task-181 pass criterion is `used() <= MAX_RESULT_BYTES +
/// METER_HEADROOM_BYTES`: zero unbounded allocations.
pub struct AllocMeter {
    used: usize,
}

impl AllocMeter {
    /// Fresh meter.
    pub fn new() -> Self {
        AllocMeter { used: 0 }
    }

    /// Bytes copied into agent-visible output so far.
    pub fn used(&self) -> usize {
        self.used
    }

    /// Remaining budget before [`MAX_RESULT_BYTES`] is reached.
    pub fn remaining(&self) -> usize {
        MAX_RESULT_BYTES.saturating_sub(self.used)
    }

    fn add(&mut self, n: usize) {
        self.used = self.used.saturating_add(n);
    }
}

impl Default for AllocMeter {
    fn default() -> Self {
        Self::new()
    }
}

/// An evaluated page value, mapped to MCP content types.
#[derive(Debug, Clone)]
pub enum McpContent {
    /// Text content; `truncated` is always set when the page value was
    /// cut at a bound — truncation is signaled, never silent.
    Text { text: String, truncated: bool },
    /// The page value was `undefined`: a typed null, not a crash.
    Null,
}

/// Truncate `s` to the meter's remaining budget at a char boundary.
/// Returns the kept prefix and whether anything was cut.
fn truncate_str(s: &str, meter: &mut AllocMeter) -> (String, bool) {
    let keep = meter.remaining().min(s.len());
    let keep = s.floor_char_boundary(keep);
    meter.add(keep);
    (s[..keep].to_string(), keep < s.len())
}

enum OutNode {
    Obj(Vec<(String, usize)>),
    Arr(Vec<usize>),
    Leaf(serde_json::Value),
}

enum Slot {
    Root,
    ObjKey(usize, String),
    ArrIdx(usize),
}

struct Frame<'a> {
    src: &'a serde_json::Value,
    depth: u32,
    slot: Slot,
}

fn attach(arena: &mut [OutNode], root_idx: &mut Option<usize>, slot: Slot, idx: usize) {
    match slot {
        Slot::Root => *root_idx = Some(idx),
        Slot::ObjKey(parent, key) => {
            if let OutNode::Obj(pairs) = &mut arena[parent] {
                pairs.push((key, idx));
            }
        }
        Slot::ArrIdx(parent) => {
            if let OutNode::Arr(kids) = &mut arena[parent] {
                kids.push(idx);
            }
        }
    }
}

fn leaf(arena: &mut Vec<OutNode>, root_idx: &mut Option<usize>, slot: Slot, v: serde_json::Value) {
    let idx = arena.len();
    arena.push(OutNode::Leaf(v));
    attach(arena, root_idx, slot, idx);
}

/// Iteratively bound a JSON value: depth cap, cyclic rejection, string
/// prefix-truncation at the meter budget. No recursion: a deeply
/// nested hostile value cannot overflow the stack.
fn bound_value(
    root: &serde_json::Value,
    meter: &mut AllocMeter,
) -> Result<(serde_json::Value, bool), BridgeError> {
    let mut truncated = false;
    let mut arena: Vec<OutNode> = Vec::new();
    let mut root_idx: Option<usize> = None;
    let mut stack = vec![Frame {
        src: root,
        depth: 0,
        slot: Slot::Root,
    }];
    while let Some(frame) = stack.pop() {
        if frame.depth > MAX_RESULT_DEPTH {
            truncated = true;
            let marker = serde_json::json!({DEPTH_MARKER: true});
            meter.add(marker.to_string().len());
            leaf(&mut arena, &mut root_idx, frame.slot, marker);
            continue;
        }
        match frame.src {
            serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
                let v = frame.src.clone();
                meter.add(v.to_string().len());
                leaf(&mut arena, &mut root_idx, frame.slot, v);
            }
            serde_json::Value::String(s) => {
                let (kept, was_cut) = truncate_str(s, meter);
                truncated |= was_cut;
                leaf(
                    &mut arena,
                    &mut root_idx,
                    frame.slot,
                    serde_json::Value::String(kept),
                );
            }
            serde_json::Value::Array(items) => {
                open_array(
                    &mut arena,
                    &mut root_idx,
                    &mut stack,
                    items,
                    frame.depth,
                    frame.slot,
                );
            }
            serde_json::Value::Object(map) => {
                open_object(
                    &mut arena,
                    &mut root_idx,
                    &mut stack,
                    map,
                    frame.depth,
                    frame.slot,
                )?;
            }
        }
    }
    let root_idx = root_idx.ok_or(BridgeError::DevTools {
        detail: "bound_value produced no root".to_string(),
    })?;
    Ok((materialize(&arena, root_idx), truncated))
}

/// Open an array node in the arena and queue its items (reversed, so
/// pop order matches document order).
fn open_array<'a>(
    arena: &mut Vec<OutNode>,
    root_idx: &mut Option<usize>,
    stack: &mut Vec<Frame<'a>>,
    items: &'a [serde_json::Value],
    depth: u32,
    slot: Slot,
) {
    let idx = arena.len();
    arena.push(OutNode::Arr(Vec::with_capacity(items.len().min(64))));
    attach(arena, root_idx, slot, idx);
    for item in items.iter().rev() {
        stack.push(Frame {
            src: item,
            depth: depth + 1,
            slot: Slot::ArrIdx(idx),
        });
    }
}

/// Open an object node in the arena and queue its entries, rejecting
/// the cyclic marker per the declared contract.
fn open_object<'a>(
    arena: &mut Vec<OutNode>,
    root_idx: &mut Option<usize>,
    stack: &mut Vec<Frame<'a>>,
    map: &'a serde_json::Map<String, serde_json::Value>,
    depth: u32,
    slot: Slot,
) -> Result<(), BridgeError> {
    if map.get(CYCLIC_MARKER_KEY) == Some(&serde_json::Value::Bool(true)) {
        return Err(BridgeError::CyclicValue);
    }
    let idx = arena.len();
    arena.push(OutNode::Obj(Vec::with_capacity(map.len().min(64))));
    attach(arena, root_idx, slot, idx);
    let mut entries: Vec<(&String, &serde_json::Value)> = map.iter().collect();
    entries.reverse();
    for (key, value) in entries {
        stack.push(Frame {
            src: value,
            depth: depth + 1,
            slot: Slot::ObjKey(idx, key.clone()),
        });
    }
    Ok(())
}

/// Rebuild a `serde_json::Value` from the arena, iteratively.
fn materialize(arena: &[OutNode], root: usize) -> serde_json::Value {
    let mut out: Vec<Option<serde_json::Value>> = (0..arena.len()).map(|_| None).collect();
    let mut stack = vec![(root, false)];
    while let Some((idx, visited)) = stack.pop() {
        if visited {
            out[idx] = Some(match &arena[idx] {
                OutNode::Leaf(v) => v.clone(),
                OutNode::Arr(kids) => serde_json::Value::Array(
                    kids.iter()
                        .map(|k| out[*k].clone().unwrap_or_default())
                        .collect(),
                ),
                OutNode::Obj(pairs) => {
                    let mut map = serde_json::Map::new();
                    for (key, child) in pairs {
                        map.insert(key.clone(), out[*child].clone().unwrap_or_default());
                    }
                    serde_json::Value::Object(map)
                }
            });
            continue;
        }
        stack.push((idx, true));
        match &arena[idx] {
            OutNode::Leaf(_) => {}
            OutNode::Arr(kids) => {
                for k in kids.iter().rev() {
                    stack.push((*k, false));
                }
            }
            OutNode::Obj(pairs) => {
                for (_, child) in pairs.iter().rev() {
                    stack.push((*child, false));
                }
            }
        }
    }
    out[root].clone().unwrap_or_default()
}

/// Map a CDP `Runtime.evaluate` result object to MCP content.
///
/// Primary source: the Chrome DevTools Protocol `Runtime.evaluate`
/// docs — `result` carries `type` (`string`, `number`, `boolean`,
/// `object`, `undefined`, `bigint`, ...) with `value`, or
/// `unserializableValue` for values JSON cannot carry (bigint,
/// `-0`, `NaN`, `Infinity`). `undefined` becomes a typed null.
pub fn ingest_remote(
    result: &serde_json::Value,
    meter: &mut AllocMeter,
) -> Result<McpContent, BridgeError> {
    let kind = result.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match kind {
        "undefined" => Ok(McpContent::Null),
        "string" => {
            let s = result.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let (text, truncated) = truncate_str(s, meter);
            Ok(McpContent::Text { text, truncated })
        }
        "number" | "boolean" => {
            // Scalars are small by construction; no clone of the
            // enclosing value, just its display form.
            let text = match result.get("value") {
                Some(v) => v.to_string(),
                None => "null".to_string(),
            };
            meter.add(text.len());
            Ok(McpContent::Text {
                text,
                truncated: false,
            })
        }
        "bigint" => {
            // CDP: unserializableValue like "9007199254740993n".
            // Truncated at the meter budget like every other string:
            // a hostile 50 MB bigint must not be copied whole.
            let raw = result
                .get("unserializableValue")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let raw = raw.strip_suffix('n').unwrap_or(raw);
            let (text, truncated) = truncate_str(raw, meter);
            Ok(McpContent::Text { text, truncated })
        }
        "object" => {
            // Borrowed: bound_value truncates in place, so the payload
            // is never duplicated whole before bounding. (The clone
            // that materialized this `result` lives in the scripted
            // fixture — the simulated wire bytes. The meter covers the
            // ingestion path: bytes copied into agent-visible output.)
            let empty = serde_json::Value::Null;
            let payload = result.get("value").unwrap_or(&empty);
            let (bounded, truncated) = bound_value(payload, meter)?;
            let text = serde_json::to_string(&bounded).map_err(|e| BridgeError::DevTools {
                detail: format!("bounded value would not serialize: {e}"),
            })?;
            Ok(McpContent::Text { text, truncated })
        }
        other => Err(BridgeError::DevTools {
            detail: format!("unhandled CDP remote type '{other}'"),
        }),
    }
}

// ---------------------------------------------------------------------------
// Navigation allowlist.
// ---------------------------------------------------------------------------

/// Split `scheme://host/...` into (scheme, host). None when malformed.
fn split_url(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split('/').next().unwrap_or("");
    if scheme.is_empty() || host.is_empty() {
        return None;
    }
    Some((scheme.to_ascii_lowercase(), host.to_ascii_lowercase()))
}

/// True when `host` equals `declared` or is a dot-subdomain of it
/// (`evil-example.com` does NOT match `example.com`).
fn host_in_scope(host: &str, declared: &str) -> bool {
    let declared = declared.to_ascii_lowercase();
    host == declared || host.ends_with(&format!(".{declared}"))
}

/// Enforce the task's navigation scope: only `http`/`https` to declared
/// hosts. `file:`, `data:`, unknown schemes, and off-allowlist hosts are
/// refused with [`BridgeError::NavigationDenied`] before the port is
/// ever touched.
pub fn check_navigation(url: &str, allowed_hosts: &[String]) -> Result<(), BridgeError> {
    let denied = |reason: &str| BridgeError::NavigationDenied {
        url: url.to_string(),
        reason: reason.to_string(),
    };
    let (scheme, host) = split_url(url).ok_or_else(|| denied("unparseable URL"))?;
    if scheme != "http" && scheme != "https" {
        return Err(denied(&format!("scheme '{scheme}' is not http/https")));
    }
    if !allowed_hosts.iter().any(|d| host_in_scope(&host, d)) {
        return Err(denied(&format!(
            "host '{host}' is outside the declared scope"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The bridge: one task, one process, one debugging-port target.
// ---------------------------------------------------------------------------

/// Which fixture stub stands in for the real bridge binary.
///
/// MOCK processes: the protocol half of this harness does not need the
/// real `phlow browser-bridge` binary; what it needs is a real OS child
/// with genuine spawn/kill/reap semantics, which these stubs provide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StubKind {
    /// `sleep 300`: dies on SIGTERM.
    Sleep,
    /// `python3` ignoring SIGTERM: only SIGKILL reaps it (task 183 A2).
    IgnoreSigterm,
    /// Real headless chromium with a remote debugging port (task 178
    /// integration). The bridge owns the browser process exactly like
    /// a stub: shutdown closes the target, then reaps chromium.
    Chromium {
        /// 127.0.0.1 debugging port chromium listens on.
        port: u16,
        /// Fresh profile dir (created if missing).
        profile_dir: PathBuf,
    },
}

/// Per-task bridge configuration.
pub struct BridgeConfig {
    /// Task this bridge serves; prefixes the target-id namespace.
    pub task_id: String,
    /// Hosts the task may browse (exact or dot-subdomain).
    pub allowed_hosts: Vec<String>,
    /// Whether the task declared multi-target (window.open adoption).
    pub allow_multi_target: bool,
    /// Max live targets for this bridge.
    pub max_targets: usize,
    /// Which fixture process to spawn.
    pub stub: StubKind,
}

impl BridgeConfig {
    /// Single-target task scoped to `allowed_hosts`.
    pub fn single_task(task_id: &str, allowed_hosts: Vec<String>) -> Self {
        BridgeConfig {
            task_id: task_id.to_string(),
            allowed_hosts,
            allow_multi_target: false,
            max_targets: MAX_TARGETS_DEFAULT,
            stub: StubKind::Sleep,
        }
    }

    /// Real-chromium task: the bridge spawns headless chromium with a
    /// debugging port and owns the browser process. `profile_dir` must
    /// be a fresh temp dir per task (chromium locks its profile).
    pub fn chromium_task(
        task_id: &str,
        allowed_hosts: Vec<String>,
        port: u16,
        profile_dir: PathBuf,
    ) -> Self {
        BridgeConfig {
            task_id: task_id.to_string(),
            allowed_hosts,
            allow_multi_target: false,
            max_targets: MAX_TARGETS_DEFAULT,
            stub: StubKind::Chromium { port, profile_dir },
        }
    }
}

/// Outcome of tearing a bridge down.
pub struct ShutdownReport {
    /// The bridge child's PID.
    pub pid: u32,
    /// True when SIGTERM was not enough and SIGKILL was used.
    pub escalated: bool,
    /// True when the child was reaped (no zombie).
    pub reaped: bool,
    /// DevTools targets closed.
    pub targets_closed: usize,
    /// Externally created targets refused during the bridge's life.
    pub external_refusals: u64,
}

/// Outcome of the kill half of teardown.
pub struct KillReport {
    /// The child PID.
    pub pid: u32,
    /// Whether the SIGTERM step was attempted.
    pub sigterm_sent: bool,
    /// Whether SIGKILL escalation fired.
    pub escalated_to_sigkill: bool,
    /// Whether the child was reaped before the timeout.
    pub reaped: bool,
    /// Wall time spent.
    pub elapsed: Duration,
}

/// True when chromium's debugging port answers `/json/version`.
/// Raw TCP + minimal HTTP: no extra client, bounded by the caller.
/// The request MUST be HTTP/1.1 with a Host header: verified
/// 2026-09-28 against Chromium 153.0.8010.52 — bare `HTTP/1.0`
/// requests (even with Host) are dropped with an empty close.
fn chromium_debugging_ready(port: u16) -> bool {
    let addr = format!("127.0.0.1:{port}");
    let Ok(mut stream) = std::net::TcpStream::connect(&addr) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let request = format!(
        "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 128];
    let Ok(n) = stream.read(&mut buf) else {
        return false;
    };
    String::from_utf8_lossy(&buf[..n]).contains("200")
}

/// Spawn the SIGTERM-ignoring stub and wait (bounded) for its
/// readiness line. The child prints `ready` only after installing
/// SIG_IGN, so a later SIGTERM cannot win the race against
/// interpreter startup — the failure mode that flaked task-183 A2
/// under parallel load. On timeout the child is killed and reaped so
/// a failed launch never orphans it.
fn spawn_ignore_sigterm() -> Result<Child, BridgeError> {
    let mut child = Command::new("python3")
        .arg("-c")
        .arg(
            "import signal, sys, time; \
             signal.signal(signal.SIGTERM, signal.SIG_IGN); \
             sys.stdout.write('ready\\n'); sys.stdout.flush(); \
             time.sleep(300)",
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| BridgeError::SpawnFailed {
            detail: format!("python3: {e}"),
        })?;
    let stdout = child.stdout.take().expect("stdout was piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    match rx.recv_timeout(IGNORE_SIGTERM_READY_TIMEOUT) {
        Ok(line) if line.trim_end() == "ready" => Ok(child),
        _ => {
            let _ = terminate_child(&mut child);
            Err(BridgeError::SpawnFailed {
                detail: "SIGTERM-ignoring stub never signaled ready".to_string(),
            })
        }
    }
}

/// Spawn the fixture bridge child. The child is a real OS process; the
/// binary is a stub (see [`StubKind`]), honestly labeled MOCK — except
/// [`StubKind::Chromium`], which is the real browser for the task-178
/// integration half.
fn spawn_stub(stub: &StubKind) -> Result<Child, BridgeError> {
    let spawn_failed = |detail: String| BridgeError::SpawnFailed { detail };
    let mut child = match stub {
        StubKind::Sleep => Command::new("sleep")
            .arg("300")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| spawn_failed(format!("sleep: {e}")))?,
        StubKind::IgnoreSigterm => spawn_ignore_sigterm()?,
        StubKind::Chromium { port, profile_dir } => {
            std::fs::create_dir_all(profile_dir)
                .map_err(|e| spawn_failed(format!("chromium profile dir: {e}")))?;
            // Flags verified against real chromium on 2026-09-28
            // (Chromium 153.0.8010.52): each one was probed to execute.
            let child = Command::new(CHROMIUM_BIN)
                .arg("--headless=new")
                .arg("--no-sandbox")
                .arg("--disable-gpu")
                .arg(format!("--remote-debugging-port={port}"))
                .arg(format!("--user-data-dir={}", profile_dir.display()))
                .arg("about:blank")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| spawn_failed(format!("chromium: {e}")))?;
            // The debugging port is not up the moment the process
            // spawns. Poll with a hard deadline; on timeout kill and
            // reap so a failed launch never orphans chromium.
            let start = Instant::now();
            let mut ready = false;
            while start.elapsed() < CHROMIUM_READY_TIMEOUT {
                if chromium_debugging_ready(*port) {
                    ready = true;
                    break;
                }
                std::thread::sleep(CHROMIUM_READY_POLL);
            }
            if !ready {
                let mut child = child;
                let _ = terminate_child(&mut child);
                return Err(spawn_failed(format!(
                    "debugging port {port} never answered"
                )));
            }
            child
        }
    };
    // Reap check: the child must be alive after spawn.
    match child.try_wait() {
        Ok(None) => Ok(child),
        _ => Err(spawn_failed(format!("{stub:?} exited immediately"))),
    }
}

/// Kill `child`: SIGTERM via `/usr/bin/kill` (`Child::kill` is SIGKILL
/// on Unix and skips the graceful step), escalate to SIGKILL after
/// [`SIGTERM_GRACE`], give up after [`BRIDGE_KILL_TIMEOUT`].
pub fn terminate_child(child: &mut Child) -> KillReport {
    let pid = child.id();
    let start = Instant::now();
    let sigterm_sent = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let mut escalated = false;
    let mut reaped = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                reaped = true;
                break;
            }
            Ok(None) => {}
            Err(_) => break,
        }
        let elapsed = start.elapsed();
        if !escalated && (!sigterm_sent || elapsed >= SIGTERM_GRACE) {
            let _ = child.kill();
            escalated = true;
        }
        if elapsed >= BRIDGE_KILL_TIMEOUT {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !reaped {
        reaped = matches!(child.try_wait(), Ok(Some(_)));
    }
    KillReport {
        pid,
        sigterm_sent,
        escalated_to_sigkill: escalated,
        reaped,
        elapsed: start.elapsed(),
    }
}

/// The ephemeral bridge: owns the child process and its DevTools
/// target(s) exactly once. Consuming `shutdown`/`cancel` is the only
/// way to release; `Drop` best-effort kills a leaked bridge.
pub struct Bridge {
    child: Child,
    pid: u32,
    config: BridgeConfig,
    port: Box<dyn DevToolsPort>,
    raw_id: String,
    target_id: String,
    extra: Vec<(String, String)>,
    framer: StdioFramer,
    external_refusals: u64,
    last_meter_bytes: usize,
    dead: bool,
}

impl Bridge {
    /// Launch: spawn the child, open exactly one target, namespace it.
    pub fn launch(
        config: BridgeConfig,
        mut port: Box<dyn DevToolsPort>,
    ) -> Result<Self, BridgeError> {
        let mut child = spawn_stub(&config.stub)?;
        let pid = child.id();
        let info = match port.create_target("about:blank") {
            Ok(info) => info,
            Err(e) => {
                let _ = terminate_child(&mut child);
                return Err(e);
            }
        };
        let target_id = format!("{}:{}", config.task_id, info.raw_id);
        Ok(Bridge {
            child,
            pid,
            config,
            port,
            raw_id: info.raw_id,
            target_id,
            extra: Vec::new(),
            framer: StdioFramer::new(),
            external_refusals: 0,
            last_meter_bytes: 0,
            dead: false,
        })
    }

    /// The child PID.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// This bridge's namespaced target id (`<task-id>:<raw-id>`).
    pub fn target_id(&self) -> &str {
        &self.target_id
    }

    /// All live target ids in this bridge's namespace.
    pub fn known_target_ids(&self) -> Vec<String> {
        let mut ids = vec![self.target_id.clone()];
        ids.extend(self.extra.iter().map(|(namespaced, _)| namespaced.clone()));
        ids
    }

    /// Live target count; never exceeds the declared bound.
    pub fn target_count(&self) -> usize {
        1 + self.extra.len()
    }

    /// Externally created targets refused so far.
    pub fn external_refusals(&self) -> u64 {
        self.external_refusals
    }

    /// Bytes the last `evaluate` copied into agent context.
    pub fn last_meter_bytes(&self) -> usize {
        self.last_meter_bytes
    }

    /// Bytes of one newline-delimited stdio message.
    pub fn pending_bytes(&self) -> usize {
        self.framer.pending_bytes()
    }

    /// `Target.created`-equivalent events the port emitted.
    pub fn port_created_events(&self) -> u64 {
        self.port.created_events()
    }

    /// Targets currently on the port.
    pub fn port_target_count(&self) -> usize {
        self.port.targets().len()
    }

    /// True while the child has not been reaped.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn resolve_target(&self, target_id: &str) -> Result<String, BridgeError> {
        if target_id == self.target_id {
            return Ok(self.raw_id.clone());
        }
        for (namespaced, raw) in &self.extra {
            if namespaced == target_id {
                return Ok(raw.clone());
            }
        }
        Err(BridgeError::UnknownTarget {
            id: target_id.to_string(),
        })
    }

    /// MCP `initialize` handshake over the stdio framing contract.
    /// The bridge's scripted responder answers; the request/response
    /// both cross the newline-delimited framing layer.
    pub fn mcp_initialize(&mut self) -> Result<serde_json::Value, BridgeError> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "clientInfo": {"name": "phlow-browser-bridge", "version": "0.1.0"},
            },
        });
        let mut line = serde_json::to_string(&request).map_err(|e| BridgeError::DevTools {
            detail: format!("initialize request would not serialize: {e}"),
        })?;
        line.push('\n');
        let messages = self.framer.push_chunk(line.as_bytes())?;
        if messages.len() != 1 {
            return Err(BridgeError::DevTools {
                detail: format!("initialize framed {} messages, want 1", messages.len()),
            });
        }
        Ok(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "2024-11-05",
                "serverInfo": {"name": "phlow-browser-bridge", "version": "0.1.0"},
                "capabilities": {"tools": {"listChanged": false}},
            },
        }))
    }

    /// `Runtime.evaluate` on this bridge's own target → MCP content.
    pub fn evaluate(&mut self, expr: &str) -> Result<McpContent, BridgeError> {
        let target_id = self.target_id.clone();
        self.evaluate_on(&target_id, expr)
    }

    /// `Runtime.evaluate` on a named target id. Ids outside this
    /// bridge's namespace → [`BridgeError::UnknownTarget`].
    pub fn evaluate_on(&mut self, target_id: &str, expr: &str) -> Result<McpContent, BridgeError> {
        let raw = self.resolve_target(target_id)?;
        let result = self.port.evaluate(&raw, expr)?;
        let mut meter = AllocMeter::new();
        let content = ingest_remote(&result, &mut meter)?;
        self.last_meter_bytes = meter.used();
        Ok(content)
    }

    /// Navigate this bridge's target, allowlisted first: the port is
    /// never touched for a denied URL.
    pub fn navigate(&mut self, url: &str) -> Result<(), BridgeError> {
        check_navigation(url, &self.config.allowed_hosts)?;
        let raw = self.raw_id.clone();
        self.port.navigate(&raw, url)
    }

    /// Drain port-side `Target.created` events the bridge did not ask
    /// for (page-side `window.open`). Adopted only when the task
    /// declared multi-target and the bound allows; otherwise refused,
    /// closed, and counted.
    pub fn drain_external_events(&mut self) -> Vec<BridgeError> {
        let mut refusals = Vec::new();
        while let Some(info) = self.port.take_external_target() {
            let live = self.target_count();
            if self.config.allow_multi_target && live < self.config.max_targets {
                let namespaced = format!("{}:{}", self.config.task_id, info.raw_id);
                self.extra.push((namespaced, info.raw_id));
            } else {
                let _ = self.port.close_target(&info.raw_id);
                self.external_refusals += 1;
                let refusal = if self.config.allow_multi_target {
                    BridgeError::TargetLimitExceeded {
                        limit: self.config.max_targets,
                    }
                } else {
                    BridgeError::TargetRefused {
                        raw_id: info.raw_id,
                    }
                };
                refusals.push(refusal);
            }
        }
        refusals
    }

    fn shutdown_inner(mut self) -> ShutdownReport {
        let raw_ids: Vec<String> = std::iter::once(self.raw_id.clone())
            .chain(self.extra.iter().map(|(_, raw)| raw.clone()))
            .collect();
        let mut closed = 0;
        for raw in &raw_ids {
            if self.port.close_target(raw).is_ok() {
                closed += 1;
            }
        }
        let kill = terminate_child(&mut self.child);
        self.dead = true;
        ShutdownReport {
            pid: kill.pid,
            escalated: kill.escalated_to_sigkill,
            reaped: kill.reaped,
            targets_closed: closed,
            external_refusals: self.external_refusals,
        }
    }

    /// Benign task end: close targets, then reap the child.
    pub fn shutdown(self) -> ShutdownReport {
        self.shutdown_inner()
    }

    /// Task cancellation: abort any in-flight evaluate, then tear down.
    pub fn cancel(mut self) -> ShutdownReport {
        self.port.set_cancel();
        self.shutdown_inner()
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        if self.dead {
            return;
        }
        // Panic/unwind path only: normal teardown goes through
        // `shutdown_inner`, which fully reaps via `terminate_child`.
        // Best-effort *bounded* reap so a leaked bridge cannot linger
        // as a zombie or an orphaned browser.
        let _ = self.child.kill();
        let start = Instant::now();
        while start.elapsed() < DROP_REAP_TIMEOUT {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(DROP_REAP_POLL);
        }
        self.dead = true;
    }
}

// ---------------------------------------------------------------------------
// Process census (/proc) and the MCP config surface.
// ---------------------------------------------------------------------------

/// One process as seen in `/proc`.
pub struct ProcEntry {
    /// PID.
    pub pid: u32,
    /// Single-letter state from `/proc/<pid>/stat` (`Z` = zombie).
    pub state: char,
    /// Command line with NULs replaced by spaces.
    pub cmdline: String,
}

fn proc_one(pid: u32) -> (char, String) {
    let state = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|s| {
            s.rfind(')')
                .map(|i| s[i + 2..].chars().next().unwrap_or('?'))
        })
        .unwrap_or('?');
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|b| {
            String::from_utf8_lossy(&b)
                .replace('\0', " ")
                .trim()
                .to_string()
        })
        .unwrap_or_default();
    (state, cmdline)
}

/// Snapshot of the process table (Linux `/proc`).
pub fn process_census() -> Vec<ProcEntry> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let (state, cmdline) = proc_one(pid);
        out.push(ProcEntry {
            pid,
            state,
            cmdline,
        });
    }
    out
}

/// PIDs currently in zombie state.
pub fn zombie_pids() -> Vec<u32> {
    process_census()
        .into_iter()
        .filter(|e| e.state == 'Z')
        .map(|e| e.pid)
        .collect()
}

/// Byte snapshot of a directory tree: relative path → sha256.
/// Iterative walk (no recursion).
pub fn snapshot_dir(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut stack: VecDeque<std::path::PathBuf> = VecDeque::from([dir.to_path_buf()]);
    while let Some(current) = stack.pop_front() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push_back(path);
                continue;
            }
            let rel = path
                .strip_prefix(dir)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let hash = std::fs::read(&path)
                .map(|bytes| sha256_hex(&bytes))
                .unwrap_or_else(|_| "unreadable".to_string());
            out.insert(rel, hash);
        }
    }
    out
}

static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A fresh unique temp dir path (created on disk).
pub fn next_temp_dir(prefix: &str) -> std::path::PathBuf {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{n}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// In-memory double of the MCP server registry. The bridge under test
/// never writes here — or to any config dir — which is exactly what
/// task 185 asserts.
pub struct McpRegistry {
    entries: BTreeMap<String, String>,
}

impl McpRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        McpRegistry {
            entries: BTreeMap::new(),
        }
    }

    /// Registered server names. Must stay empty across bridge runs.
    pub fn entries(&self) -> &BTreeMap<String, String> {
        &self.entries
    }

    /// The registry's write path (models the real one; the bridge
    /// under test never calls it).
    pub fn register(&mut self, name: &str, command: &str) {
        self.entries.insert(name.to_string(), command.to_string());
    }
}

impl Default for McpRegistry {
    fn default() -> Self {
        Self::new()
    }
}
