// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Wire-protocol discipline, adapted from Ghostex `packages/gx-protocol`
//! (study of `lib.rs`, `rpc.rs`, `event.rs`, `de.rs`, `open_enum.rs` @
//! c911466; re-implemented, never ported verbatim).
//!
//! The adapted *rules*:
//!
//! - String enums are open: known variants plus `Other(String)`, so an
//!   unknown value round-trips instead of failing the frame that
//!   carries it (Ghostex `open_enum.rs`).
//! - Unknown fields never fail a parse: `deny_unknown_fields` is never
//!   used (Ghostex `lib.rs` rules).
//! - Parsing is lenient within typed bounds: whitespace and key order
//!   do not matter; counts read leniently (Ghostex `de.rs`).
//! - Every envelope carries a version; the receiver routes on it with
//!   typed errors, never panics, never silent misparse (Ghostex
//!   `rpc.rs` / `event.rs` versioned envelopes).
//!
//! Tiger Style additions: every limit is a named constant, the parser
//! is iterative (no recursion), the size bound is checked before any
//! buffering, and every refusal is a typed error. The Neovim editor
//! socket read path ([`EditorSocket`]) runs this same discipline —
//! there is no second, weaker parser.

use std::sync::atomic::{AtomicBool, Ordering};

/// Largest envelope the parser will materialize: 64 KiB.
pub const MAX_ENVELOPE_BYTES: usize = 65_536;
/// Largest frame the read path will touch: 1 MiB. Checked before any
/// buffering, copying, or UTF-8 validation, so an oversized hostile
/// frame dies before it costs anything.
pub const MAX_FRAME_BYTES: usize = 1_048_576;
/// Deepest JSON nesting the parser accepts. Checked by the iterative
/// pre-scan before `serde_json` parses, so a nesting bomb cannot reach
/// its recursive descent.
pub const MAX_NESTING: u32 = 128;
/// Oldest envelope version this build routes.
pub const MIN_SUPPORTED_VERSION: u64 = 1;
/// Newest envelope version this build routes.
pub const MAX_SUPPORTED_VERSION: u64 = 3;
/// Envelope keys this build understands. Anything else is counted in
/// [`Envelope::unknown_fields`] and dropped — never denied.
const KNOWN_KEYS: [&str; 4] = ["version", "kind", "id", "body"];
/// Bound for diagnostic strings embedded in errors: evidence is
/// diagnostic text, not bulk data.
const MSG_CHARS_MAX: usize = 160;

/// Envelope `kind`: an open string enum.
///
/// Known spellings map to unit variants by exact match; anything else
/// is kept verbatim in [`Kind::Other`]. There is no case folding, no
/// trimming, and no Unicode normalization anywhere on this path: what
/// the peer sent is what `Other` holds, byte for byte.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Liveness probe.
    Ping,
    /// Liveness reply.
    Pong,
    /// A request expecting a response.
    Request,
    /// The response to a request.
    Response,
    /// A one-way event frame.
    Event,
    /// Subscribe to a topic.
    Subscribe,
    /// Unsubscribe from a topic.
    Unsubscribe,
    /// A spelling this build does not know; kept verbatim.
    Other(String),
}

impl Kind {
    /// The wire spelling of this value. For [`Kind::Other`] this is
    /// the exact string the peer sent.
    pub fn as_str(&self) -> &str {
        match self {
            Kind::Ping => "ping",
            Kind::Pong => "pong",
            Kind::Request => "request",
            Kind::Response => "response",
            Kind::Event => "event",
            Kind::Subscribe => "subscribe",
            Kind::Unsubscribe => "unsubscribe",
            Kind::Other(value) => value.as_str(),
        }
    }

    /// Maps a wire spelling to a variant. Never fails and never
    /// normalizes: the match is exact, so `"Ping"`, `" ping"`, and
    /// `"admin_override"` all land in [`Kind::Other`].
    pub fn from_wire(value: &str) -> Self {
        match value {
            "ping" => Kind::Ping,
            "pong" => Kind::Pong,
            "request" => Kind::Request,
            "response" => Kind::Response,
            "event" => Kind::Event,
            "subscribe" => Kind::Subscribe,
            "unsubscribe" => Kind::Unsubscribe,
            other => Kind::Other(other.to_string()),
        }
    }

    /// True for the seven known spellings.
    pub fn is_known(&self) -> bool {
        !matches!(self, Kind::Other(_))
    }

    /// The JSON string value this kind serializes to. Unknown
    /// spellings round-trip byte-identical through here.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::Value::String(self.as_str().to_string())
    }
}

