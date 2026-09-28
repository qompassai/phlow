// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Phone pairing for the phlow daemon (gauntlet wave 28, tasks 170–177).
//!
//! Adapted concepts (from Ghostex `server/src/remote_access/{pairing_code,
//! pair_device}.rs` and `server/src/tailcat/supervisor.rs`, re-implemented
//! — never ported): Easy-Connect-style one-time pairing codes, a 15-minute
//! TTL, secrets that are hash-compared and single-use, a 5-attempts/minute
//! rate limit, and a supervised sidecar process whose death never takes the
//! daemon down.
//!
//! What is ours: the `phlow-ec1:` prefix, the JSON payload shape, the
//! HMAC-SHA256 instance-key binding (Ghostex keeps the live plaintext in
//! memory; we bind each code to the daemon instance key instead, so a code
//! minted by another daemon is structurally refused), per-code (not
//! per-process) rate-limit scoping, and the loopback/remote path split.
//!
//! Mock boundaries, stated plainly: the loopback harness stands in for the
//! Tailscale transport (real-transport behavior is out of scope); OS
//! entropy comes from `/dev/urandom` and issuance fails closed when it is
//! unavailable; the sidecar is driven by scripted fixture binaries, not a
//! real Tailscale daemon.
//!
//! Security contracts (all asserted by the wave-28 drivers):
//! - The store holds SHA-256(secret), never the secret.
//! - The presented secret is compared in constant time; the caller's
//!   buffer is zeroized on every return path.
//! - Forgery (bad prefix, bad base64url, bad JSON, bad MAC, foreign
//!   instance key) is refused before any secret is checked.
//! - A consumed code never pairs again; concurrent verifications
//!   serialize on one mutex, so exactly one wins.
//! - Rate-limited attempts never reach the secret check.
//! - The audit log records labels and outcomes, never secrets.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

/// Wire prefix pinned on every code this daemon mints or accepts.
pub const CODE_PREFIX: &str = "phlow-ec1:";
/// Code format version.
pub const CODE_VERSION: u8 = 1;
/// Pairing-code time to live, seconds (15 minutes, Ghostex's TTL).
pub const TTL_SECS: u64 = 900;
/// Rate-limit: at most this many verify attempts per code per window.
pub const RATE_LIMIT_MAX_ATTEMPTS: usize = 5;
/// Rate-limit window, seconds.
pub const RATE_LIMIT_WINDOW_SECS: u64 = 60;
/// Device-label bound, characters.
pub const LABEL_MAX_CHARS: usize = 80;
/// Pairing-secret length, bytes (base64url-encoded on the wire).
pub const SECRET_LEN_BYTES: usize = 32;
/// Code-id length, bytes.
pub const CODE_ID_LEN_BYTES: usize = 16;
/// Daemon instance-key length, bytes.
pub const INSTANCE_KEY_LEN_BYTES: usize = 32;
/// Consecutive sidecar failures tolerated before the supervisor parks it.
pub const MAX_SIDECAR_RESTARTS: u32 = 3;
/// Upper bound on one sidecar tunnel round-trip.
pub const TUNNEL_ROUNDTRIP_TIMEOUT: Duration = Duration::from_secs(2);
/// Longest single tunnel line the pump thread forwards.
pub const MAX_TUNNEL_LINE_BYTES: usize = 4096;

/// Typed pairing failures. Ordering in `verify_inner` is load-bearing:
/// [`PairingError::Authenticity`] precedes the store lookup, and
/// [`PairingError::RateLimited`] precedes the secret comparison, so
/// forgery and throttled attempts never reach the hash check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingError {
    /// Bad prefix, bad base64url, bad JSON, or missing/mistyped fields.
    Malformed,
    /// MAC check failed: tampered payload or a code minted by a
    /// different daemon instance.
    Authenticity,
    /// Well-formed and authentic, but the code id is not in the store.
    Unknown,
    /// Presented at or after `issued_at + TTL_SECS`.
    Expired,
    /// Already used to pair a device.
    Consumed,
    /// `RATE_LIMIT_MAX_ATTEMPTS` verify attempts inside the window.
    RateLimited,
    /// Secret hash did not match.
    Mismatch,
    /// Remote path with no live sidecar.
    SidecarDown,
    /// Device label empty or longer than `LABEL_MAX_CHARS`.
    BadLabel,
    /// OS randomness unavailable; issuance fails closed.
    Entropy,
}

