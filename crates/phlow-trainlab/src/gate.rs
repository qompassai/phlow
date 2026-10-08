//! The selection / confirmation gate.
//!
//! The video's core evaluation discipline, expressed as a persistent
//! ledger: hyperparameters and checkpoints are **selected on dev**;
//! the confirmation split is then **opened exactly once**, bound to
//! that recorded selection. Opening is a state transition on disk —
//! not a convention — so a second experiment cannot quietly re-open
//! confirmation against a different selection and call it fresh.
//!
//! The ledger is a small JSON document, written atomically
//! (temp file + rename) on every transition.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::TrainlabError;

/// Maximum selection-id length (characters).
pub const SELECTION_ID_CHARS_MAX: usize = 128;

/// Ledger state, serialized to the gate file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GateState {
    /// The selection recorded from dev (checkpoint/hyperparameter id).
    pub selection_id: Option<String>,
    /// The selection the confirmation split was opened for.
    pub confirm_opened_for: Option<String>,
    /// How many times confirmation has been opened. Invariant: the
    /// gate's transitions keep this at most 1; a ledger claiming
    /// more is rejected as tampered (see [`ConfirmationGate::load`]).
    pub confirm_open_count: u32,
}

/// Proof that confirmation is open for a selection.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenRecord {
    /// The selection confirmation is open for.
    pub selection_id: String,
    /// `true` when this call found confirmation already open (the
    /// opening itself happened in an earlier call).
    pub already_open: bool,
}

/// The confirmation gate over one ledger file.
#[derive(Debug, Clone)]
pub struct ConfirmationGate {
    path: PathBuf,
    state: GateState,
}

impl ConfirmationGate {
    /// Load the gate at `path`; a missing file is a fresh gate.
    ///
    /// A ledger whose open count exceeds 1 violates the gate's own
    /// invariant and is rejected rather than trusted.
    pub fn load(path: impl AsRef<Path>) -> Result<ConfirmationGate, TrainlabError> {
        let path = path.as_ref().to_path_buf();
        let state = if path.exists() {
            let text = fs::read_to_string(&path)?;
            let state: GateState = serde_json::from_str(&text)?;
            if state.confirm_open_count > 1 {
                return Err(TrainlabError::Confirmation(format!(
                    "ledger at {} claims {} confirmation openings; refusing to trust it",
                    path.display(),
                    state.confirm_open_count
                )));
            }
            state
        } else {
            GateState::default()
        };
        Ok(ConfirmationGate { path, state })
    }

    /// Current state (read-only view).
    pub fn state(&self) -> &GateState {
        &self.state
    }

    /// Record the selection made on dev. Re-recording a *different*
    /// selection after confirmation has opened is refused: the
    /// opening is bound to the selection that existed at open time.
    pub fn record_selection(&mut self, selection_id: &str) -> Result<(), TrainlabError> {
        validate_selection_id(selection_id)?;
        if self.state.confirm_opened_for.is_some() {
            return Err(TrainlabError::Confirmation(
                "selection is locked: confirmation has already been opened".to_string(),
            ));
        }
        self.state.selection_id = Some(selection_id.to_string());
        self.persist()
    }

    /// Open the confirmation split for `selection_id`.
    ///
    /// Refuses when: no selection was recorded, the id does not match
    /// the recorded selection, or confirmation was already opened for
    /// a different selection. Re-opening for the *same* selection is
    /// idempotent and reports `already_open`.
    pub fn open_confirmation(&mut self, selection_id: &str) -> Result<OpenRecord, TrainlabError> {
        validate_selection_id(selection_id)?;
        let recorded = self.state.selection_id.as_deref().ok_or_else(|| {
            TrainlabError::Confirmation(
                "no selection recorded; select on dev before opening confirmation".to_string(),
            )
        })?;
        if recorded != selection_id {
            return Err(TrainlabError::Confirmation(format!(
                "selection mismatch: recorded {recorded}, asked {selection_id}"
            )));
        }
        if let Some(opened_for) = self.state.confirm_opened_for.as_deref() {
            if opened_for == selection_id {
                return Ok(OpenRecord {
                    selection_id: selection_id.to_string(),
                    already_open: true,
                });
            }
            return Err(TrainlabError::Confirmation(format!(
                "confirmation already opened for {opened_for}; it opens once"
            )));
        }
        self.state.confirm_opened_for = Some(selection_id.to_string());
        self.state.confirm_open_count = 1;
        self.persist()?;
        Ok(OpenRecord {
            selection_id: selection_id.to_string(),
            already_open: false,
        })
    }

