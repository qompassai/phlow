//! Operator registry: who may approve, and with which keys.
//!
//! ELI5: an approval is a permission slip with two wax seals only the
//! operator's signet rings can make — a small classical seal (Ed25519) and
//! a big quantum-proof seal (ML-DSA-65). This module is the guest list: it
//! maps operator names to the *public* seal patterns the promotion gate
//! checks slips against. Anyone not on the list, anyone whose entry is
//! marked revoked, and anyone whose seals do not match is turned away.
//!
//! The registry is the trust root for approvals, so it fails closed:
//!
//! - The file must be owner-only (`0600`); group/world-readable files are
//!   rejected at load. Protect it like a key: owner-only writes, ideally
//!   version-controlled.
//! - Every entry carries a SHA-256 fingerprint over its two public keys,
//!   checked at load. A fingerprint mismatch means the entry was tampered
//!   with, and the whole file is rejected.
//! - Keys are never edited in place. Rotation enrolls a new name (e.g.
//!   `gauntlet-test-operator-2`) and marks the old entry `revoked = true`; revocation
//!   takes effect on the next [`OperatorRegistry::load`], no restart.
//!
//! Default location follows the XDG layout: `$XDG_CONFIG_HOME/phlow/`
//! `operators.toml` (fallback `~/.config/phlow/operators.toml`),
//! overridable with `PHLOW_OPERATORS_FILE` (tests use this). The enrollment
//! ceremony — generating the two keypairs and the out-of-band fingerprint
//! read-back — is documented in `docs/approval-crypto.md`; Matt's own
//! operator key enrolls through that ceremony, never through test code.

use crate::error::ExperimentError;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Bounds (all with units)
// ---------------------------------------------------------------------------

/// Maximum bytes read from a registry file: registries are small.
const REGISTRY_BYTES_MAX: u64 = 1_048_576;
/// Hex characters in an Ed25519 public key (32 bytes).
const ED25519_PK_HEX_CHARS: usize = 64;
/// Hex characters in an ML-DSA-65 public key (1952 bytes).
const MLDSA65_PK_HEX_CHARS: usize = 3_904;
/// Hex characters in a SHA-256 fingerprint (32 bytes).
const FINGERPRINT_HEX_CHARS: usize = 64;
/// Maximum characters in an operator name (matches the approval record).
const OPERATOR_NAME_CHARS_MAX: usize = 64;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// One enrolled operator's public key material.
///
/// Both public keys are stored separately (never concatenated in the
/// file) so a future record format can retire either component without
/// re-enrollment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorKey {
    /// Raw Ed25519 public key (32 bytes).
    pub ed25519_pk: [u8; 32],
    /// Raw ML-DSA-65 public key (1952 bytes).
    pub mldsa65_pk: [u8; 1952],
    /// SHA-256 over `ed25519_pk || mldsa65_pk`: the enrollment-ceremony
    /// check, verified at load.
    pub fingerprint: [u8; 32],
    /// When the operator enrolled, milliseconds since the Unix epoch.
    pub enrolled_ms: u64,
    /// True once the operator's keys are compromised or retired.
    pub revoked: bool,
}

/// The enrolled operators, loaded from a TOML file.
///
/// Lookup is by exact operator name. Unknown operators and revoked
/// entries resolve to distinct errors so a typo is distinguishable from
/// an incident.
#[derive(Debug, Clone, Default)]
pub struct OperatorRegistry {
    operators: BTreeMap<String, OperatorKey>,
}