/// Why a frame could not be read as an envelope. Every hostile input
/// in this wave maps to one of these; none of them panics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// The frame is bigger than [`MAX_FRAME_BYTES`]. Reported before
    /// any buffering, copying, or UTF-8 validation.
    TooLarge { got: usize, bound: usize },
    /// Nesting deeper than [`MAX_NESTING`], found by the iterative
    /// pre-scan before `serde_json` parses.
    DepthExceeded { depth: u32, bound: u32 },
    /// The bytes are not valid UTF-8.
    Encoding,
    /// The bytes end mid-string or with brackets left open: a cut-off
    /// frame, not a malformed one.
    Truncated,
    /// Valid UTF-8 and structurally complete, but not an envelope.
    Malformed(String),
    /// The envelope has no string `kind`.
    MissingKind,
}

/// Why an envelope version could not be routed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionError {
    /// No `version` key on the envelope.
    Missing,
    /// Below [`MIN_SUPPORTED_VERSION`]; 0 lands here.
    TooOld { got: u64, min: u64 },
    /// Above [`MAX_SUPPORTED_VERSION`]. Checked before any parser
    /// dispatch, so a huge version never indexes a dispatch table.
    Unsupported { got: u64, max: u64 },
    /// Below the session's negotiated version: a downgrade replay.
    Downgrade { got: u64, session: u64 },
}

/// The editor socket's read-path error: the same two error families as
/// the daemon link, so the socket gets no weaker parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketError {
    Frame(FrameError),
    Version(VersionError),
}

/// A parsed envelope. `body` stays a loose [`serde_json::Value`]:
/// fields whose shape is loose stay loosely typed (Ghostex `lib.rs`
/// rules) instead of failing the frame that carries them.
#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    /// `None` when the peer sent no `version` key; routing then fails
    /// with [`VersionError::Missing`].
    pub version: Option<u64>,
    pub kind: Kind,
    /// Defaults to `""` when absent.
    pub id: String,
    /// Defaults to `null` when absent.
    pub body: serde_json::Value,
    /// Debug metric: how many unrecognized keys were dropped.
    pub unknown_fields: usize,
}

impl Envelope {
    /// Canonical JSON: deterministic key order, known keys only. Two
    /// envelopes that parse to the same value serialize to the same
    /// value here, whatever whitespace or key order the peer used.
    pub fn to_canonical_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        if let Some(v) = self.version {
            map.insert("version".to_string(), serde_json::Value::from(v));
        }
        map.insert("kind".to_string(), self.kind.to_json());
        map.insert("id".to_string(), serde_json::Value::String(self.id.clone()));
        map.insert("body".to_string(), self.body.clone());
        serde_json::Value::Object(map)
    }
}

/// Outcome of the iterative nesting pre-scan.
struct DepthScan {
    /// Deepest nesting seen.
    max_depth: i64,
    /// True when the input ends inside a string literal.
    in_string: bool,
    /// Final bracket depth; positive means unclosed brackets.
    final_depth: i64,
    /// Bytes visited. The scan is one linear pass, so this always
    /// equals the input length.
    bytes_scanned: usize,
}

/// One linear pass over the bytes: tracks string literals (with
/// escapes) so brackets inside strings do not count, and records the
/// deepest nesting. Iterative — no recursion, no allocation.
fn scan_depth(text: &str) -> DepthScan {
    let mut max_depth: i64 = 0;
    let mut depth: i64 = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut bytes_scanned = 0usize;
    for &b in text.as_bytes() {
        bytes_scanned += 1;
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
        } else {
            match b {
                b'"' => in_string = true,
                b'{' | b'[' => {
                    depth += 1;
                    max_depth = max_depth.max(depth);
                }
                b'}' | b']' => depth -= 1,
                _ => {}
            }
        }
    }
    DepthScan {
        max_depth,
        in_string,
        final_depth: depth,
        bytes_scanned,
    }
}

/// Bounds a diagnostic message: evidence is diagnostic text, not bulk
/// data, and error strings must never smuggle a hostile payload into
/// a log unbounded.
fn truncate_msg(msg: &str) -> String {
    let mut out: String = msg.chars().take(MSG_CHARS_MAX).collect();
    if msg.chars().count() > MSG_CHARS_MAX {
        out.push_str("…[truncated]");
    }
    out
}

/// Lenient count read: `3` and `3.0` read as 3; `null`, negatives,
/// non-numbers, and overflow read as `None`. The float-flooring rule
/// is adapted from Ghostex `de.rs` (`number_as_u64`); the `Option`
/// shape is ours, keeping "absent" distinct from "unparseable".
fn lenient_u64(value: &serde_json::Value) -> Option<u64> {
    if let Some(u) = value.as_u64() {
        return Some(u);
    }
    let n = value.as_f64()?;
    if !n.is_finite() || n < 0.0 {
        return None;
    }
    let floored = n.floor();
    if floored > u64::MAX as f64 {
        return None;
    }
    Some(floored as u64)
}