/// Typed daemon-call failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonError {
    /// No device registered under this id.
    UnknownDevice,
}

/// One issued code: the wire string plus the secret the phone must
/// present. The secret is plaintext here by construction — it is the
/// phone's credential — and the caller zeroizes it when done.
pub struct IssuedCode {
    pub code: String,
    pub secret: Vec<u8>,
    pub code_id: String,
    pub expires_at: u64,
}

/// SHA-256 of arbitrary bytes. `pub(crate)` for the drivers'
/// timing/zeroization instrumentation.
pub(crate) fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// SHA-256 hex of arbitrary bytes. Public so drivers can build the
/// positive control for the never-plaintext store scan.
pub fn sha256_hex(data: &[u8]) -> String {
    hex_encode(&sha256_bytes(data))
}

/// Hex-encode bytes without hashing. The store dump renders the
/// stored secret hash with this — `sha256_hex` would hash it twice
/// and the positive control would never match.
fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

/// HMAC-SHA256, the standard construction (RFC 2104), hand-rolled
/// over the already-present `sha2` dependency — no new crates.
fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK_LEN: usize = 64;
    let mut k = [0u8; BLOCK_LEN];
    if key.len() > BLOCK_LEN {
        k[..32].copy_from_slice(&sha256_bytes(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK_LEN];
    let mut opad = [0x5cu8; BLOCK_LEN];
    for i in 0..BLOCK_LEN {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(msg);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    let out = outer.finalize();
    let mut mac = [0u8; 32];
    mac.copy_from_slice(&out);
    mac
}

const B64URL_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// base64url without padding, hand-rolled: the crate has no base64
/// dependency and the transform is 64 fixed mappings.
fn base64url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = if chunk.len() > 1 {
            u32::from(chunk[1])
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            u32::from(chunk[2])
        } else {
            0
        };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64URL_ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(B64URL_ALPHABET[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64URL_ALPHABET[((n >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(B64URL_ALPHABET[(n & 63) as usize] as char);
        }
    }
    out
}

fn b64url_val(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    }
}

pub(crate) fn base64url_decode(text: &str) -> Result<Vec<u8>, PairingError> {
    let bytes = text.as_bytes();
    if bytes.len() % 4 == 1 {
        return Err(PairingError::Malformed);
    }
    let mut out = Vec::with_capacity(bytes.len().div_ceil(4) * 3);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, &b) in chunk.iter().enumerate() {
            let v = b64url_val(b).ok_or(PairingError::Malformed)?;
            n |= u32::from(v) << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

/// OS randomness, fail-closed: no fabricated entropy on I/O failure.
fn random_bytes<const N: usize>() -> Result<[u8; N], PairingError> {
    let mut file = std::fs::File::open("/dev/urandom").map_err(|_| PairingError::Entropy)?;
    let mut buf = [0u8; N];
    file.read_exact(&mut buf)
        .map_err(|_| PairingError::Entropy)?;
    Ok(buf)
}

/// Overwrite every byte. Called on the caller's `&mut [u8]`, so the
/// stores are observable through the borrow and cannot be elided.
fn zeroize(buf: &mut [u8]) {
    for byte in buf.iter_mut() {
        *byte = 0;
    }
}

/// Constant-time digest comparison. Visits every byte on every call —
/// no early exit — and reports the visit count so tests assert the
/// property instead of assuming it.
pub(crate) fn ct_compare(a: &[u8; 32], b: &[u8; 32]) -> (bool, u64) {
    let mut diff = 0u8;
    let mut visited = 0u64;
    for i in 0..32 {
        visited += 1;
        diff |= a[i] ^ b[i];
    }
    (diff == 0, visited)
}

/// One live pairing code: the hash, never the secret, plus the
/// per-code attempt ledger for the rate limiter.
struct CodeRecord {
    code_id: String,
    label: String,
    issued_at: u64,
    expires_at: u64,
    secret_hash: [u8; 32],
    consumed: bool,
    attempts: Vec<u64>,
}

/// One paired phone. The id is the map key; the record carries the
/// metadata the daemon otherwise only writes.
struct DeviceRecord {
    label: String,
    paired_at: u64,
}

struct DaemonInner {
    instance_key: [u8; INSTANCE_KEY_LEN_BYTES],
    codes: HashMap<String, CodeRecord>,
    devices: HashMap<String, DeviceRecord>,
    audit: Vec<String>,
    hash_comparisons: u64,
    sidecar: Option<SidecarSupervisor>,
}

/// The phlow daemon side of phone pairing. Shareable across threads
/// (`Arc<Daemon>`): `verify` holds one mutex for the whole
/// lookup → consume → register sequence, so concurrent verifications
/// of one code serialize and exactly one wins.
pub struct Daemon {
    inner: Mutex<DaemonInner>,
}

/// A code payload after parsing and MAC verification, before the
/// store lookup. The store's own timestamps are authoritative, so
/// only the code id travels forward.
struct ParsedCode {
    code_id: String,
}

impl Daemon {
    /// New daemon with a fresh instance key. Codes minted here verify
    /// only here — the cross-instance refusal (task 175) is structural.
    pub fn new() -> Result<Self, PairingError> {
        Ok(Daemon {
            inner: Mutex::new(DaemonInner {
                instance_key: random_bytes()?,
                codes: HashMap::new(),
                devices: HashMap::new(),
                audit: Vec::new(),
                hash_comparisons: 0,
                sidecar: None,
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, DaemonInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Mint a one-time code for `label`. Stores SHA-256(secret) only.
    pub fn issue(&self, label: &str, now: u64) -> Result<IssuedCode, PairingError> {
        let label = label.trim();
        if label.is_empty() || label.chars().count() > LABEL_MAX_CHARS {
            return Err(PairingError::BadLabel);
        }
        let code_id = base64url_encode(&random_bytes::<CODE_ID_LEN_BYTES>()?);
        let secret = base64url_encode(&random_bytes::<SECRET_LEN_BYTES>()?).into_bytes();
        let issued_at = now;
        let expires_at = now.saturating_add(TTL_SECS);
        let mac_input = format!(
            "{}|{code_id}|{label}|{issued_at}|{TTL_SECS}",
            u64::from(CODE_VERSION)
        );
        let mac = {
            let inner = self.lock();
            hmac_sha256(&inner.instance_key, mac_input.as_bytes())
        };
        let payload = serde_json::json!({
            "v": CODE_VERSION,
            "code_id": code_id,
            "label": label,
            "issued_at": issued_at,
            "ttl_secs": TTL_SECS,
            "mac": base64url_encode(&mac),
        });
        let payload_bytes = serde_json::to_vec(&payload).map_err(|_| PairingError::Malformed)?;
        let code = format!("{CODE_PREFIX}{}", base64url_encode(&payload_bytes));
        {
            let mut inner = self.lock();
            inner.codes.insert(
                code_id.clone(),
                CodeRecord {
                    code_id: code_id.clone(),
                    label: label.to_string(),
                    issued_at,
                    expires_at,
                    secret_hash: sha256_bytes(&secret),
                    consumed: false,
                    attempts: Vec::new(),
                },
            );
            inner.audit.push(format!(
                "issued code '{code_id}' label='{label}' expires_at={expires_at}"
            ));
        }
        Ok(IssuedCode {
            code,
            secret,
            code_id,
            expires_at,
        })
    }

    /// Parse, authenticate, and MAC-verify a code string. Reaches no
    /// store and no secret: forgery dies here.
    fn parse_code(&self, code: &str) -> Result<ParsedCode, PairingError> {
        let payload_b64 = code
            .strip_prefix(CODE_PREFIX)
            .ok_or(PairingError::Malformed)?;
        let payload_bytes = base64url_decode(payload_b64)?;
        let value: serde_json::Value =
            serde_json::from_slice(&payload_bytes).map_err(|_| PairingError::Malformed)?;
        let version = value
            .get("v")
            .and_then(serde_json::Value::as_u64)
            .ok_or(PairingError::Malformed)?;
        if version != u64::from(CODE_VERSION) {
            return Err(PairingError::Malformed);
        }
        let code_id = value
            .get("code_id")
            .and_then(serde_json::Value::as_str)
            .ok_or(PairingError::Malformed)?;
        let label = value
            .get("label")
            .and_then(serde_json::Value::as_str)
            .ok_or(PairingError::Malformed)?;
        let issued_at = value
            .get("issued_at")
            .and_then(serde_json::Value::as_u64)
            .ok_or(PairingError::Malformed)?;
        let ttl_secs = value
            .get("ttl_secs")
            .and_then(serde_json::Value::as_u64)
            .ok_or(PairingError::Malformed)?;
        let mac_b64 = value
            .get("mac")
            .and_then(serde_json::Value::as_str)
            .ok_or(PairingError::Malformed)?;
        let mac_bytes = base64url_decode(mac_b64)?;
        let mac: [u8; 32] = mac_bytes.try_into().map_err(|_| PairingError::Malformed)?;
        let mac_input = format!("{version}|{code_id}|{label}|{issued_at}|{ttl_secs}");
        let expected = {
            let inner = self.lock();
            hmac_sha256(&inner.instance_key, mac_input.as_bytes())
        };
        let (ok, _) = ct_compare(&mac, &expected);
        if !ok {
            return Err(PairingError::Authenticity);
        }
        Ok(ParsedCode {
            code_id: code_id.to_string(),
        })
    }

    /// Verify a presented code + secret on the loopback path. The
    /// caller's secret buffer is zeroized on every return path.
    pub fn verify(&self, code: &str, secret: &mut [u8], now: u64) -> Result<String, PairingError> {
        let outcome = self.verify_inner(code, secret, now);
        zeroize(secret);
        outcome
    }

    /// Gate 1: the code exists, is unconsumed, and is unexpired.
    /// Pure over the record — no store mutation, so no borrow
    /// conflicts with the audit log or the device map.
    fn check_consumable(record: &CodeRecord, now: u64) -> Result<(), PairingError> {
        if record.consumed {
            return Err(PairingError::Consumed);
        }
        if now.saturating_sub(record.issued_at) >= TTL_SECS {
            return Err(PairingError::Expired);
        }
        Ok(())
    }

    /// Gate 2: the per-code sliding-window rate limit. The attempt
    /// is recorded only when it is admitted; a refusal returns the
    /// label and code id for the audit entry.
    fn admit_attempt(record: &mut CodeRecord, now: u64) -> Result<(), (String, String)> {
        record
            .attempts
            .retain(|t| *t <= now && now - *t < RATE_LIMIT_WINDOW_SECS);
        if record.attempts.len() >= RATE_LIMIT_MAX_ATTEMPTS {
            Err((record.label.clone(), record.code_id.clone()))
        } else {
            record.attempts.push(now);
            Ok(())
        }
    }

    /// The whole check sequence under one lock: lookup → gates →
    /// compare → consume → register. The mutex is never released
    /// mid-sequence, so concurrent verifications of one code
    /// serialize and exactly one wins (task 174). Record borrows are
    /// scoped to short blocks: borrows through a `MutexGuard` do not
    /// split into disjoint field borrows, so each phase takes what
    /// it needs and ends the borrow before the next phase runs.
    fn verify_inner(&self, code: &str, secret: &[u8], now: u64) -> Result<String, PairingError> {
        let parsed = self.parse_code(code)?;
        let mut inner = self.lock();
        {
            let record = inner
                .codes
                .get(&parsed.code_id)
                .ok_or(PairingError::Unknown)?;
            Self::check_consumable(record, now)?;
        }
        let refused = {
            let record = inner
                .codes
                .get_mut(&parsed.code_id)
                .ok_or(PairingError::Unknown)?;
            Self::admit_attempt(record, now).err()
        };
        if let Some((label, code_id)) = refused {
            inner.audit.push(format!(
                "rate-limited pairing attempt label='{label}' code_id='{code_id}'"
            ));
            return Err(PairingError::RateLimited);
        }
        // Gate 3: the presented secret is hashed, then the two
        // digests are compared in constant time — accept and reject
        // do identical work.
        let mut presented = secret.to_vec();
        let presented_hash = sha256_bytes(&presented);
        zeroize(&mut presented);
        inner.hash_comparisons += 1;
        let (ok, _) = {
            let record = inner
                .codes
                .get(&parsed.code_id)
                .ok_or(PairingError::Unknown)?;
            ct_compare(&presented_hash, &record.secret_hash)
        };
        if !ok {
            return Err(PairingError::Mismatch);
        }
        // Commit: draw the device id BEFORE consuming, so an entropy
        // failure leaves the code live — the pairing never happened,
        // so there is nothing to roll back.
        let device_id = base64url_encode(&random_bytes::<CODE_ID_LEN_BYTES>()?);
        let label = {
            let record = inner
                .codes
                .get_mut(&parsed.code_id)
                .ok_or(PairingError::Unknown)?;
            record.consumed = true;
            record.label.clone()
        };
        inner.devices.insert(
            device_id.clone(),
            DeviceRecord {
                label: label.clone(),
                paired_at: now,
            },
        );
        inner
            .audit
            .push(format!("paired device '{device_id}' label='{label}'"));
        Ok(device_id)
    }

    /// Verify on the remote path, through the sidecar tunnel. The
    /// sidecar must be alive and answer the tunnel round-trip first;
    /// a dead sidecar fails typed [`PairingError::SidecarDown`] with
    /// no blocking beyond [`TUNNEL_ROUNDTRIP_TIMEOUT`].
    pub fn verify_remote(
        &self,
        code: &str,
        secret: &mut [u8],
        now: u64,
    ) -> Result<String, PairingError> {
        let outcome = self.verify_remote_inner(code, secret, now);
        zeroize(secret);
        outcome
    }

    fn verify_remote_inner(
        &self,
        code: &str,
        secret: &mut [u8],
        now: u64,
    ) -> Result<String, PairingError> {
        let supervisor = {
            let inner = self.lock();
            inner.sidecar.clone().ok_or(PairingError::SidecarDown)?
        };
        if !supervisor.is_alive() {
            return Err(PairingError::SidecarDown);
        }
        supervisor.tunnel_roundtrip()?;
        self.verify(code, secret, now)
    }

    /// A call from an already-paired device. Unknown ids are refused;
    /// pairing is the only way in.
    pub fn device_call(&self, device_id: &str, op: &str) -> Result<String, DaemonError> {
        let inner = self.lock();
        if inner.devices.contains_key(device_id) {
            Ok(format!("ok:{op}"))
        } else {
            Err(DaemonError::UnknownDevice)
        }
    }

    /// Metadata for a paired device: `(label, paired_at)`. The read
    /// path for the record fields the daemon otherwise only writes.
    pub fn device_info(&self, device_id: &str) -> Option<(String, u64)> {
        let inner = self.lock();
        inner
            .devices
            .get(device_id)
            .map(|record| (record.label.clone(), record.paired_at))
    }

    /// Drop every code with `expires_at < now`. Returns the count
    /// removed.
    pub fn purge_expired(&self, now: u64) -> usize {
        let mut inner = self.lock();
        let before = inner.codes.len();
        inner.codes.retain(|_, record| record.expires_at >= now);
        before - inner.codes.len()
    }

    /// Deterministic serialization of every stored record: code ids,
    /// labels, timestamps, and the secret *hashes* (hex). The pairing
    /// secrets themselves must never appear here — drivers scan for
    /// them, with the hashes as the positive control.
    pub fn store_dump(&self) -> Vec<u8> {
        let inner = self.lock();
        let mut ids: Vec<&String> = inner.codes.keys().collect();
        ids.sort();
        let mut out = Vec::new();
        for id in ids {
            let record = &inner.codes[id];
            out.extend_from_slice(
                format!(
                    "code_id={} label={} issued_at={} expires_at={} consumed={} secret_hash={}\n",
                    record.code_id,
                    record.label,
                    record.issued_at,
                    record.expires_at,
                    record.consumed,
                    hex_encode(&record.secret_hash),
                )
                .as_bytes(),
            );
        }
        out
    }

    /// Audit entries: labels, code ids, outcomes. Never secrets.
    pub fn audit_log(&self) -> Vec<String> {
        self.lock().audit.clone()
    }

    /// Instrumentation for the rate-limit test: how many secret hash
    /// comparisons actually ran.
    pub fn hash_comparisons(&self) -> u64 {
        self.lock().hash_comparisons
    }

    pub fn device_count(&self) -> usize {
        self.lock().devices.len()
    }

    pub fn code_count(&self) -> usize {
        self.lock().codes.len()
    }
}

// ===== sidecar supervision =====
// Adapted from Ghostex `server/src/tailcat/supervisor.rs`: the daemon
// supervises the sidecar process — restarts on failure with a bounded
// retry count, then parks it. The sidecar's death never takes the
// daemon down. Re-implemented: scripted fixture binaries instead of
// tailcat, an explicit `poll()` step instead of a background thread,
// and a PING/PONG tunnel round-trip as the remote path's liveness
// proof.

/// What to spawn as the sidecar. Tests pass a scripted fixture
/// binary; the supervisor never inspects the binary beyond
/// spawn/wait/kill.
pub struct SidecarSpec {
    pub bin: PathBuf,
    pub args: Vec<String>,
}

/// Supervisor state, observable by tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarStatus {
    Stopped,
    Running,
    Restarting { restarts: u32 },
    Failed { restarts: u32 },
}

struct ChildHandles {
    child: Child,
    // `Option` because the tunnel round-trip takes the handle out
    // while the supervisor lock is released (`ChildStdin` has no
    // `try_clone`) and puts it back afterwards.
    stdin: Option<ChildStdin>,
    lines: Arc<Mutex<mpsc::Receiver<String>>>,
}

struct SupervisorInner {
    spec: SidecarSpec,
    child: Option<ChildHandles>,
    consecutive_failures: u32,
    restarts: u32,
    status: SidecarStatus,
    stopped: bool,
}

/// Supervises one sidecar child: restarts it while
/// `consecutive_failures <= MAX_SIDECAR_RESTARTS`, then parks it in
/// [`SidecarStatus::Failed`]. Cloneable through an inner `Arc`, so the
/// daemon can hand it to the remote verify path without holding its
/// own lock across the tunnel round-trip.
#[derive(Clone)]
pub struct SidecarSupervisor {
    inner: Arc<Mutex<SupervisorInner>>,
    max_restarts: u32,
}

impl SidecarSupervisor {
    pub fn new(spec: SidecarSpec) -> Self {
        SidecarSupervisor {
            inner: Arc::new(Mutex::new(SupervisorInner {
                spec,
                child: None,
                consecutive_failures: 0,
                restarts: 0,
                status: SidecarStatus::Stopped,
                stopped: false,
            })),
            max_restarts: MAX_SIDECAR_RESTARTS,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SupervisorInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Spawn the sidecar. Idempotent while running; a spawn failure
    /// parks the supervisor immediately (nothing to supervise).
    pub fn start(&self) {
        let mut inner = self.lock();
        if inner.stopped || inner.child.is_some() {
            return;
        }
        match Self::spawn_child(&inner.spec) {
            Ok(handles) => {
                inner.child = Some(handles);
                inner.consecutive_failures = 0;
                inner.status = SidecarStatus::Running;
            }
            Err(_) => {
                inner.consecutive_failures = 1;
                inner.status = SidecarStatus::Failed {
                    restarts: inner.restarts,
                };
            }
        }
    }

    /// One supervision step: reap an exited child and either restart
    /// it or park the supervisor. A live child resets the
    /// consecutive-failure count — "consecutive" is literal.
    pub fn poll(&self) {
        let mut inner = self.lock();
        if inner.stopped || matches!(inner.status, SidecarStatus::Failed { .. }) {
            return;
        }
        let alive = match inner.child.as_mut() {
            Some(handles) => matches!(handles.child.try_wait(), Ok(None)),
            None => false,
        };
        if alive {
            // A single alive observation right after a (re)spawn
            // proves nothing — the child may be mid-exec. Forgive
            // the streak only after the child has been seen alive
            // twice in a row: promote a fresh child to Running, and
            // reset only a child that was already Running.
            if matches!(inner.status, SidecarStatus::Running) {
                inner.consecutive_failures = 0;
            } else {
                inner.status = SidecarStatus::Running;
            }
            return;
        }
        inner.child = None;
        inner.consecutive_failures += 1;
        if inner.consecutive_failures > self.max_restarts {
            inner.status = SidecarStatus::Failed {
                restarts: inner.restarts,
            };
            return;
        }
        match Self::spawn_child(&inner.spec) {
            Ok(handles) => {
                inner.restarts += 1;
                inner.child = Some(handles);
                inner.status = SidecarStatus::Restarting {
                    restarts: inner.restarts,
                };
            }
            Err(_) => {
                // Spawn failure counts as this round's failure; the
                // next poll retries. No restart counted, nothing lost.
                inner.status = SidecarStatus::Restarting {
                    restarts: inner.restarts,
                };
            }
        }
    }

    /// Stop supervision and release the child. The daemon keeps
    /// serving loopback clients; only the remote path goes down.
    pub fn stop(&self) {
        let mut inner = self.lock();
        inner.stopped = true;
        if let Some(mut handles) = inner.child.take() {
            let _ = handles.child.kill();
            let _ = handles.child.wait();
        }
        inner.status = SidecarStatus::Stopped;
    }

    /// Test hook: SIGKILL the sidecar child (`Child::kill` is SIGKILL
    /// on Unix) without touching supervision state, so a test can
    /// observe a mid-operation death. Reaping and restart/park happen
    /// on the next `poll`, as with any real crash.
    pub fn kill_child(&self) {
        let mut inner = self.lock();
        if let Some(handles) = inner.child.as_mut() {
            let _ = handles.child.kill();
        }
    }

    /// Non-blocking liveness: true only while a child handle exists
    /// and `try_wait` says it has not exited. Never restarts, never
    /// blocks — the remote path's first gate.
    pub fn is_alive(&self) -> bool {
        let mut inner = self.lock();
        match inner.child.as_mut() {
            Some(handles) => matches!(handles.child.try_wait(), Ok(None)),
            None => false,
        }
    }

    pub fn status(&self) -> SidecarStatus {
        self.lock().status
    }

    pub fn restart_count(&self) -> u32 {
        self.lock().restarts
    }

    /// Tunnel health check for the remote pairing path: write PING to
    /// the child's stdin, wait for PONG on its stdout. Any failure —
    /// dead child, broken pipe, wrong reply, or the timeout — is
    /// [`PairingError::SidecarDown`]. The wait is bounded by
    /// [`TUNNEL_ROUNDTRIP_TIMEOUT`], so a wedged sidecar cannot hang
    /// the caller; a SIGKILL mid-round-trip surfaces as EOF, also
    /// typed, also prompt.
    ///
    /// `ChildStdin` has no `try_clone`, so the handle is taken out of
    /// the supervisor while the lock is released and put back
    /// afterwards. If supervision state changed under us (a
    /// stop/poll raced the round-trip), the handle is simply
    /// dropped — the next round-trip re-checks liveness first.
    pub fn tunnel_roundtrip(&self) -> Result<(), PairingError> {
        let (mut stdin, lines) = {
            let mut inner = self.lock();
            let handles = inner.child.as_mut().ok_or(PairingError::SidecarDown)?;
            if !matches!(handles.child.try_wait(), Ok(None)) {
                return Err(PairingError::SidecarDown);
            }
            let stdin = handles.stdin.take().ok_or(PairingError::SidecarDown)?;
            (stdin, Arc::clone(&handles.lines))
        };
        let outcome = ping_pong(&mut stdin, &lines);
        let mut inner = self.lock();
        if let Some(handles) = inner.child.as_mut() {
            handles.stdin = Some(stdin);
        }
        outcome
    }

    fn spawn_child(spec: &SidecarSpec) -> std::io::Result<ChildHandles> {
        let mut child = Command::new(&spec.bin)
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("sidecar stdin not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("sidecar stdout not piped"))?;
        let (line_tx, line_rx) = mpsc::channel();
        std::thread::spawn(move || pump_lines(stdout, line_tx));
        Ok(ChildHandles {
            child,
            stdin: Some(stdin),
            lines: Arc::new(Mutex::new(line_rx)),
        })
    }
}

/// One PING → PONG exchange, bounded by
/// [`TUNNEL_ROUNDTRIP_TIMEOUT`]. Any failure — broken pipe, wrong
/// reply, timeout, or the pump thread's EOF after a SIGKILL — is
/// [`PairingError::SidecarDown`].
fn ping_pong(
    stdin: &mut ChildStdin,
    lines: &Arc<Mutex<mpsc::Receiver<String>>>,
) -> Result<(), PairingError> {
    writeln!(stdin, "PING").map_err(|_| PairingError::SidecarDown)?;
    let guard = lines
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match guard.recv_timeout(TUNNEL_ROUNDTRIP_TIMEOUT) {
        Ok(line) if line == "PONG" => Ok(()),
        Ok(_) => Err(PairingError::SidecarDown),
        Err(_) => Err(PairingError::SidecarDown),
    }
}

/// Own-and-release: the child is killed and reaped when the last
/// supervisor handle disappears. Temporary clones (e.g. the ones
/// `Daemon::sidecar_poll` makes per call) share the `Arc` and must
/// not kill the child — cleanup runs only when the last `Arc` goes
/// away, i.e. when the owning daemon is dropped. Best-effort and
/// panic-free — `Drop` must not fail.
impl Drop for SupervisorInner {
    fn drop(&mut self) {
        if let Some(mut handles) = self.child.take() {
            let _ = handles.child.kill();
            let _ = handles.child.wait();
        }
    }
}

/// Forwards the child's stdout lines to the channel, one message per
/// line, capped at [`MAX_TUNNEL_LINE_BYTES`]. Exits on EOF, I/O
/// error, a dropped receiver, or an over-long line (protocol
/// violation) — a dead child always releases its pump thread, so no
/// thread outlives the process it serves.
fn pump_lines(stdout: ChildStdout, line_tx: mpsc::Sender<String>) {
    let mut stdout = stdout;
    let mut byte = [0u8; 1];
    let mut line = Vec::with_capacity(64);
    loop {
        match stdout.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0] == b'\n' {
                    let text = String::from_utf8_lossy(&line).into_owned();
                    line.clear();
                    if line_tx.send(text).is_err() {
                        break;
                    }
                } else {
                    line.push(byte[0]);
                    if line.len() > MAX_TUNNEL_LINE_BYTES {
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }
}

impl Daemon {
    /// Attach and start a supervised sidecar. The loopback path
    /// works with or without one; the remote path requires it.
    pub fn attach_sidecar(&self, spec: SidecarSpec) {
        let supervisor = SidecarSupervisor::new(spec);
        supervisor.start();
        self.lock().sidecar = Some(supervisor);
    }

    /// One supervision step on the attached sidecar, if any.
    pub fn sidecar_poll(&self) {
        if let Some(supervisor) = self.lock().sidecar.clone() {
            supervisor.poll();
        }
    }

    pub fn sidecar_status(&self) -> Option<SidecarStatus> {
        self.lock().sidecar.as_ref().map(|s| s.status())
    }

    pub fn sidecar_restart_count(&self) -> u32 {
        self.lock()
            .sidecar
            .as_ref()
            .map(|s| s.restart_count())
            .unwrap_or(0)
    }

    /// Test hook: SIGKILL the attached sidecar's child without
    /// stopping supervision; the next poll reaps as with a real crash.
    pub fn sidecar_kill_child(&self) {
        if let Some(supervisor) = self.lock().sidecar.clone() {
            supervisor.kill_child();
        }
    }

    pub fn sidecar_stop(&self) {
        if let Some(supervisor) = self.lock().sidecar.clone() {
            supervisor.stop();
        }
    }
}