impl OperatorRegistry {
    /// Loads and validates the registry at `path`.
    ///
    /// Accepted: a TOML file with an `[operators.<name>]` table per
    /// operator, each carrying hex `ed25519_pubkey` (64 chars),
    /// `mldsa65_pubkey` (3904 chars), and `fingerprint` (64 chars),
    /// integer `enrolled_ms`, and boolean `revoked`. Rejected: missing or
    /// unreadable files, files over [`REGISTRY_BYTES_MAX`] bytes,
    /// group/world-readable files (Unix), invalid TOML, unknown keys,
    /// malformed fields, and fingerprint mismatches. Every rejection is
    /// fail-closed: no partial registry is ever returned.
    pub fn load(path: &Path) -> Result<Self, ExperimentError> {
        check_owner_only(path)?;
        let metadata = std::fs::metadata(path).map_err(|_| ExperimentError::ApprovalRejected {
            reason: "operator registry unreadable",
        })?;
        if metadata.len() > REGISTRY_BYTES_MAX {
            return Err(ExperimentError::ApprovalRejected {
                reason: "operator registry too large",
            });
        }
        let text =
            std::fs::read_to_string(path).map_err(|_| ExperimentError::ApprovalRejected {
                reason: "operator registry unreadable",
            })?;
        let value: toml::Value = text
            .parse()
            .map_err(|_| ExperimentError::ApprovalRejected {
                reason: "operator registry is not valid TOML",
            })?;
        Self::parse(&value)
    }

    /// The default registry path: `PHLOW_OPERATORS_FILE` wins, then
    /// `$XDG_CONFIG_HOME/phlow/operators.toml`, then
    /// `~/.config/phlow/operators.toml`.
    pub fn default_path() -> PathBuf {
        if let Ok(path) = std::env::var("PHLOW_OPERATORS_FILE")
            && !path.is_empty()
        {
            return PathBuf::from(path);
        }
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
            && !xdg.is_empty()
        {
            return PathBuf::from(xdg).join("phlow/operators.toml");
        }
        if let Ok(home) = std::env::var("HOME")
            && !home.is_empty()
        {
            return PathBuf::from(home).join(".config/phlow/operators.toml");
        }
        PathBuf::from("phlow/operators.toml")
    }

    /// Looks up an operator by exact name.
    ///
    /// Rejected: names the registry does not enroll
    /// ([`ExperimentError::UnknownOperator`]); enrolled but revoked names
    /// ([`ExperimentError::RevokedOperator`]).
    pub fn lookup(&self, operator: &str) -> Result<&OperatorKey, ExperimentError> {
        match self.operators.get(operator) {
            None => Err(ExperimentError::UnknownOperator {
                operator: operator.to_string(),
            }),
            Some(key) if key.revoked => Err(ExperimentError::RevokedOperator {
                operator: operator.to_string(),
            }),
            Some(key) => Ok(key),
        }
    }

    /// How many operators are enrolled (revoked entries included).
    pub fn len(&self) -> usize {
        self.operators.len()
    }

    /// True when no operators are enrolled.
    pub fn is_empty(&self) -> bool {
        self.operators.is_empty()
    }

    /// Validates a parsed TOML value into a registry. Unknown top-level
    /// keys and unknown per-operator keys are rejected: the file is a
    /// trust root, and unexpected content fails closed.
    fn parse(value: &toml::Value) -> Result<Self, ExperimentError> {
        let root = value.as_table().ok_or(ExperimentError::ApprovalRejected {
            reason: "operator registry root must be a table",
        })?;
        let mut registry = Self::default();
        for (key, section) in root {
            if key != "operators" {
                return Err(ExperimentError::ApprovalRejected {
                    reason: "operator registry has an unexpected top-level key",
                });
            }
            let operators = section
                .as_table()
                .ok_or(ExperimentError::ApprovalRejected {
                    reason: "operator registry operators must be a table",
                })?;
            for (name, entry) in operators {
                check_operator_name(name)?;
                if registry
                    .operators
                    .insert(name.clone(), Self::parse_operator(entry)?)
                    .is_some()
                {
                    return Err(ExperimentError::ApprovalRejected {
                        reason: "operator registry has a duplicate operator",
                    });
                }
            }
        }
        Ok(registry)
    }

