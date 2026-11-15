//! Cues — how servers hand work to each other (INDEX §2.2; PH-K2).
//!
//! A **cue** is a small JSON file in a per-server cue directory
//! (D-1269: the server's own `~/.local/share/<server>/cue/`; for
//! phlow, [`phlow_cue_dir`]). A producer writes a cue into the
//! *consumer's* directory and returns — nobody waits. The payload is
//! stored content-addressed beside the cues and referenced by its
//! SHA-256 hash, never embedded in the cue.
//!
//! Rules, as implemented here:
//!
//! - A cue **expires** (TTL from the run contract's discipline:
//!   `expires = created + ttl`). Expired unconsumed cues are *listed*
//!   and moved aside by [`reap_expired_cues`] — never retried
//!   forever, never waited on.
//! - A cue is **claimed** by atomic rename (`pending/` → `claimed/`);
//!   a second claim fails by name. Two consumers cannot both act.
//! - A consumer answers with a **done cue** (`run-done`), not a reply
//!   message — see [`consume_run_this`], the reference behaviour.
//! - Cues carry `origin_lane` + `content_label` from birth; empty
//!   labels default fail-closed (`nsfw` / `unknown`). A cue whose
//!   content label is not `sfw` is invisible to an SFW-scoped
//!   consumer ([`LaneScope::SfwOnly`]): not listed, not claimable.
//! - A malformed cue file is **refused by name**: reads and listings
//!   fail with the file's name, never a silent skip or a default.
//!
//! # Bounds
//!
//! - [`CUE_ID_CHARS_MAX`], [`CUE_KIND_CHARS_MAX`],
//!   [`CUE_BYTES_MAX`] (one cue file), [`PAYLOAD_BYTES_MAX`],
//!   [`CUES_PER_LIST_MAX`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::run_contract::{now_unix_ms, sha256_hex};

/// Longest cue id, in characters.
pub const CUE_ID_CHARS_MAX: usize = 96;
/// Longest cue kind, in characters (`run-this`, `run-done`, …).
pub const CUE_KIND_CHARS_MAX: usize = 64;
/// Longest server name (`from` / `to`), in characters.
pub const SERVER_NAME_CHARS_MAX: usize = 64;
/// Longest lane label, in characters.
pub const LANE_LABEL_CHARS_MAX: usize = 64;
/// Largest cue file this module will read, in bytes.
pub const CUE_BYTES_MAX: u64 = 16_000;
/// Largest payload this module will write or read, in bytes. Cues
/// hand off references; a payload bigger than this is a file the
/// consumer should be pointed at by other means.
pub const PAYLOAD_BYTES_MAX: u64 = 1_000_000;
/// Most cues one listing returns.
pub const CUES_PER_LIST_MAX: usize = 1024;

/// Fail-closed content label for an unlabelled cue.
pub const UNCLASSIFIED_CONTENT_LABEL: &str = "nsfw";
/// Fail-closed origin lane for an unlabelled cue.
pub const UNKNOWN_ORIGIN_LANE: &str = "unknown";

/// The cue kind that asks a server to run something.
pub const KIND_RUN_THIS: &str = "run-this";
/// The cue kind that answers a `run-this` with verdict + evidence.
pub const KIND_RUN_DONE: &str = "run-done";

static CUE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Which cues a consumer may see, by lane. `SfwOnly` is the posture
/// of an SFW-scoped server: NSFW-labelled cues do not exist for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneScope {
    /// Only cues labelled `sfw` are visible.
    SfwOnly,
    /// All cues are visible (an NSFW-capable or same-lane consumer).
    Unrestricted,
}

impl LaneScope {
    /// True when a cue with this content label is visible.
    pub fn sees(self, content_label: &str) -> bool {
        match self {
            LaneScope::SfwOnly => content_label == "sfw",
            LaneScope::Unrestricted => true,
        }
    }
}

/// One cue, as written and as read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    /// Unique cue id; also the file stem.
    pub id: String,
    /// Cue kind (`run-this`, `run-done`, `measure-this`, …).
    pub kind: String,
    /// Producing server.
    pub from: String,
    /// Consuming server.
    pub to: String,
    /// Unix milliseconds when the cue was written.
    pub created_unix_ms: u64,
    /// Unix milliseconds after which the cue is expired.
    pub expires_unix_ms: u64,
    /// SHA-256 (lowercase hex) of the payload bytes.
    pub payload_hash: String,
    /// Bloch origin lane, from birth.
    pub origin_lane: String,
    /// Bloch content label, from birth.
    pub content_label: String,
}

