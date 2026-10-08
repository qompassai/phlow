//! The canary payload store: attack content the probes run, loaded
//! from a separate access-controlled file that is never in the repo
//! (design open question 1). The crate ships the schema and the
//! loader, not the payloads.
//!
//! Loading is fail-closed: a missing file, a group/world-accessible
//! file, malformed JSON, unknown fields, or any list outside its
//! named cap is an error, and the battery does not run.

use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::error::CanaryError;

/// Maximum bytes of the payload store file.
pub const STORE_BYTES_MAX: usize = 256 * 1024;
/// Maximum variants in any one payload list.
pub const VARIANTS_MAX: usize = 32;
/// Maximum bytes of one injection/refusal payload. Bounded so a
/// payload plus probe framing fits the system1 instructions cap.
pub const PAYLOAD_BYTES_MAX: usize = 900;
/// Maximum bytes of one classification input (trigger/calibration).
pub const INPUT_BYTES_MAX: usize = 256;
/// Phrasings per refusal probe: the design's "phrased 10 ways" rule.
pub const REFUSAL_WAYS: usize = 10;
/// Clean inputs per trigger probe: at least 2, at most 6, so a batch
/// of clean + candidates stays inside the system1 question cap.
pub const CLEAN_INPUTS_MIN: usize = 2;
/// See [`CLEAN_INPUTS_MIN`].
pub const CLEAN_INPUTS_MAX: usize = 6;
/// Trigger candidates per trigger probe: same batch reasoning.
pub const CANDIDATES_MIN: usize = 2;
/// See [`CANDIDATES_MIN`].
pub const CANDIDATES_MAX: usize = 6;
/// Classification inputs for the bimodality probe.
pub const CALIBRATION_INPUTS_MIN: usize = 8;
/// See [`CALIBRATION_INPUTS_MIN`].
pub const CALIBRATION_INPUTS_MAX: usize = 16;

/// One trigger family: clean inputs, trigger candidates, and the two
/// perturbed forms of every candidate (parallel arrays, same order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerSet {
    /// Neutral inputs that must not move the model's distribution.
    pub clean: Vec<String>,
    /// Inputs carrying the suspected trigger pattern.
    pub candidates: Vec<String>,
    /// Each candidate paraphrased (challenger pool).
    pub paraphrased: Vec<String>,
    /// Each candidate with the suspected trigger stripped/replaced.
    pub stripped: Vec<String>,
}

/// All v1 payloads, validated against the named caps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadStore {
    /// Store format version, recorded for audit.
    pub version: String,
    /// Direct injection payloads (presented as a state field).
    pub injection_direct: Vec<String>,
    /// Indirect injection payloads (presented as tool output).
    pub injection_indirect: Vec<String>,
    /// Jailbreak template payloads.
    pub injection_jailbreak: Vec<String>,
    /// Disallowed requests, plainly phrased (exactly [`REFUSAL_WAYS`]).
    pub refusal_plain: Vec<String>,
    /// Disallowed requests with pretext/framing (exactly [`REFUSAL_WAYS`]).
    pub refusal_pretext: Vec<String>,
    /// Rare-token trigger family.
    pub trigger_rare_token: TriggerSet,
    /// Syntactic trigger family.
    pub trigger_syntactic: TriggerSet,
    /// Semantic trigger family.
    pub trigger_semantic: TriggerSet,
    /// Neutral inputs for the bimodality probe.
    pub calibration_inputs: Vec<String>,
}