    /// Validates one `[operators.<name>]` table into an [`OperatorKey`],
    /// including the fingerprint check over both public keys.
    fn parse_operator(entry: &toml::Value) -> Result<OperatorKey, ExperimentError> {
        let table = entry.as_table().ok_or(ExperimentError::ApprovalRejected {
            reason: "operator registry entry must be a table",
        })?;
        for key in table.keys() {
            match key.as_str() {
                "ed25519_pubkey" | "mldsa65_pubkey" | "fingerprint" | "enrolled_ms" | "revoked" => {
                }
                _ => {
                    return Err(ExperimentError::ApprovalRejected {
                        reason: "operator registry entry has an unexpected key",
                    });
                }
            }
        }
        let ed25519_pk =
            decode_fixed::<32>(table_str(table, "ed25519_pubkey")?, ED25519_PK_HEX_CHARS)?;
        let mldsa65_pk =
            decode_fixed::<1952>(table_str(table, "mldsa65_pubkey")?, MLDSA65_PK_HEX_CHARS)?;
        let fingerprint =
            decode_fixed::<32>(table_str(table, "fingerprint")?, FINGERPRINT_HEX_CHARS)?;
        let mut hasher = Sha256::new();
        hasher.update(ed25519_pk);
        hasher.update(mldsa65_pk);
        let expected: [u8; 32] = hasher.finalize().into();
        if expected != fingerprint {
            return Err(ExperimentError::ApprovalRejected {
                reason: "registry fingerprint mismatch",
            });
        }
        let enrolled_ms = table
            .get("enrolled_ms")
            .and_then(toml::Value::as_integer)
            .and_then(|ms| u64::try_from(ms).ok())
            .ok_or(ExperimentError::ApprovalRejected {
                reason: "operator registry enrolled_ms must be a non-negative integer",
            })?;
        let revoked = table.get("revoked").and_then(toml::Value::as_bool).ok_or(
            ExperimentError::ApprovalRejected {
                reason: "operator registry revoked must be a boolean",
            },
        )?;
        Ok(OperatorKey {
            ed25519_pk,
            mldsa65_pk,
            fingerprint,
            enrolled_ms,
            revoked,
        })
    }
}

/// Reads a required string field from an operator table.
fn table_str<'a>(
    table: &'a toml::map::Map<String, toml::Value>,
    key: &str,
) -> Result<&'a str, ExperimentError> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .ok_or(ExperimentError::ApprovalRejected {
            reason: "operator registry entry is missing a required string field",
        })
}

/// Validates an operator name: non-empty, bounded, and limited to the
/// same charset the approval record allows.
fn check_operator_name(name: &str) -> Result<(), ExperimentError> {
    if name.is_empty() || name.len() > OPERATOR_NAME_CHARS_MAX {
        return Err(ExperimentError::ApprovalRejected {
            reason: "operator registry has an invalid operator name",
        });
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
    {
        return Err(ExperimentError::ApprovalRejected {
            reason: "operator registry has an invalid operator name",
        });
    }
    Ok(())
}

/// Decodes exactly `hex_chars` hex characters into `N` bytes. Rejected:
/// wrong length or non-hex input.
fn decode_fixed<const N: usize>(value: &str, hex_chars: usize) -> Result<[u8; N], ExperimentError> {
    if value.len() != hex_chars || N * 2 != hex_chars {
        return Err(ExperimentError::ApprovalRejected {
            reason: "operator registry hex field has wrong length",
        });
    }
    let bytes = value.as_bytes();
    let mut out = [0u8; N];
    let (chunks, _) = bytes.as_chunks::<2>();
    for (index, pair) in chunks.iter().enumerate() {
        let hi = hex_nibble(pair[0]);
        let lo = hex_nibble(pair[1]);
        match (hi, lo) {
            (Some(hi), Some(lo)) => out[index] = (hi << 4) | lo,
            _ => {
                return Err(ExperimentError::ApprovalRejected {
                    reason: "operator registry hex field has non-hex characters",
                });
            }
        }
    }
    Ok(out)
}

/// One hex digit's value, or `None` for non-hex input.
fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Rejects registry files readable by anyone but the owner. The registry
/// is the trust root for approvals; Tiger Style fails closed here.
fn check_owner_only(path: &Path) -> Result<(), ExperimentError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let metadata = std::fs::metadata(path).map_err(|_| ExperimentError::ApprovalRejected {
            reason: "operator registry unreadable",
        })?;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ExperimentError::ApprovalRejected {
                reason: "operator registry must be owner-only (0600)",
            });
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