/// Builds the typed envelope from a JSON value. Unknown keys are
/// counted and dropped; the `deny_unknown_fields` attribute appears
/// nowhere in this module.
fn envelope_from_value(value: serde_json::Value) -> Result<Envelope, FrameError> {
    let obj = match value {
        serde_json::Value::Object(map) => map,
        _ => {
            return Err(FrameError::Malformed(
                "envelope must be a JSON object".to_string(),
            ));
        }
    };
    let mut unknown_fields = 0usize;
    for key in obj.keys() {
        if !KNOWN_KEYS.contains(&key.as_str()) {
            unknown_fields += 1;
        }
    }
    let version = match obj.get("version") {
        None => None,
        Some(v) => Some(lenient_u64(v).ok_or_else(|| {
            FrameError::Malformed("envelope 'version' must be a number".to_string())
        })?),
    };
    let kind = match obj.get("kind") {
        None => return Err(FrameError::MissingKind),
        Some(serde_json::Value::String(s)) => Kind::from_wire(s),
        Some(_) => {
            return Err(FrameError::Malformed(
                "envelope 'kind' must be a string".to_string(),
            ));
        }
    };
    let id = match obj.get("id") {
        Some(serde_json::Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    let body = obj.get("body").cloned().unwrap_or(serde_json::Value::Null);
    Ok(Envelope {
        version,
        kind,
        id,
        body,
        unknown_fields,
    })
}

/// What one parse measured. The scanner visits each input byte exactly
/// once, so `bytes_scanned` always equals the trimmed input length —
/// the linearity evidence for task-153.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseStats {
    pub bytes_scanned: usize,
}

/// Reads one envelope frame. Check order is the security contract:
///
/// 1. size bound — before any buffering, copying, or UTF-8 work;
/// 2. UTF-8 validity;
/// 3. iterative nesting pre-scan — before `serde_json` parses;
/// 4. JSON shape — unknown fields counted and dropped, never denied.
///
/// Every refusal is a typed [`FrameError`]; nothing here panics on
/// attacker-controlled input.
pub fn parse_envelope(bytes: &[u8]) -> Result<(Envelope, ParseStats), FrameError> {
    // 1. Size first: a 100 MB frame on a 1 MB bound dies here, before
    //    the parser buffers or copies anything.
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge {
            got: bytes.len(),
            bound: MAX_FRAME_BYTES,
        });
    }
    // 2. The wire is UTF-8 JSON.
    let text = std::str::from_utf8(bytes).map_err(|_| FrameError::Encoding)?;
    let text = text.trim();
    // 3. Nesting pre-scan: iterative, so a 10,000-deep bomb cannot
    //    reach serde_json's recursive descent.
    let scan = scan_depth(text);
    if scan.max_depth > i64::from(MAX_NESTING) {
        return Err(FrameError::DepthExceeded {
            depth: scan.max_depth as u32,
            bound: MAX_NESTING,
        });
    }
    // 4. Parse. On failure the pre-scan tells truncation (cut off
    //    mid-string or brackets left open) from malformed input.
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| {
        if scan.in_string || scan.final_depth > 0 {
            FrameError::Truncated
        } else {
            FrameError::Malformed(truncate_msg(&e.to_string()))
        }
    })?;
    let stats = ParseStats {
        bytes_scanned: scan.bytes_scanned,
    };
    Ok((envelope_from_value(value)?, stats))
}

/// Routes an envelope version to its parser generation. The check is
/// an if-chain over ranges — never `parsers[version]` — so an absurd
/// version cannot index a dispatch table or overflow it.
pub fn route_version(version: Option<u64>) -> Result<u64, VersionError> {
    match version {
        None => Err(VersionError::Missing),
        Some(v) if v < MIN_SUPPORTED_VERSION => Err(VersionError::TooOld {
            got: v,
            min: MIN_SUPPORTED_VERSION,
        }),
        Some(v) if v > MAX_SUPPORTED_VERSION => Err(VersionError::Unsupported {
            got: v,
            max: MAX_SUPPORTED_VERSION,
        }),
        Some(v) => Ok(v),
    }
}

/// One peer session's version state. The negotiated version is a
/// high-water mark: it only moves up, so a replayed older version is
/// a [`VersionError::Downgrade`], never a silent parser switch.
#[derive(Debug, Clone)]
pub struct Session {
    negotiated: u64,
    accepted: u64,
}

impl Session {
    /// Opens a session at the negotiated version. The version must
    /// itself be routable.
    pub fn negotiate(version: u64) -> Result<Self, VersionError> {
        let v = route_version(Some(version))?;
        Ok(Session {
            negotiated: v,
            accepted: 0,
        })
    }

