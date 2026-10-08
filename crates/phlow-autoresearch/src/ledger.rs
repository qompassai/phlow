//! The experiment ledger: Karpathy's `results.tsv`, rebuilt to
//! phlow's evidence standard. Each iteration is one JSON entry file,
//! written atomically and never overwritten, and each entry names the
//! SHA-256 of its predecessor — a hash chain over the whole run.
//! [`Ledger::open`] replays and verifies the entire chain before the
//! loop takes a single step: a tampered entry, a sequence gap, or a
//! broken link is [`AutoresearchError::LedgerCorrupt`] and nothing runs.
//!
//! The ledger directory may also hold the loop's `STOP` file and
//! operator notes; only files matching the entry pattern are chain
//! members, and any file that matches the pattern but does not parse
//! or verify is corruption, never skipped.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::{AutoresearchError, FailureClass};

/// Ledger entry schema identifier, versioned from the first build.
pub const LEDGER_SCHEMA: &str = "phlow.autoresearch.ledger/v1";
/// Hard bound on entries in one ledger directory.
pub const LEDGER_ENTRIES_MAX: u64 = 1024;
/// Maximum characters in an entry's description; longer text is
/// truncated on append so the bound holds for every writer.
pub const DESCRIPTION_CHARS_MAX: usize = 256;

/// Whether an entry records one experiment or the run's end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// One proposed/evaluated change-set.
    Iteration,
    /// The terminal entry: why the run stopped.
    Halt,
}

/// The loop's verdict on one iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Metric improved past epsilon (or this entry is the baseline):
    /// the change-set becomes the incumbent.
    Keep,
    /// Evaluated and not better: the change-set is reverted.
    Discard,
    /// The iteration failed; see `failure`.
    Crash,
    /// Terminal entry: the run halted; see `failure` / description.
    Halt,
}

/// One ledger entry. Field order is part of the hash contract: the
/// entry's hash is the SHA-256 of its canonical JSON (this struct's
/// derived serialization), stored in the filename and in the next
/// entry's `prev_sha256`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LedgerEntry {
    /// Schema identifier ([`LEDGER_SCHEMA`]).
    pub schema: String,
    /// 1-based sequence number, matching the filename prefix.
    pub seq: u64,
    /// Iteration or terminal halt.
    pub kind: EntryKind,
    /// SHA-256 of the previous entry; `None` only at genesis.
    pub prev_sha256: Option<String>,
    /// The change-set's identifier (empty for halt entries and for
    /// iterations where the proposer produced nothing identifiable).
    pub change_set_id: String,
    /// SHA-256 of the change-set's canonical JSON, when one exists.
    pub change_set_sha256: Option<String>,
    /// Incumbent metric before this iteration (`None` pre-baseline).
    pub metric_before: Option<f64>,
    /// Metric this iteration measured (`None` when it produced none).
    pub metric_after: Option<f64>,
    /// Keep / discard / crash / halt.
    pub decision: Decision,
    /// Failure class for crash and halt entries.
    pub failure: Option<FailureClass>,
    /// Trainlab receipt hash backing `metric_after`, when produced.
    pub trainlab_receipt_sha256: Option<String>,
    /// Trainer receipt hash backing `metric_after`, when produced.
    pub trainer_receipt_sha256: Option<String>,
    /// Wall-clock milliseconds this iteration consumed.
    pub wall_clock_ms: u64,
    /// One-line description, bounded by [`DESCRIPTION_CHARS_MAX`].
    pub description: String,
}

/// Fields a caller supplies to [`Ledger::append`]; the ledger assigns
/// `schema`, `seq`, and `prev_sha256` itself so callers cannot forge
/// chain position.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryDraft {
    /// Iteration or terminal halt.
    pub kind: EntryKind,
    /// The change-set's identifier (may be empty).
    pub change_set_id: String,
    /// SHA-256 of the change-set, when one exists.
    pub change_set_sha256: Option<String>,
    /// Incumbent metric before this iteration.
    pub metric_before: Option<f64>,
    /// Metric this iteration measured.
    pub metric_after: Option<f64>,
    /// Keep / discard / crash / halt.
    pub decision: Decision,
    /// Failure class for crash and halt entries.
    pub failure: Option<FailureClass>,
    /// Trainlab receipt hash, when produced.
    pub trainlab_receipt_sha256: Option<String>,
    /// Trainer receipt hash, when produced.
    pub trainer_receipt_sha256: Option<String>,
    /// Wall-clock milliseconds consumed.
    pub wall_clock_ms: u64,
    /// One-line description (truncated to the bound on append).
    pub description: String,
}

/// An opened, verified ledger. Single-writer: one loop owns a ledger
/// directory at a time.
#[derive(Debug)]
pub struct Ledger {
    dir: PathBuf,
    entries: Vec<LedgerEntry>,
    head_sha256: Option<String>,
}