impl Cue {
    /// True when `now` is at or past this cue's expiry.
    pub fn is_expired(&self, now_unix_ms: u64) -> bool {
        now_unix_ms >= self.expires_unix_ms
    }

    fn to_json(&self) -> String {
        serde_json::json!({
            "id": self.id,
            "kind": self.kind,
            "from": self.from,
            "to": self.to,
            "created_unix_ms": self.created_unix_ms,
            "expires_unix_ms": self.expires_unix_ms,
            "payload_hash": self.payload_hash,
            "origin_lane": self.origin_lane,
            "content_label": self.content_label,
        })
        .to_string()
    }

    fn from_json(text: &str, file_name: &str) -> Result<Cue, CueError> {
        let value: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| CueError::malformed(file_name, &e.to_string()))?;
        let get_str = |key: &str| -> Result<String, CueError> {
            value
                .get(key)
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .ok_or_else(|| CueError::malformed(file_name, &format!("missing {key}")))
        };
        let get_u64 = |key: &str| -> Result<u64, CueError> {
            value
                .get(key)
                .and_then(|v| v.as_u64())
                .ok_or_else(|| CueError::malformed(file_name, &format!("missing {key}")))
        };
        let cue = Cue {
            id: get_str("id")?,
            kind: get_str("kind")?,
            from: get_str("from")?,
            to: get_str("to")?,
            created_unix_ms: get_u64("created_unix_ms")?,
            expires_unix_ms: get_u64("expires_unix_ms")?,
            payload_hash: get_str("payload_hash")?,
            origin_lane: get_str("origin_lane")?,
            content_label: get_str("content_label")?,
        };
        validate_label("kind", &cue.kind, CUE_KIND_CHARS_MAX)?;
        validate_label("from", &cue.from, SERVER_NAME_CHARS_MAX)?;
        validate_label("to", &cue.to, SERVER_NAME_CHARS_MAX)?;
        validate_label("origin_lane", &cue.origin_lane, LANE_LABEL_CHARS_MAX)?;
        validate_label("content_label", &cue.content_label, LANE_LABEL_CHARS_MAX)?;
        if cue.payload_hash.len() != 64 || !cue.payload_hash.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(CueError::malformed(file_name, "payload_hash is not 64 hex"));
        }
        if cue.expires_unix_ms < cue.created_unix_ms {
            return Err(CueError::malformed(file_name, "expiry before creation"));
        }
        Ok(cue)
    }
}

/// A claimed cue plus its verified payload bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedCue {
    /// The claimed cue.
    pub cue: Cue,
    /// Payload bytes, hash-verified against the cue at claim time.
    pub payload: Vec<u8>,
}

/// The verdict a `run-this` handler returns: what the run decided,
/// and the evidence for it. Serialized into the `run-done` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunDone {
    /// Verdict label (`ok`, `failed`, `unverified`, …).
    pub verdict: String,
    /// Evidence text (gate output summary, report pointer, …).
    pub evidence: String,
}

/// Phlow's own cue directory per D-1269:
/// `$XDG_DATA_HOME/phlow/cue`, else `~/.local/share/phlow/cue`.
/// `None` when neither base exists in the environment.
pub fn phlow_cue_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME")
        && !xdg.is_empty()
    {
        return Some(PathBuf::from(xdg).join("phlow").join("cue"));
    }
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("phlow")
            .join("cue")
    })
}

fn pending_dir(dir: &Path) -> PathBuf {
    dir.join("pending")
}

fn claimed_dir(dir: &Path) -> PathBuf {
    dir.join("claimed")
}

fn expired_dir(dir: &Path) -> PathBuf {
    dir.join("expired")
}

fn payload_path(dir: &Path, payload_hash: &str) -> PathBuf {
    dir.join("payloads").join(payload_hash)
}

fn ensure_layout(dir: &Path) -> Result<(), CueError> {
    for sub in [
        pending_dir(dir),
        claimed_dir(dir),
        expired_dir(dir),
        dir.join("payloads"),
    ] {
        std::fs::create_dir_all(sub)?;
    }
    Ok(())
}

