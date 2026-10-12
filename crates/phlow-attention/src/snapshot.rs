//! A saliency snapshot: the shared frame two agents coordinate within.
//!
//! Plain words: a snapshot scores every target in a workspace once and
//! hashes the result. The hash names the frame. Two agents holding the
//! same hash are provably looking at the same scored workspace; two
//! agents with different hashes must not compare foci at all. This is
//! the analog of the paper's shared, unpermuted frame — the thing that
//! makes a stranger's attention interpretable.
//!
//! The canonical serialisation (targets sorted by identifier, scores
//! at fixed precision, one per line) is part of the protocol: change
//! it and every hash changes, which is the correct, loud failure.

use std::collections::BTreeMap;
use std::fmt;

use sha2::{Digest, Sha256};

use crate::error::AttentionError;
use crate::saliency::{SaliencyInput, saliency};
use crate::{SNAPSHOT_TARGETS_MAX, TARGET_ID_MAX_BYTES};

/// A workspace object an agent can attend to: a file path, or
/// `path#symbol` for a symbol within a file.
///
/// Validated at construction: non-empty, within the byte bound, and
/// free of control characters (a tab or newline inside an id would
/// corrupt the canonical snapshot frame it gets hashed into).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetId(String);

impl TargetId {
    /// Validate and wrap a raw target identifier.
    pub fn new(raw: &str) -> Result<TargetId, AttentionError> {
        if raw.is_empty() || raw.len() > TARGET_ID_MAX_BYTES || raw.chars().any(char::is_control) {
            return Err(AttentionError::InvalidTargetId(raw.to_owned()));
        }
        Ok(TargetId(raw.to_owned()))
    }

    /// The identifier as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Wrap a raw identifier arriving from a transport, where the
    /// length bound has already been enforced by the deserialiser.
    /// Full re-validation happens in `Focus::validate` — this exists
    /// so the tuple field stays private outside this module.
    pub(crate) fn from_wire(raw: String) -> TargetId {
        TargetId(raw)
    }
}

impl fmt::Display for TargetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&TargetId> for String {
    fn from(id: &TargetId) -> String {
        id.0.clone()
    }
}

/// Scores at six decimal places in the canonical frame: fine enough
/// that ranking differences survive, coarse enough that float noise
/// from different evaluation orders cannot split a frame.
fn canonical_score(score: f64) -> String {
    format!("{score:.6}")
}

/// An immutable scored workspace plus its frame hash.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    scores: BTreeMap<TargetId, f64>,
    hash: String,
}

impl Snapshot {
    /// Build a snapshot from raw per-target signals.
    ///
    /// Rejects duplicate targets and inputs beyond
    /// [`SNAPSHOT_TARGETS_MAX`]: a snapshot is a bounded object two
    /// agents can afford to exchange and verify, not a database dump.
    pub fn build(inputs: Vec<(TargetId, SaliencyInput)>) -> Result<Snapshot, AttentionError> {
        if inputs.len() > SNAPSHOT_TARGETS_MAX {
            return Err(AttentionError::TooManyTargets {
                count: inputs.len(),
                max: SNAPSHOT_TARGETS_MAX,
            });
        }
        let mut scores = BTreeMap::new();
        for (id, input) in inputs {
            let score = saliency(&input);
            if scores.insert(id.clone(), score).is_some() {
                return Err(AttentionError::DuplicateTarget(id.to_string()));
            }
        }
        let hash = hash_scores(&scores);
        Ok(Snapshot { scores, hash })
    }

    /// The frame hash: SHA-256 over the canonical serialisation,
    /// hex-encoded. Two snapshots with equal hashes are the same
    /// frame for every protocol purpose.
    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// The saliency of one target, if the snapshot carries it.
    pub fn score(&self, id: &TargetId) -> Option<f64> {
        self.scores.get(id).copied()
    }

    /// Number of targets in the snapshot.
    pub fn len(&self) -> usize {
        self.scores.len()
    }

    /// Whether the snapshot carries no targets.
    pub fn is_empty(&self) -> bool {
        self.scores.is_empty()
    }

