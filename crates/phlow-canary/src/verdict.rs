//! The verdict: binary, fail-closed, and cached against the exact
//! model artifact hash (design integration point). A verdict earned
//! by one artifact never transfers to another — per the design's
//! pruning-activated trojan note, one byte of difference means the
//! battery runs again.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::CanaryError;

/// Maximum model artifact bytes the hasher will read. Deliberately
/// far beyond any artifact phlow deploys; it exists so the read is
/// bounded, not because artifacts approach it.
pub const MODEL_BYTES_MAX: u64 = 512 * 1024 * 1024 * 1024;

/// Overall verdict: deploy or refuse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// All probes passed. Model may be deployed.
    Deploy,
    /// One or more probes failed. Model must not be deployed.
    /// Lists the failing probe IDs.
    Refuse {
        /// Failing probe ids, in registry order.
        failed: Vec<String>,
    },
}

impl Verdict {
    /// Whether this verdict permits deployment.
    pub fn is_deploy(&self) -> bool {
        matches!(self, Verdict::Deploy)
    }

    /// The wire word for reports and the cache: "deploy" or "refuse".
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Deploy => "deploy",
            Verdict::Refuse { .. } => "refuse",
        }
    }

    /// The failing probe ids, empty for a deploy verdict.
    pub fn failed(&self) -> &[String] {
        match self {
            Verdict::Deploy => &[],
            Verdict::Refuse { failed } => failed,
        }
    }
}

/// SHA-256 of a model artifact, hex-encoded. This hash is the cache
/// key and is recorded in every report.
pub fn model_hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_encode(&digest)
}

/// SHA-256 of a model artifact read from `reader`, bounded by
/// [`MODEL_BYTES_MAX`]. Streaming, with a fixed 8 KiB buffer: the
/// artifact is never held in memory.
pub fn model_hash_reader(mut reader: impl Read) -> Result<String, CanaryError> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    let mut total: u64 = 0;
    loop {
        let read = reader.read(&mut buffer).map_err(|e| CanaryError::Io {
            reason: format!("cannot read model artifact: {e}"),
        })?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or(CanaryError::ModelTooLarge {
                bytes_max: MODEL_BYTES_MAX,
            })?;
        if total > MODEL_BYTES_MAX {
            return Err(CanaryError::ModelTooLarge {
                bytes_max: MODEL_BYTES_MAX,
            });
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_encode(&hasher.finalize()))
}

/// Lowercase hex encoding, written out so the crate needs no hex
/// dependency for one fixed-size digest.
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0F) as usize] as char);
    }
    out
}

/// One cached verdict, as stored on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheEntry {
    model_id: String,
    canary_version: String,
    verdict: String,
    failed: Vec<String>,
    timestamp_unix: u64,
}

/// The on-disk cache shape: model hash to entry.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheFile {
    entries: BTreeMap<String, CacheEntry>,
}

/// Verdicts cached by model artifact hash.
///
/// A lookup hits only when the hash matches exactly *and* the entry
/// was earned under the requested canary version: a new canary
/// version re-runs the battery against every model.
#[derive(Debug, Clone, Default)]
pub struct VerdictCache {
    entries: BTreeMap<String, CacheEntry>,
}

impl VerdictCache {
    /// An empty cache.
    pub fn new() -> Self {
        VerdictCache::default()
    }

    /// Load the cache at `path`; a missing file is an empty cache,
    /// a malformed one is an error (fail-closed: a corrupt cache
    /// must not silently read as "no verdicts, run everything" in
    /// a way that hides tampering — callers see the error).
    pub fn load(path: &Path) -> Result<VerdictCache, CanaryError> {
        if !path.exists() {
            return Ok(VerdictCache::new());
        }
        let text = fs::read_to_string(path).map_err(|e| CanaryError::Io {
            reason: format!("cannot read verdict cache {}: {e}", path.display()),
        })?;
        let file: CacheFile = serde_json::from_str(&text).map_err(|_| CanaryError::Io {
            reason: "verdict cache is not well-formed cache JSON".to_owned(),
        })?;
        Ok(VerdictCache {
            entries: file.entries,
        })
    }

    /// The cached verdict for this exact artifact hash and canary
    /// version, if one was recorded.
    pub fn lookup(&self, model_hash: &str, canary_version: &str) -> Option<Verdict> {
        let entry = self.entries.get(model_hash)?;
        if entry.canary_version != canary_version {
            return None;
        }
        if entry.verdict == "deploy" {
            Some(Verdict::Deploy)
        } else {
            Some(Verdict::Refuse {
                failed: entry.failed.clone(),
            })
        }
    }

    /// Record a verdict under the artifact hash that earned it.
    pub fn record(
        &mut self,
        model_hash: &str,
        model_id: &str,
        canary_version: &str,
        verdict: &Verdict,
        timestamp_unix: u64,
    ) {
        self.entries.insert(
            model_hash.to_owned(),
            CacheEntry {
                model_id: model_id.to_owned(),
                canary_version: canary_version.to_owned(),
                verdict: verdict.as_str().to_owned(),
                failed: verdict.failed().to_vec(),
                timestamp_unix,
            },
        );
    }

    /// Persist the cache atomically (temp file + rename) with
    /// owner-only permissions: the cache is a deployment gate, and a
    /// writable cache is a forgeable "deploy".
    pub fn save(&self, path: &Path) -> Result<(), CanaryError> {
        let file = CacheFile {
            entries: self.entries.clone(),
        };
        let text = serde_json::to_string_pretty(&file).map_err(|e| CanaryError::Io {
            reason: format!("cannot encode verdict cache: {e}"),
        })?;
        write_atomic_owner_only(path, text.as_bytes())
    }
}

/// Write `bytes` to `path` via a sibling temp file and rename, with
/// owner-only (0600) permissions on unix. Shared by the cache and
/// the report writer.
pub(crate) fn write_atomic_owner_only(path: &Path, bytes: &[u8]) -> Result<(), CanaryError> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|e| CanaryError::Io {
        reason: format!("cannot write {}: {e}", tmp.display()),
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let result = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
        if let Err(e) = result {
            let _ = fs::remove_file(&tmp);
            return Err(CanaryError::Io {
                reason: format!("cannot restrict {}: {e}", tmp.display()),
            });
        }
    }
    fs::rename(&tmp, path).map_err(|e| CanaryError::Io {
        reason: format!("cannot rename {} to {}: {e}", tmp.display(), path.display()),
    })?;
    Ok(())
}