    /// Accepts one envelope version for this session. Bounds are
    /// checked before dispatch; a version below the mark is a
    /// downgrade and the session keeps its version.
    pub fn accept(&mut self, version: u64) -> Result<(), VersionError> {
        let v = route_version(Some(version))?;
        if v < self.negotiated {
            return Err(VersionError::Downgrade {
                got: v,
                session: self.negotiated,
            });
        }
        if v > self.negotiated {
            self.negotiated = v;
        }
        self.accepted += 1;
        Ok(())
    }

    /// The session's current (monotonic) version.
    pub fn negotiated(&self) -> u64 {
        self.negotiated
    }

    /// How many envelope versions this session has accepted.
    pub fn accepted(&self) -> u64 {
        self.accepted
    }
}

/// Where an envelope goes. Unknown kinds land in
/// [`Dispatch::Default`] — a dead end, not a backdoor: the match is on
/// the [`Kind`] enum, so no crafted string can reach a known handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatch {
    Ping,
    Pong,
    Request,
    Response,
    Event,
    /// [`Kind::Subscribe`] is the privileged arm: it mutates daemon
    /// state. Only the exact `"subscribe"` spelling reaches it.
    Subscribe,
    Unsubscribe,
    /// The default handler. Every [`Kind::Other`] lands here with its
    /// payload verbatim.
    Default {
        raw: String,
    },
}

/// Dispatches on the parsed kind. The match arms are the whole
/// dispatch table: there is no string-keyed lookup for an attacker to
/// confuse, and `Other` is never normalized before the match.
pub fn dispatch(kind: &Kind) -> Dispatch {
    match kind {
        Kind::Ping => Dispatch::Ping,
        Kind::Pong => Dispatch::Pong,
        Kind::Request => Dispatch::Request,
        Kind::Response => Dispatch::Response,
        Kind::Event => Dispatch::Event,
        Kind::Subscribe => Dispatch::Subscribe,
        Kind::Unsubscribe => Dispatch::Unsubscribe,
        Kind::Other(raw) => Dispatch::Default { raw: raw.clone() },
    }
}

/// A successfully read editor-socket message: the envelope plus the
/// version it was routed at.
#[derive(Debug, Clone)]
pub struct RoutedEnvelope {
    pub version: u64,
    pub envelope: Envelope,
}

/// The Neovim editor socket read path. It runs the *same*
/// [`parse_envelope`] + [`route_version`] discipline as the daemon
/// link: there is no second, weaker parser. A refused frame increments
/// `rejected` and the socket stays up — refusal is never a
/// disconnect, by construction (no transition sets `alive` to false).
#[derive(Debug, Default)]
pub struct EditorSocket {
    alive: bool,
    received: u64,
    rejected: u64,
}

impl EditorSocket {
    /// Models an accepted connection; the read path under test.
    pub fn new() -> Self {
        EditorSocket {
            alive: true,
            received: 0,
            rejected: 0,
        }
    }

    /// Reads one frame. Malformed or hostile input is a typed
    /// [`SocketError`]; the socket stays up either way.
    pub fn read(&mut self, bytes: &[u8]) -> Result<RoutedEnvelope, SocketError> {
        let (envelope, _stats) = parse_envelope(bytes).map_err(|e| {
            self.rejected += 1;
            SocketError::Frame(e)
        })?;
        let version = route_version(envelope.version).map_err(|e| {
            self.rejected += 1;
            SocketError::Version(e)
        })?;
        self.received += 1;
        Ok(RoutedEnvelope { version, envelope })
    }

    /// The socket is up. Nothing on the read path can change this.
    pub fn is_alive(&self) -> bool {
        self.alive
    }

    /// Frames accepted so far.
    pub fn received(&self) -> u64 {
        self.received
    }

    /// Frames refused with a typed error so far.
    pub fn rejected(&self) -> u64 {
        self.rejected
    }
}

/// Records whether any panic fired while armed. Written by
/// [`arm_panic_trap`]; read by the hostile-input cases.
static PANIC_TRAPPED: AtomicBool = AtomicBool::new(false);

/// Arms a process-wide panic hook that records (but does not
/// suppress) panics. The hostile-input cases arm it, feed
/// attacker-controlled bytes, then assert nothing was trapped: zero
/// panics is the pass criterion.
pub fn arm_panic_trap() {
    PANIC_TRAPPED.store(false, Ordering::SeqCst);
    std::panic::set_hook(Box::new(|_| {
        PANIC_TRAPPED.store(true, Ordering::SeqCst);
    }));
}

/// True when a panic fired since [`arm_panic_trap`].
pub fn panic_trapped() -> bool {
    PANIC_TRAPPED.load(Ordering::SeqCst)
}