    /// All targets in canonical (identifier) order.
    pub fn targets(&self) -> impl Iterator<Item = &TargetId> {
        self.scores.keys()
    }

    /// Targets ranked for attention: saliency descending, identifier
    /// ascending as the deterministic tie-break. Every agent derives
    /// the same ranking from the same snapshot — this shared ranking
    /// is what replaces a shared convention.
    pub fn ranked(&self) -> Vec<(&TargetId, f64)> {
        let mut ranked: Vec<(&TargetId, f64)> =
            self.scores.iter().map(|(id, s)| (id, *s)).collect();
        ranked.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(b.0))
        });
        ranked
    }
}

/// Hash the canonical serialisation of a score map.
fn hash_scores(scores: &BTreeMap<TargetId, f64>) -> String {
    let mut frame = String::new();
    for (id, score) in scores {
        frame.push_str(id.as_str());
        frame.push('\t');
        frame.push_str(&canonical_score(*score));
        frame.push('\n');
    }
    let digest = Sha256::digest(frame.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saliency::SaliencyInput;

    fn target(raw: &str) -> TargetId {
        TargetId::new(raw).expect("test target id is valid")
    }

    fn named() -> SaliencyInput {
        let mut input = SaliencyInput::quiet();
        input.named_in_task = true;
        input
    }

    #[test]
    fn hash_is_insertion_order_independent() {
        let a = Snapshot::build(vec![
            (target("src/a.rs"), named()),
            (target("src/b.rs"), SaliencyInput::quiet()),
        ])
        .expect("builds");
        let b = Snapshot::build(vec![
            (target("src/b.rs"), SaliencyInput::quiet()),
            (target("src/a.rs"), named()),
        ])
        .expect("builds");
        assert_eq!(a.hash(), b.hash());
        assert_eq!(a, b);
    }

    #[test]
    fn different_signals_produce_a_different_frame() {
        let a = Snapshot::build(vec![(target("src/a.rs"), named())]).expect("builds");
        let b =
            Snapshot::build(vec![(target("src/a.rs"), SaliencyInput::quiet())]).expect("builds");
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn duplicate_targets_are_rejected() {
        let err = Snapshot::build(vec![
            (target("src/a.rs"), named()),
            (target("src/a.rs"), named()),
        ])
        .expect_err("duplicates must fail");
        assert!(matches!(err, AttentionError::DuplicateTarget(_)));
    }

    #[test]
    fn oversized_snapshot_is_rejected() {
        let inputs: Vec<(TargetId, SaliencyInput)> = (0..=SNAPSHOT_TARGETS_MAX)
            .map(|i| (target(&format!("src/f{i}.rs")), SaliencyInput::quiet()))
            .collect();
        let err = Snapshot::build(inputs).expect_err("oversized snapshot must fail");
        assert!(matches!(err, AttentionError::TooManyTargets { .. }));
    }

    #[test]
    fn ranked_orders_by_saliency_then_identifier() {
        let mut mid = SaliencyInput::quiet();
        mid.graph_centrality = 0.5;
        let snap = Snapshot::build(vec![
            (target("src/z.rs"), mid),
            (target("src/a.rs"), named()),
            (target("src/m.rs"), mid),
        ])
        .expect("builds");
        let ranked = snap.ranked();
        let order: Vec<&str> = ranked.iter().map(|(id, _)| id.as_str()).collect();
        // a.rs named (0.35) first; m.rs and z.rs tie on centrality
        // (0.10) and fall back to identifier order.
        assert_eq!(order, vec!["src/a.rs", "src/m.rs", "src/z.rs"]);
    }

    #[test]
    fn target_id_validation_rejects_frame_breakers() {
        assert!(TargetId::new("").is_err());
        assert!(TargetId::new("src/a\tb.rs").is_err());
        assert!(TargetId::new("src/a\nb.rs").is_err());
        assert!(TargetId::new(&"x".repeat(TARGET_ID_MAX_BYTES + 1)).is_err());
        assert!(TargetId::new("src/a.rs#main_loop").is_ok());
    }
}