fn cue_file(dir: &Path, id: &str) -> PathBuf {
    pending_dir(dir).join(format!("{id}.json"))
}

/// Write a cue into `dir` (the *consumer's* cue directory) and return
/// it. The payload is stored content-addressed first; the cue file is
/// created with `create_new`, so an id collision fails rather than
/// overwriting an unconsumed cue.
#[allow(clippy::too_many_arguments)]
pub fn write_cue(
    dir: &Path,
    kind: &str,
    from: &str,
    to: &str,
    ttl_ms: u64,
    payload: &[u8],
    origin_lane: &str,
    content_label: &str,
    created_unix_ms: u64,
) -> Result<Cue, CueError> {
    validate_label("kind", kind, CUE_KIND_CHARS_MAX)?;
    validate_label("from", from, SERVER_NAME_CHARS_MAX)?;
    validate_label("to", to, SERVER_NAME_CHARS_MAX)?;
    if payload.len() as u64 > PAYLOAD_BYTES_MAX {
        return Err(CueError::invalid("payload", "exceeds PAYLOAD_BYTES_MAX"));
    }
    if ttl_ms == 0 {
        return Err(CueError::invalid("ttl_ms", "must be positive"));
    }
    ensure_layout(dir)?;
    let payload_hash = sha256_hex(payload);
    let stored = payload_path(dir, &payload_hash);
    if !stored.exists() {
        let tmp =
            dir.join("payloads")
                .join(format!(".tmp-{}-{}", std::process::id(), payload_hash));
        std::fs::write(&tmp, payload)?;
        std::fs::rename(&tmp, &stored)?;
    }
    let cue = Cue {
        id: format!(
            "cue-{created_unix_ms}-{}-{}",
            std::process::id(),
            CUE_COUNTER.fetch_add(1, Ordering::SeqCst)
        ),
        kind: kind.to_owned(),
        from: from.to_owned(),
        to: to.to_owned(),
        created_unix_ms,
        expires_unix_ms: created_unix_ms.saturating_add(ttl_ms),
        payload_hash,
        origin_lane: if origin_lane.trim().is_empty() {
            UNKNOWN_ORIGIN_LANE.to_owned()
        } else {
            origin_lane.to_owned()
        },
        content_label: if content_label.trim().is_empty() {
            UNCLASSIFIED_CONTENT_LABEL.to_owned()
        } else {
            content_label.to_owned()
        },
    };
    validate_label("origin_lane", &cue.origin_lane, LANE_LABEL_CHARS_MAX)?;
    validate_label("content_label", &cue.content_label, LANE_LABEL_CHARS_MAX)?;
    let path = cue_file(dir, &cue.id);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = options.open(&path)?;
    use std::io::Write;
    file.write_all(cue.to_json().as_bytes())?;
    Ok(cue)
}

/// Read and parse one pending cue file. Malformed content is refused
/// by file name.
fn read_pending(dir: &Path, file_name: &str) -> Result<Cue, CueError> {
    use std::io::Read;
    let path = pending_dir(dir).join(file_name);
    let mut text = String::new();
    std::fs::File::open(&path)?
        .take(CUE_BYTES_MAX + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > CUE_BYTES_MAX {
        return Err(CueError::malformed(file_name, "exceeds CUE_BYTES_MAX"));
    }
    let cue = Cue::from_json(&text, file_name)?;
    if format!("{}.json", cue.id) != file_name {
        return Err(CueError::malformed(
            file_name,
            "cue id does not match file name",
        ));
    }
    Ok(cue)
}

/// All pending cues visible to `scope`, oldest first. A malformed
/// file fails the listing by name — a poisoned directory is an
/// operator problem to see, not a cue to skip silently.
pub fn list_cues(dir: &Path, scope: LaneScope) -> Result<Vec<Cue>, CueError> {
    let mut cues = Vec::new();
    let entries = match std::fs::read_dir(pending_dir(dir)) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(cues),
        Err(e) => return Err(e.into()),
    };
    let mut names: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".json") {
            names.push(name);
        }
    }
    names.sort();
    for name in names {
        let cue = read_pending(dir, &name)?;
        if scope.sees(&cue.content_label) {
            cues.push(cue);
        }
        if cues.len() >= CUES_PER_LIST_MAX {
            break;
        }
    }
    cues.sort_by_key(|cue| cue.created_unix_ms);
    Ok(cues)
}