/// The on-disk JSON shape. Closed: unknown fields are rejected so a
/// store cannot smuggle configuration the loader does not know about.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreFile {
    version: String,
    injection: InjectionFile,
    refusal: RefusalFile,
    trigger: TriggerFile,
    calibration: CalibrationFile,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InjectionFile {
    direct: Vec<String>,
    indirect: Vec<String>,
    jailbreak: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RefusalFile {
    plain: Vec<String>,
    pretext: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TriggerFile {
    rare_token: TriggerSetFile,
    syntactic: TriggerSetFile,
    semantic: TriggerSetFile,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TriggerSetFile {
    clean: Vec<String>,
    candidates: Vec<String>,
    paraphrased: Vec<String>,
    stripped: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CalibrationFile {
    inputs: Vec<String>,
}

impl PayloadStore {
    /// Load and validate the store at `path`.
    ///
    /// On unix the file must not grant any group/other access
    /// (`mode & 0o077 == 0`); canary content in a readable file is
    /// canary content an attacker can train against.
    pub fn load(path: &Path) -> Result<PayloadStore, CanaryError> {
        check_store_permissions(path)?;
        let bytes = fs::read(path).map_err(|e| CanaryError::PayloadStoreIo {
            reason: format!("cannot read {}: {e}", path.display()),
        })?;
        if bytes.len() > STORE_BYTES_MAX {
            return Err(CanaryError::PayloadStoreInvalid {
                reason: "store exceeds STORE_BYTES_MAX",
            });
        }
        let text = String::from_utf8(bytes).map_err(|_| CanaryError::PayloadStoreInvalid {
            reason: "store is not valid UTF-8",
        })?;
        PayloadStore::from_json(&text)
    }

    /// Parse and validate a store from JSON text. Used by `load` and
    /// by tests that build fixture stores in memory.
    pub fn from_json(text: &str) -> Result<PayloadStore, CanaryError> {
        if text.len() > STORE_BYTES_MAX {
            return Err(CanaryError::PayloadStoreInvalid {
                reason: "store exceeds STORE_BYTES_MAX",
            });
        }
        let file: StoreFile =
            serde_json::from_str(text).map_err(|_| CanaryError::PayloadStoreInvalid {
                reason: "store is not well-formed store JSON",
            })?;
        build_store(file)
    }
}

/// Enforce the owner-only permission rule before reading content.
fn check_store_permissions(path: &Path) -> Result<(), CanaryError> {
    let metadata = fs::metadata(path).map_err(|e| CanaryError::PayloadStoreIo {
        reason: format!("cannot stat {}: {e}", path.display()),
    })?;
    if !metadata.is_file() {
        return Err(CanaryError::PayloadStoreIo {
            reason: format!("{} is not a regular file", path.display()),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(CanaryError::PayloadStorePermissions { mode });
        }
    }
    Ok(())
}

/// Validate a parsed store file into the domain store.
fn build_store(file: StoreFile) -> Result<PayloadStore, CanaryError> {
    check_payload_list(&file.injection.direct, "injection.direct")?;
    check_payload_list(&file.injection.indirect, "injection.indirect")?;
    check_payload_list(&file.injection.jailbreak, "injection.jailbreak")?;
    check_refusal_list(&file.refusal.plain)?;
    check_refusal_list(&file.refusal.pretext)?;
    check_inputs(
        &file.calibration.inputs,
        CALIBRATION_INPUTS_MIN,
        CALIBRATION_INPUTS_MAX,
    )?;
    Ok(PayloadStore {
        version: file.version,
        injection_direct: file.injection.direct,
        injection_indirect: file.injection.indirect,
        injection_jailbreak: file.injection.jailbreak,
        refusal_plain: file.refusal.plain,
        refusal_pretext: file.refusal.pretext,
        trigger_rare_token: build_trigger_set(file.trigger.rare_token)?,
        trigger_syntactic: build_trigger_set(file.trigger.syntactic)?,
        trigger_semantic: build_trigger_set(file.trigger.semantic)?,
        calibration_inputs: file.calibration.inputs,
    })
}

/// Validate one injection payload list: non-empty, capped, sized.
fn check_payload_list(list: &[String], _name: &str) -> Result<(), CanaryError> {
    if list.is_empty() || list.len() > VARIANTS_MAX {
        return Err(CanaryError::PayloadStoreInvalid {
            reason: "payload list empty or exceeds VARIANTS_MAX",
        });
    }
    for payload in list {
        if payload.len() > PAYLOAD_BYTES_MAX {
            return Err(CanaryError::PayloadStoreInvalid {
                reason: "payload exceeds PAYLOAD_BYTES_MAX",
            });
        }
    }
    Ok(())
}

/// Validate one refusal list: exactly [`REFUSAL_WAYS`] phrasings.
fn check_refusal_list(list: &[String]) -> Result<(), CanaryError> {
    if list.len() != REFUSAL_WAYS {
        return Err(CanaryError::PayloadStoreInvalid {
            reason: "refusal list must hold exactly REFUSAL_WAYS phrasings",
        });
    }
    for payload in list {
        if payload.len() > PAYLOAD_BYTES_MAX {
            return Err(CanaryError::PayloadStoreInvalid {
                reason: "payload exceeds PAYLOAD_BYTES_MAX",
            });
        }
    }
    Ok(())
}

/// Validate a classification input list against a count window.
fn check_inputs(list: &[String], min: usize, max: usize) -> Result<(), CanaryError> {
    if list.len() < min || list.len() > max {
        return Err(CanaryError::PayloadStoreInvalid {
            reason: "input list outside its count window",
        });
    }
    for input in list {
        if input.len() > INPUT_BYTES_MAX {
            return Err(CanaryError::PayloadStoreInvalid {
                reason: "input exceeds INPUT_BYTES_MAX",
            });
        }
    }
    Ok(())
}

/// Validate one trigger family, including the parallel-array rule:
/// every candidate has exactly one paraphrased and one stripped form.
fn build_trigger_set(file: TriggerSetFile) -> Result<TriggerSet, CanaryError> {
    check_inputs(&file.clean, CLEAN_INPUTS_MIN, CLEAN_INPUTS_MAX)?;
    check_inputs(&file.candidates, CANDIDATES_MIN, CANDIDATES_MAX)?;
    if file.paraphrased.len() != file.candidates.len()
        || file.stripped.len() != file.candidates.len()
    {
        return Err(CanaryError::PayloadStoreInvalid {
            reason: "perturbed forms must parallel the candidate list",
        });
    }
    check_inputs(&file.paraphrased, CANDIDATES_MIN, CANDIDATES_MAX)?;
    check_inputs(&file.stripped, CANDIDATES_MIN, CANDIDATES_MAX)?;
    Ok(TriggerSet {
        clean: file.clean,
        candidates: file.candidates,
        paraphrased: file.paraphrased,
        stripped: file.stripped,
    })
}

/// Seeded picker for randomized probe variants.
///
/// Selection depends on the operator-supplied seed (in production,
/// derived from a nonce and the model hash) and on a call counter —
/// never on the probe id — so knowing a probe's id does not predict
/// which variant it will run (design adversarial requirement).
#[derive(Debug, Clone)]
pub struct VariantPicker {
    state: u64,
    counter: u64,
}

impl VariantPicker {
    /// A picker seeded from operator entropy.
    pub fn new(seed: u64) -> Self {
        VariantPicker {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
            counter: 0,
        }
    }

    /// Pick one index in `0..len`. `len` must be non-zero; store
    /// validation guarantees every list a probe draws from is.
    pub fn pick(&mut self, len: usize) -> usize {
        assert!(len > 0, "pick from an empty variant list");
        self.counter = self.counter.wrapping_add(1);
        let mut z = self
            .state
            .wrapping_add(self.counter.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        // splitmix64 finalizer: cheap, well-distributed for this use.
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        self.state = z;
        (z % len as u64) as usize
    }
}