    /// Require that confirmation is open for `selection_id` before a
    /// confirmation-split evaluation may proceed.
    pub fn require_confirmation_open(&self, selection_id: &str) -> Result<(), TrainlabError> {
        match self.state.confirm_opened_for.as_deref() {
            Some(opened) if opened == selection_id => Ok(()),
            Some(opened) => Err(TrainlabError::Confirmation(format!(
                "confirmation is open for {opened}, not {selection_id}"
            ))),
            None => Err(TrainlabError::Confirmation(
                "confirmation split is sealed; open it against a recorded selection first"
                    .to_string(),
            )),
        }
    }

    /// Atomically persist the state (temp file + rename in the same
    /// directory, so a crash never leaves a torn ledger).
    fn persist(&self) -> Result<(), TrainlabError> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let temp = parent.join(format!(
            ".phlow-trainlab-gate-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        let text = serde_json::to_string_pretty(&self.state)?;
        fs::write(&temp, text)?;
        fs::rename(&temp, &self.path)?;
        Ok(())
    }
}

/// Selection ids are short printable tokens; anything else is a
/// configuration error, not something to sanitize silently.
fn validate_selection_id(selection_id: &str) -> Result<(), TrainlabError> {
    if selection_id.is_empty()
        || selection_id.chars().count() > SELECTION_ID_CHARS_MAX
        || !selection_id.chars().all(|ch| ch.is_ascii_graphic())
    {
        return Err(TrainlabError::InvalidConfig(format!(
            "selection id must be 1..={SELECTION_ID_CHARS_MAX} printable ASCII characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_gate_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "phlow-trainlab-gate-test-{}-{tag}.json",
            std::process::id()
        ))
    }

    #[test]
    fn open_requires_a_recorded_matching_selection() {
        let path = temp_gate_path("order");
        let _ = fs::remove_file(&path);
        let mut gate = ConfirmationGate::load(&path).expect("gate");
        // No selection recorded yet.
        assert!(gate.open_confirmation("sel-1").is_err());
        gate.record_selection("sel-1").expect("record");
        // Wrong selection cannot open.
        assert!(gate.open_confirmation("sel-2").is_err());
        // The recorded one can.
        let record = gate.open_confirmation("sel-1").expect("open");
        assert!(!record.already_open);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn confirmation_opens_once_and_persists() {
        let path = temp_gate_path("once");
        let _ = fs::remove_file(&path);
        let mut gate = ConfirmationGate::load(&path).expect("gate");
        gate.record_selection("sel-1").expect("record");
        gate.open_confirmation("sel-1").expect("open");
        // Re-open for the same selection: idempotent view.
        let again = gate.open_confirmation("sel-1").expect("reopen");
        assert!(again.already_open);
        // The state survives a reload, with the count still 1.
        let reloaded = ConfirmationGate::load(&path).expect("reload");
        assert_eq!(reloaded.state().confirm_open_count, 1);
        assert!(reloaded.require_confirmation_open("sel-1").is_ok());
        assert!(reloaded.require_confirmation_open("sel-2").is_err());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn selection_locks_after_opening() {
        // Adversarial: re-selecting after confirmation opened would
        // let a second candidate claim the same fresh split.
        let path = temp_gate_path("lock");
        let _ = fs::remove_file(&path);
        let mut gate = ConfirmationGate::load(&path).expect("gate");
        gate.record_selection("sel-1").expect("record");
        gate.open_confirmation("sel-1").expect("open");
        assert!(gate.record_selection("sel-2").is_err());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn tampered_ledger_is_rejected() {
        // Adversarial: a hand-edited ledger claiming two openings
        // must be refused, not trusted.
        let path = temp_gate_path("tamper");
        fs::write(
            &path,
            r#"{"selection_id":"s","confirm_opened_for":"s","confirm_open_count":2}"#,
        )
        .expect("write");
        assert!(matches!(
            ConfirmationGate::load(&path),
            Err(TrainlabError::Confirmation(_))
        ));
        let _ = fs::remove_file(&path);
    }
}
