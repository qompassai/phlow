//! Projection-head contract for contrastive System-1 scoring.
//!
//! Plain words: an encoder turns text into wide embeddings; a projection head
//! maps those into a smaller space where cosine similarity means "this action
//! fits this state". CS1 trains two such heads — a state head and an action
//! head — as small MLPs (`hidden -> width -> ... -> proj`, optional LayerNorm
//! / residual blocks) and scores a (state, candidate) pair as
//! `exp(logit_scale) * cosine(state_head(s), action_head(c))`, the InfoNCE
//! objective (`/tmp/CLM/src/clm/heads.py`: `make_head`, `HeadPair::_project`).
//! This module captures the *contract* as a Rust trait so the scorer never
//! depends on torch: any head (a future ONNX/GGUF loader, a test double)
//! plugs in. PyTorch is deliberately not ported.
//!
//! # Contract
//!
//! - Inputs are `[n, embed_dim]` L2-normalised encoder embeddings.
//! - Outputs are `[n, projection_dim]` L2-normalised projections.
//! - `logit_scale` is the already-exponentiated InfoNCE scale, clamped to
//!   [`LOGIT_SCALE_MAX`], mirroring `heads.py`'s `.clamp(max=100.0)`.
//! - [`ProjectionHead::namespace`] is `{name}@{generation}`: the cache stamps
//!   rows with the exact weights that produced them, so a hot-reloaded head
//!   simply stops matching its previous rows (`heads.py`, `namespace`
//!   property; `/tmp/CLM/src/clm/cache.py` module docstring).
//!
//! # Bounds
//!
//! - `embed_dim >= 1`, `projection_dim >= 1` (validated at construction).
//! - `0 < logit_scale <= LOGIT_SCALE_MAX`, finite (validated).
//! - Batches hold at most [`EMBEDDINGS_MAX`] embedding rows.
//!
//! # Unsafe policy
//!
//! This module forbids unsafe code (crate-level `#![forbid(unsafe_code)]`).

use std::fmt;

/// Qwen3-8B hidden size: the encoder embedding width the reference head
/// expects (`/tmp/CLM/src/clm/heads.py:24`, `HIDDEN`).
pub const ENCODER_WIDTH: usize = 4096;

/// Reference projection width (`/tmp/CLM/src/clm/heads.py:25`, `PROJ_DIM`).
pub const PROJECTION_WIDTH: usize = 512;

/// Upper clamp for the exponentiated logit scale
/// (`/tmp/CLM/src/clm/heads.py:104`, `.clamp(max=100.0)`).
pub const LOGIT_SCALE_MAX: f32 = 100.0;

/// Maximum embedding rows accepted in one projection call.
pub const EMBEDDINGS_MAX: usize = 4096;

/// Maximum projection width accepted by the test double.
pub const PROJECTION_DIM_MAX: usize = 1 << 16;

/// Projected state rows paired with projected action rows, in input order.
pub type ProjectedPair = (Vec<Vec<f32>>, Vec<Vec<f32>>);

/// Failures of projection-head configuration or projection. All are caller
/// errors (bad config, bad batch), never internal faults.
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectionError {
    /// A batch held no embedding rows.
    EmptyBatch,
    /// A batch held more than [`EMBEDDINGS_MAX`] rows.
    BatchTooLarge {
        /// Rows offered.
        rows: usize,
    },
    /// Row `index` did not have the head's embedding width.
    DimMismatch {
        /// Row index in the batch.
        index: usize,
        /// The head's `embed_dim`.
        expected: usize,
        /// The row's actual width.
        got: usize,
    },
    /// Row `index` contained a non-finite component.
    NonFinite {
        /// Row index in the batch.
        index: usize,
    },
    /// A projection collapsed to the zero vector (cannot be normalised).
    Degenerate {
        /// Row index in the batch.
        index: usize,
    },
    /// Head configuration was invalid (zero width, bad scale).
    BadConfig {
        /// Which field was invalid.
        field: &'static str,
    },
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectionError::EmptyBatch => write!(f, "projection batch must not be empty"),
            ProjectionError::BatchTooLarge { rows } => {
                write!(
                    f,
                    "batch of {rows} rows exceeds EMBEDDINGS_MAX={EMBEDDINGS_MAX}"
                )
            }
            ProjectionError::DimMismatch {
                index,
                expected,
                got,
            } => {
                write!(
                    f,
                    "row {index} has width {got}, expected embed_dim={expected}"
                )
            }
            ProjectionError::NonFinite { index } => {
                write!(f, "row {index} contains a non-finite component")
            }
            ProjectionError::Degenerate { index } => {
                write!(f, "row {index} projected to the zero vector")
            }
            ProjectionError::BadConfig { field } => {
                write!(f, "invalid projection head config: {field}")
            }
        }
    }
}