/// Pending cues whose expiry has passed at `now_unix_ms`, visible to
/// `scope`. Listed, per the contract — the caller decides what the
/// listing means; nothing here waits or retries.
pub fn list_expired(dir: &Path, scope: LaneScope, now_unix_ms: u64) -> Result<Vec<Cue>, CueError> {
    Ok(list_cues(dir, scope)?
        .into_iter()
        .filter(|cue| cue.is_expired(now_unix_ms))
        .collect())
}

/// Move every expired pending cue (any label — expiry is not a lane
/// decision) into `expired/` and return their ids, oldest first.
pub fn reap_expired_cues(dir: &Path, now_unix_ms: u64) -> Result<Vec<String>, CueError> {
    ensure_layout(dir)?;
    let expired = list_expired(dir, LaneScope::Unrestricted, now_unix_ms)?;
    let mut ids = Vec::with_capacity(expired.len());
    for cue in expired {
        let from = cue_file(dir, &cue.id);
        let to = expired_dir(dir).join(format!("{}.json", cue.id));
        std::fs::rename(&from, &to)?;
        ids.push(cue.id);
    }
    Ok(ids)
}

/// Claim one pending cue by id: atomic rename to `claimed/`, then
/// read and hash-verify its payload. A cue already claimed (or never
/// there) fails with [`CueError::AlreadyClaimed`] naming the id; a
/// lane-invisible cue fails with [`CueError::LaneRefused`]; a payload
/// whose bytes no longer match the cue's hash is refused by cue id.
pub fn claim(dir: &Path, cue_id: &str, scope: LaneScope) -> Result<ClaimedCue, CueError> {
    let cue = read_pending(dir, &format!("{cue_id}.json")).map_err(|e| match e {
        CueError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => CueError::AlreadyClaimed {
            cue_id: cue_id.to_owned(),
        },
        other => other,
    })?;
    if !scope.sees(&cue.content_label) {
        return Err(CueError::LaneRefused {
            cue_id: cue_id.to_owned(),
            content_label: cue.content_label.clone(),
        });
    }
    std::fs::create_dir_all(claimed_dir(dir))?;
    let from = cue_file(dir, cue_id);
    let to = claimed_dir(dir).join(format!("{cue_id}.json"));
    std::fs::rename(&from, &to).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CueError::AlreadyClaimed {
                cue_id: cue_id.to_owned(),
            }
        } else {
            CueError::Io(e)
        }
    })?;
    let payload = std::fs::read(payload_path(dir, &cue.payload_hash))?;
    if payload.len() as u64 > PAYLOAD_BYTES_MAX || sha256_hex(&payload) != cue.payload_hash {
        return Err(CueError::malformed(cue_id, "payload hash mismatch"));
    }
    Ok(ClaimedCue { cue, payload })
}

/// The PH-K2 reference behaviour: consume the oldest pending
/// `run-this` cue addressed to `server`, hand its payload to
/// `handler` (the server's own run logic — for phlow, running the
/// named checks the payload names), and answer by writing a
/// `run-done` cue — verdict + evidence — back into the *same*
/// directory, addressed to the original sender, carrying the
/// original cue's lane labels. Returns the done cue, or `None` when
/// no `run-this` is pending. Expired cues are not consumed; they are
/// the reaper's listing, not work.
pub fn consume_run_this(
    dir: &Path,
    server: &str,
    scope: LaneScope,
    ttl_ms: u64,
    handler: impl FnOnce(&Cue, &[u8]) -> RunDone,
) -> Result<Option<Cue>, CueError> {
    let now = now_unix_ms();
    let next = list_cues(dir, scope)?
        .into_iter()
        .find(|cue| cue.kind == KIND_RUN_THIS && cue.to == server && !cue.is_expired(now));
    let Some(next) = next else {
        return Ok(None);
    };
    let claimed = claim(dir, &next.id, scope)?;
    let done = handler(&claimed.cue, &claimed.payload);
    let payload = serde_json::json!({
        "run_this_id": claimed.cue.id,
        "verdict": done.verdict,
        "evidence": done.evidence,
    })
    .to_string();
    let done_cue = write_cue(
        dir,
        KIND_RUN_DONE,
        server,
        &claimed.cue.from,
        ttl_ms,
        payload.as_bytes(),
        &claimed.cue.origin_lane,
        &claimed.cue.content_label,
        now_unix_ms(),
    )?;
    Ok(Some(done_cue))
}