impl Ledger {
    /// Open (creating if needed) and fully verify the ledger in `dir`.
    ///
    /// # Errors
    /// [`AutoresearchError::LedgerCorrupt`] on any chain defect: an
    /// entry whose bytes do not hash to its filename, a sequence gap
    /// or duplicate, a `prev_sha256` that does not name its
    /// predecessor, an unknown schema, or an entry-pattern file that
    /// does not parse. I/O and JSON failures surface as their variants.
    pub fn open(dir: impl AsRef<Path>) -> Result<Ledger, AutoresearchError> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        let mut files: Vec<(u64, String, PathBuf)> = Vec::new();
        for item in fs::read_dir(&dir)? {
            let item = item?;
            let name = item.file_name().to_string_lossy().into_owned();
            if let Some(parsed) = parse_entry_filename(&name) {
                files.push((parsed.0, parsed.1, item.path()));
            }
        }
        files.sort_by_key(|(seq, _, _)| *seq);
        if files.len() > usize::try_from(LEDGER_ENTRIES_MAX).unwrap_or(usize::MAX) {
            return Err(AutoresearchError::LedgerCorrupt(format!(
                "ledger holds {} entries, bound is {LEDGER_ENTRIES_MAX}",
                files.len()
            )));
        }
        let mut entries: Vec<LedgerEntry> = Vec::new();
        let mut expected_prev: Option<String> = None;
        for (index, (seq, file_hash, path)) in files.iter().enumerate() {
            let expected_seq = u64::try_from(index).unwrap_or(u64::MAX) + 1;
            if *seq != expected_seq {
                return Err(AutoresearchError::LedgerCorrupt(format!(
                    "sequence gap: expected {expected_seq}, found {seq}"
                )));
            }
            let bytes = fs::read(path)?;
            let actual_hash = format!("{:x}", Sha256::digest(&bytes));
            if actual_hash != *file_hash {
                return Err(AutoresearchError::LedgerCorrupt(format!(
                    "entry {seq} bytes hash to {actual_hash}, filename claims {file_hash}"
                )));
            }
            let entry: LedgerEntry = serde_json::from_slice(&bytes).map_err(|err| {
                AutoresearchError::LedgerCorrupt(format!("entry {seq} does not parse: {err}"))
            })?;
            if entry.schema != LEDGER_SCHEMA || entry.seq != *seq {
                return Err(AutoresearchError::LedgerCorrupt(format!(
                    "entry {seq} header mismatch (schema/seq)"
                )));
            }
            if entry.prev_sha256 != expected_prev {
                return Err(AutoresearchError::LedgerCorrupt(format!(
                    "entry {seq} prev link does not name its predecessor"
                )));
            }
            expected_prev = Some(actual_hash);
            entries.push(entry);
        }
        Ok(Ledger {
            dir,
            head_sha256: expected_prev,
            entries,
        })
    }

    /// Append one entry and return its SHA-256. The write is atomic
    /// (temp file + rename within the directory) and refuses to
    /// overwrite an existing entry file.
    ///
    /// # Errors
    /// [`AutoresearchError::LimitExceeded`] at the entry bound;
    /// [`AutoresearchError::Invalid`] on a non-finite metric (NaN can
    /// never enter the chain — it is not JSON-representable and never
    /// evidence); I/O and JSON failures as their variants.
    pub fn append(&mut self, draft: EntryDraft) -> Result<String, AutoresearchError> {
        let seq = u64::try_from(self.entries.len()).unwrap_or(u64::MAX) + 1;
        if seq > LEDGER_ENTRIES_MAX {
            return Err(AutoresearchError::LimitExceeded(format!(
                "ledger entry bound {LEDGER_ENTRIES_MAX} reached"
            )));
        }
        for metric in [draft.metric_before, draft.metric_after]
            .into_iter()
            .flatten()
        {
            if !metric.is_finite() {
                return Err(AutoresearchError::Invalid(
                    "non-finite metric refused entry into the ledger".to_string(),
                ));
            }
        }
        let entry = LedgerEntry {
            schema: LEDGER_SCHEMA.to_string(),
            seq,
            kind: draft.kind,
            prev_sha256: self.head_sha256.clone(),
            change_set_id: draft.change_set_id,
            change_set_sha256: draft.change_set_sha256,
            metric_before: draft.metric_before,
            metric_after: draft.metric_after,
            decision: draft.decision,
            failure: draft.failure,
            trainlab_receipt_sha256: draft.trainlab_receipt_sha256,
            trainer_receipt_sha256: draft.trainer_receipt_sha256,
            wall_clock_ms: draft.wall_clock_ms,
            description: truncate_chars(&draft.description, DESCRIPTION_CHARS_MAX),
        };
        let bytes = serde_json::to_vec(&entry)?;
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let filename = format!("{seq:04}-{hash}.json");
        let target = self.dir.join(&filename);
        if target.exists() {
            return Err(AutoresearchError::LedgerCorrupt(format!(
                "entry file already exists: {filename}"
            )));
        }
        let temp = self.dir.join(format!(".{filename}.tmp"));
        fs::write(&temp, &bytes)?;
        fs::rename(&temp, &target)?;
        self.head_sha256 = Some(hash.clone());
        self.entries.push(entry);
        Ok(hash)
    }

    /// Entries in chain order.
    #[must_use]
    pub fn entries(&self) -> &[LedgerEntry] {
        &self.entries
    }

    /// SHA-256 of the chain head, if any entry exists.
    #[must_use]
    pub fn head_sha256(&self) -> Option<&str> {
        self.head_sha256.as_deref()
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the ledger has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The incumbent metric: the `metric_after` of the most recent
    /// keep, or `None` before any baseline. This is how a restarted
    /// loop resumes exactly where the chain says it stopped.
    #[must_use]
    pub fn incumbent_metric(&self) -> Option<f64> {
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.decision == Decision::Keep)
            .and_then(|entry| entry.metric_after)
    }
}