impl std::error::Error for ProjectionError {}

/// Validate a batch of encoder embeddings before any projection work.
fn check_embeddings(embeddings: &[Vec<f32>], embed_dim: usize) -> Result<(), ProjectionError> {
    if embeddings.is_empty() {
        return Err(ProjectionError::EmptyBatch);
    }
    if embeddings.len() > EMBEDDINGS_MAX {
        return Err(ProjectionError::BatchTooLarge {
            rows: embeddings.len(),
        });
    }
    for (index, row) in embeddings.iter().enumerate() {
        if row.len() != embed_dim {
            return Err(ProjectionError::DimMismatch {
                index,
                expected: embed_dim,
                got: row.len(),
            });
        }
        if row.iter().any(|x| !x.is_finite()) {
            return Err(ProjectionError::NonFinite { index });
        }
    }
    Ok(())
}

/// L2-normalise `row` in place. Mirrors `torch.nn.functional.normalize`
/// in `HeadPair::_project` (`/tmp/CLM/src/clm/heads.py:108-112`).
fn normalize_row(row: &mut [f32], index: usize) -> Result<(), ProjectionError> {
    let norm = row
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    if !norm.is_finite() || norm == 0.0 {
        return Err(ProjectionError::Degenerate { index });
    }
    let inv = 1.0 / norm;
    for x in row.iter_mut() {
        *x = (f64::from(*x) * inv) as f32;
    }
    Ok(())
}

/// A CS1 projection-head pair: state head + action head sharing one config.
///
/// The trait is object-safe so heads can be boxed behind `dyn`.
pub trait ProjectionHead {
    /// Head name, e.g. `"cs1-latest"`.
    fn name(&self) -> &str;

    /// Weight generation, bumped on every (re)load (`heads.py`, `HeadPair`
    /// docstring: "hot-reloaded when the file changes").
    fn generation(&self) -> u64;

    /// Identity of these exact weights, for cache keys: `{name}@{generation}`
    /// (`/tmp/CLM/src/clm/heads.py:125-128`, `namespace` property).
    fn namespace(&self) -> String {
        format!("{}@{}", self.name(), self.generation())
    }

    /// Encoder embedding width this head accepts.
    fn embed_dim(&self) -> usize;

    /// Projection width this head emits.
    fn projection_dim(&self) -> usize;

    /// Exponentiated InfoNCE scale, already clamped to [`LOGIT_SCALE_MAX`].
    fn logit_scale(&self) -> f32;

    /// Project state embeddings: `[n, embed_dim]` -> `[n, projection_dim]`,
    /// L2-normalised.
    fn project_states(&self, embeddings: &[Vec<f32>]) -> Result<Vec<Vec<f32>>, ProjectionError>;

    /// Project candidate (action) embeddings: `[n, embed_dim]` ->
    /// `[n, projection_dim]`, L2-normalised.
    fn project_actions(&self, embeddings: &[Vec<f32>]) -> Result<Vec<Vec<f32>>, ProjectionError>;

    /// Project both sides at once.
    fn project(
        &self,
        states: &[Vec<f32>],
        actions: &[Vec<f32>],
    ) -> Result<ProjectedPair, ProjectionError> {
        Ok((self.project_states(states)?, self.project_actions(actions)?))
    }
}