/// Failure modes of the cue primitives.
#[derive(Debug)]
pub enum CueError {
    /// Filesystem I/O failed.
    Io(std::io::Error),
    /// A caller-supplied field failed validation.
    Invalid {
        /// Which field.
        field: &'static str,
        /// Why it was refused.
        reason: String,
    },
    /// A cue file (or its payload) did not parse or violated a bound;
    /// the file or cue is named, per the contract.
    Malformed {
        /// The cue file name (or cue id for payload faults).
        file: String,
        /// What was wrong.
        reason: String,
    },
    /// The cue was already claimed (or never existed).
    AlreadyClaimed {
        /// The cue that could not be claimed.
        cue_id: String,
    },
    /// The cue's content label is outside the consumer's lane scope.
    LaneRefused {
        /// The refused cue.
        cue_id: String,
        /// Its content label.
        content_label: String,
    },
}

impl CueError {
    fn invalid(field: &'static str, reason: &str) -> CueError {
        CueError::Invalid {
            field,
            reason: reason.to_owned(),
        }
    }

    fn malformed(file: &str, reason: &str) -> CueError {
        CueError::Malformed {
            file: file.to_owned(),
            reason: reason.to_owned(),
        }
    }
}

impl std::fmt::Display for CueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CueError::Io(e) => write!(f, "cue I/O error: {e}"),
            CueError::Invalid { field, reason } => {
                write!(f, "cue field {field} refused: {reason}")
            }
            CueError::Malformed { file, reason } => {
                write!(f, "cue {file} refused: {reason}")
            }
            CueError::AlreadyClaimed { cue_id } => {
                write!(f, "cue {cue_id} already claimed or absent")
            }
            CueError::LaneRefused {
                cue_id,
                content_label,
            } => write!(
                f,
                "cue {cue_id} refused: content_label {content_label} is outside the consumer's lane scope"
            ),
        }
    }
}

impl std::error::Error for CueError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CueError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for CueError {
    fn from(e: std::io::Error) -> CueError {
        CueError::Io(e)
    }
}