/// Parse an entry filename `NNNN-<sha256>.json`; `None` for any other
/// name (operator files such as `STOP` are not chain members).
fn parse_entry_filename(name: &str) -> Option<(u64, String)> {
    let stem = name.strip_suffix(".json")?;
    let (seq_part, hash) = stem.split_once('-')?;
    if seq_part.len() != 4 || hash.len() != 64 {
        return None;
    }
    let seq: u64 = seq_part.parse().ok()?;
    if !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some((seq, hash.to_string()))
}

/// Truncate to at most `max` characters (not bytes), on a char boundary.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect()
}

/// Whether `value` is a 64-character lowercase/uppercase hex digest.
/// Shared evidence validation for receipt hashes.
#[must_use]
pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsupport::test_dir;

    fn draft(description: &str) -> EntryDraft {
        EntryDraft {
            kind: EntryKind::Iteration,
            change_set_id: "cs".to_string(),
            change_set_sha256: None,
            metric_before: None,
            metric_after: Some(0.5),
            decision: Decision::Keep,
            failure: None,
            trainlab_receipt_sha256: None,
            trainer_receipt_sha256: None,
            wall_clock_ms: 7,
            description: description.to_string(),
        }
    }

    #[test]
    fn chain_roundtrips_and_tracks_incumbent() {
        let dir = test_dir("ledger-roundtrip");
        let path = dir.path().join("ledger");
        let mut ledger = Ledger::open(&path).expect("open");
        let first = ledger.append(draft("baseline")).expect("append 1");
        let mut second = draft("better");
        second.metric_before = Some(0.5);
        second.metric_after = Some(0.6);
        let second_hash = ledger.append(second).expect("append 2");
        assert_eq!(ledger.head_sha256(), Some(second_hash.as_str()));
        assert_ne!(first, second_hash);
        assert_eq!(ledger.incumbent_metric(), Some(0.6));

        let reopened = Ledger::open(&path).expect("reopen");
        assert_eq!(reopened.len(), 2);
        assert_eq!(reopened.incumbent_metric(), Some(0.6));
    }

    #[test]
    fn tampered_entry_fails_closed() {
        let dir = test_dir("ledger-tamper");
        let path = dir.path().join("ledger");
        let mut ledger = Ledger::open(&path).expect("open");
        ledger.append(draft("one")).expect("append 1");
        ledger.append(draft("two")).expect("append 2");
        drop(ledger);
        let victim = fs::read_dir(&path)
            .expect("read dir")
            .filter_map(Result::ok)
            .map(|item| item.path())
            .find(|p| p.to_string_lossy().contains("0001-"))
            .expect("entry 1 exists");
        let mut bytes = fs::read(&victim).expect("read entry");
        bytes[20] ^= 0xFF;
        fs::write(&victim, bytes).expect("tamper");
        let err = Ledger::open(&path).expect_err("tamper must be caught");
        assert!(matches!(err, AutoresearchError::LedgerCorrupt(_)), "{err}");
    }

    #[test]
    fn nan_metric_never_enters_the_chain() {
        let dir = test_dir("ledger-nan");
        let path = dir.path().join("ledger");
        let mut ledger = Ledger::open(&path).expect("open");
        let mut bad = draft("nan");
        bad.metric_after = Some(f64::NAN);
        let err = ledger.append(bad).expect_err("nan must be refused");
        assert!(matches!(err, AutoresearchError::Invalid(_)), "{err}");
        assert!(ledger.is_empty());
    }
}