/// FNV-1a 64-bit hash. No external digest dependency; deterministic across
/// runs and platforms (fixed offset basis and prime, wrapping multiply).
fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Deterministic test double for [`ProjectionHead`].
///
/// Plain words: the mock playground (`/tmp/CLM/tools/playground_mock.py`,
/// `_embed`) stands in for Qwen3-8B with hashed character n-grams —
/// deterministic lexical noise, explicitly not a model. `HashProjection`
/// does the same one level down: it stands in for the *trained MLP* with
/// deterministic feature hashing over the embedding bytes, so scorer and
/// cache tests never need torch, weights, or a GPU. Like the mock, its
/// numbers are meaningless for quality; unlike the mock, it honours the
/// trait contract exactly (shapes, L2-normalised outputs, clamped scale,
/// generation-stamped namespace).
///
/// Cost: one content hash over the row's bytes plus one small sub-hash per
/// output dimension, i.e. `O(embed_dim + projection_dim)` per row — fine for
/// tests, documented so nobody mistakes it for a serving path.
#[derive(Debug, Clone)]
pub struct HashProjection {
    name: String,
    generation: u64,
    embed_dim: usize,
    projection_dim: usize,
    scale: f32,
}

impl HashProjection {
    /// Build the double. `scale` is the already-exponentiated logit scale.
    pub fn new(
        name: &str,
        embed_dim: usize,
        projection_dim: usize,
        scale: f32,
    ) -> Result<Self, ProjectionError> {
        if name.is_empty() || name.len() > 128 {
            return Err(ProjectionError::BadConfig { field: "name" });
        }
        if embed_dim == 0 {
            return Err(ProjectionError::BadConfig { field: "embed_dim" });
        }
        if projection_dim == 0 || projection_dim > PROJECTION_DIM_MAX {
            return Err(ProjectionError::BadConfig {
                field: "projection_dim",
            });
        }
        if !scale.is_finite() || scale <= 0.0 || scale > LOGIT_SCALE_MAX {
            return Err(ProjectionError::BadConfig {
                field: "logit_scale",
            });
        }
        Ok(Self {
            name: name.to_string(),
            generation: 0,
            embed_dim,
            projection_dim,
            scale,
        })
    }

    /// Simulate a hot reload: bumps the generation so the namespace changes
    /// and previously cached rows stop matching (`heads.py`, `generation`
    /// "bumped on every (re)load").
    pub fn bump_generation(&mut self) {
        self.generation = self.generation.saturating_add(1);
    }

    /// Hash one embedding row into `projection_dim` components in [-1, 1].
    fn hash_row(&self, row: &[f32]) -> Vec<f32> {
        // One content hash over the whole row, then one sub-hash per output
        // dimension seeded by the dimension index. Deterministic: same bytes
        // in, same vector out, on every platform.
        let mut content = Vec::with_capacity(row.len() * 4);
        for x in row {
            content.extend_from_slice(&x.to_bits().to_le_bytes());
        }
        let content_hash = fnv1a64(&content);
        const UNIT: f64 = 1.0 / u64::MAX as f64;
        (0..self.projection_dim)
            .map(|dim| {
                let mut seed = (dim as u64).to_le_bytes().to_vec();
                seed.extend_from_slice(&content_hash.to_le_bytes());
                (fnv1a64(&seed) as f64 * UNIT * 2.0 - 1.0) as f32
            })
            .collect()
    }

    fn project_batch(&self, embeddings: &[Vec<f32>]) -> Result<Vec<Vec<f32>>, ProjectionError> {
        check_embeddings(embeddings, self.embed_dim)?;
        let mut out = Vec::with_capacity(embeddings.len());
        for (index, row) in embeddings.iter().enumerate() {
            let mut projected = self.hash_row(row);
            normalize_row(&mut projected, index)?;
            out.push(projected);
        }
        Ok(out)
    }
}

impl ProjectionHead for HashProjection {
    fn name(&self) -> &str {
        &self.name
    }

    fn generation(&self) -> u64 {
        self.generation
    }