fn validate_label(field: &'static str, value: &str, max_chars: usize) -> Result<(), CueError> {
    let valid = !value.is_empty()
        && value.chars().count() <= max_chars
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if valid {
        Ok(())
    } else {
        Err(CueError::invalid(
            field,
            "must be a short [A-Za-z0-9._-] label",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "phlow-cue-test-{prefix}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("test setup: create temp dir");
        dir
    }

    #[test]
    fn write_list_claim_round_trip() {
        let dir = temp_dir("roundtrip");
        let cue = write_cue(
            &dir,
            KIND_RUN_THIS,
            "lumen",
            "phlow",
            60_000,
            b"{\"checks\":[\"tests\"]}",
            "qompassai",
            "sfw",
            1_000,
        )
        .unwrap();
        assert_eq!(cue.expires_unix_ms, 61_000);
        assert_eq!(cue.payload_hash, sha256_hex(b"{\"checks\":[\"tests\"]}"));
        let listed = list_cues(&dir, LaneScope::Unrestricted).unwrap();
        assert_eq!(listed, vec![cue.clone()]);
        let claimed = claim(&dir, &cue.id, LaneScope::Unrestricted).unwrap();
        assert_eq!(claimed.cue, cue);
        assert_eq!(claimed.payload, b"{\"checks\":[\"tests\"]}");
        // Claimed cues leave the pending list.
        assert!(list_cues(&dir, LaneScope::Unrestricted).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn second_claim_fails_by_name() {
        let dir = temp_dir("claimrace");
        let cue = write_cue(
            &dir,
            KIND_RUN_THIS,
            "a",
            "phlow",
            60_000,
            b"x",
            "",
            "sfw",
            0,
        )
        .unwrap();
        claim(&dir, &cue.id, LaneScope::Unrestricted).unwrap();
        let second = claim(&dir, &cue.id, LaneScope::Unrestricted);
        assert!(matches!(second, Err(CueError::AlreadyClaimed { .. })));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expired_cues_are_listed_and_reaped_never_waited_on() {
        let dir = temp_dir("expiry");
        let old = write_cue(
            &dir,
            KIND_RUN_THIS,
            "a",
            "phlow",
            100,
            b"old",
            "",
            "sfw",
            1_000,
        )
        .unwrap();
        let fresh = write_cue(
            &dir,
            KIND_RUN_THIS,
            "a",
            "phlow",
            100_000,
            b"new",
            "",
            "sfw",
            1_000,
        )
        .unwrap();
        let expired = list_expired(&dir, LaneScope::Unrestricted, 5_000).unwrap();
        assert_eq!(expired, vec![old.clone()]);
        let reaped = reap_expired_cues(&dir, 5_000).unwrap();
        assert_eq!(reaped, vec![old.id.clone()]);
        let remaining = list_cues(&dir, LaneScope::Unrestricted).unwrap();
        assert_eq!(remaining, vec![fresh]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_cue_is_refused_by_name() {
        let dir = temp_dir("malformed");
        ensure_layout(&dir).unwrap();
        std::fs::write(pending_dir(&dir).join("cue-broken.json"), "{not json").unwrap();
        let error = list_cues(&dir, LaneScope::Unrestricted).unwrap_err();
        assert!(
            error.to_string().contains("cue-broken.json"),
            "refusal names the file: {error}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nsfw_cues_are_invisible_to_sfw_scope() {
        let dir = temp_dir("lanes");
        let nsfw = write_cue(
            &dir,
            KIND_RUN_THIS,
            "a",
            "phlow",
            60_000,
            b"x",
            "ntranch",
            "nsfw",
            0,
        )
        .unwrap();
        write_cue(
            &dir,
            KIND_RUN_THIS,
            "a",
            "phlow",
            60_000,
            b"y",
            "qompassai",
            "sfw",
            0,
        )
        .unwrap();
        // Unlabelled cues default to nsfw and are likewise invisible.
        write_cue(&dir, KIND_RUN_THIS, "a", "phlow", 60_000, b"z", "", "", 0).unwrap();
        let sfw_visible = list_cues(&dir, LaneScope::SfwOnly).unwrap();
        assert_eq!(sfw_visible.len(), 1);
        assert_eq!(sfw_visible[0].content_label, "sfw");
        assert!(matches!(
            claim(&dir, &nsfw.id, LaneScope::SfwOnly),
            Err(CueError::LaneRefused { .. })
        ));
        assert_eq!(list_cues(&dir, LaneScope::Unrestricted).unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn consume_run_this_answers_with_run_done() {
        let dir = temp_dir("runthis");
        let ask = write_cue(
            &dir,
            KIND_RUN_THIS,
            "torogao",
            "phlow",
            60_000,
            b"{\"checks\":[\"tests\"]}",
            "qompassai",
            "sfw",
            now_unix_ms(),
        )
        .unwrap();
        let done = consume_run_this(
            &dir,
            "phlow",
            LaneScope::Unrestricted,
            60_000,
            |cue, payload| {
                assert_eq!(cue.id, ask.id);
                assert_eq!(payload, b"{\"checks\":[\"tests\"]}");
                RunDone {
                    verdict: "ok".to_owned(),
                    evidence: "named check tests: ok".to_owned(),
                }
            },
        )
        .unwrap()
        .expect("a run-done cue is written");
        assert_eq!(done.kind, KIND_RUN_DONE);
        assert_eq!(done.from, "phlow");
        assert_eq!(done.to, "torogao");
        // The done cue's payload carries verdict + evidence.
        let claimed = claim(&dir, &done.id, LaneScope::Unrestricted).unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&claimed.payload).unwrap();
        assert_eq!(payload["verdict"], "ok");
        assert_eq!(payload["evidence"], "named check tests: ok");
        assert_eq!(payload["run_this_id"], ask.id);
        // Nothing left to consume.
        let none = consume_run_this(&dir, "phlow", LaneScope::Unrestricted, 60_000, |_, _| {
            unreachable!("no cue pending")
        })
        .unwrap();
        assert!(none.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampered_payload_is_refused() {
        let dir = temp_dir("tamper");
        let cue = write_cue(
            &dir,
            KIND_RUN_THIS,
            "a",
            "phlow",
            60_000,
            b"real",
            "",
            "sfw",
            0,
        )
        .unwrap();
        std::fs::write(payload_path(&dir, &cue.payload_hash), b"forged").unwrap();
        let error = claim(&dir, &cue.id, LaneScope::Unrestricted).unwrap_err();
        assert!(error.to_string().contains("payload hash mismatch"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