    fn embed_dim(&self) -> usize {
        self.embed_dim
    }

    fn projection_dim(&self) -> usize {
        self.projection_dim
    }

    fn logit_scale(&self) -> f32 {
        self.scale
    }

    fn project_states(&self, embeddings: &[Vec<f32>]) -> Result<Vec<Vec<f32>>, ProjectionError> {
        self.project_batch(embeddings)
    }

    fn project_actions(&self, embeddings: &[Vec<f32>]) -> Result<Vec<Vec<f32>>, ProjectionError> {
        self.project_batch(embeddings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn double() -> HashProjection {
        HashProjection::new("test-head", 8, 16, 28.0).expect("valid test double config")
    }

    fn unit_row(dim: usize, seed: f32) -> Vec<f32> {
        (0..dim).map(|i| seed + i as f32 * 0.01).collect()
    }

    // ---- validation: the contract holds ----

    #[test]
    fn outputs_are_l2_normalized() {
        let head = double();
        let rows = head
            .project_states(&[unit_row(8, 1.0), unit_row(8, 2.0)])
            .unwrap();
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert_eq!(row.len(), 16);
            let norm = row.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-5, "norm was {norm}");
        }
    }

    #[test]
    fn namespace_embeds_name_and_generation() {
        let head = double();
        assert_eq!(head.namespace(), "test-head@0");
    }

    #[test]
    fn bump_generation_changes_namespace() {
        let mut head = double();
        let before = head.namespace();
        head.bump_generation();
        assert_ne!(head.namespace(), before);
        assert_eq!(head.namespace(), "test-head@1");
    }

    #[test]
    fn projection_is_deterministic() {
        let head = double();
        let batch = vec![unit_row(8, 1.0)];
        let first = head.project_actions(&batch).unwrap();
        let second = head.project_actions(&batch).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn states_and_actions_share_shapes_but_may_differ() {
        // The double uses one hash for both sides (a real head would not);
        // the contract only requires shapes and normalisation.
        let head = double();
        let batch = vec![unit_row(8, 3.0)];
        let (states, actions) = head.project(&batch, &batch).unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(actions.len(), 1);
        assert_eq!(states[0].len(), head.projection_dim());
    }

    #[test]
    fn scale_passthrough_within_clamp() {
        let head = HashProjection::new("h", 4, 4, 100.0).unwrap();
        assert_eq!(head.logit_scale(), 100.0);
    }

    // ---- adversarial: bad input and config are rejected ----

    #[test]
    fn empty_batch_rejected() {
        let head = double();
        assert_eq!(head.project_states(&[]), Err(ProjectionError::EmptyBatch));
    }

    #[test]
    fn dim_mismatch_reports_index() {
        let head = double();
        let batch = vec![unit_row(8, 1.0), unit_row(7, 1.0)];
        assert_eq!(
            head.project_states(&batch),
            Err(ProjectionError::DimMismatch {
                index: 1,
                expected: 8,
                got: 7
            })
        );
    }

    #[test]
    fn nonfinite_input_rejected() {
        let head = double();
        let mut row = unit_row(8, 1.0);
        row[3] = f32::NAN;
        assert_eq!(
            head.project_states(&[row]),
            Err(ProjectionError::NonFinite { index: 0 })
        );
    }

    #[test]
    fn zero_embed_dim_rejected() {
        assert!(matches!(
            HashProjection::new("h", 0, 16, 1.0),
            Err(ProjectionError::BadConfig { field: "embed_dim" })
        ));
    }

    #[test]
    fn overscale_rejected() {
        assert!(matches!(
            HashProjection::new("h", 8, 16, 100.01),
            Err(ProjectionError::BadConfig {
                field: "logit_scale"
            })
        ));
    }

    #[test]
    fn batch_over_limit_rejected() {
        let head = double();
        let batch = vec![unit_row(8, 1.0); EMBEDDINGS_MAX + 1];
        assert_eq!(
            head.project_states(&batch),
            Err(ProjectionError::BatchTooLarge {
                rows: EMBEDDINGS_MAX + 1
            })
        );
    }
}
