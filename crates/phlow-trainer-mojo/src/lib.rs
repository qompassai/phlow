#![deny(unsafe_op_in_unsafe_fn)]

//! Mojo scoring + sampling backend for phlow-trainlab (productized).
//!
//! This crate is the Rust host for Mojo GPU kernels implementing the
//! parity-critical math of the shared trainer contract
//! (`phlow.trainlab.groups/v1` in, scores out): per-token logprob
//! reduction over the vocabulary, RLOO leave-one-out advantages, and
//! — since ABI v2 — a composable sampling kernel (greedy /
//! temperature / top-k / top-p with a seeded SplitMix64 stream, so a
//! receipt carrying (seed, draw) reproduces a run exactly).
//!
//! It is deliberately **not** a trainer: no gradient step is taken
//! here and no adapter is produced. The PyTorch sidecar remains the
//! logits producer and the trainer of record (the trainer-backend
//! program's verdict); this backend accelerates the scoring and
//! sampling path behind the same groups/receipt contract the
//! PyTorch, Candle, and Burn tracks consume, and records its work in
//! a scoring receipt (`phlow.trainer-mojo.scoring-receipt/v1`)
//! naming the backend and kernel ABI version that produced it.
//!
//! # Product shape
//!
//! - **Backend surface** ([`backend`], feature `trainlab-contract`,
//!   on by default): availability probe, dispatching score /
//!   advantage / sample calls, and the production run that verifies a
//!   groups export against its trainlab receipt before scoring it.
//!   The backend is selected by the operator invoking this crate's
//!   entry points; phlow-trainlab's default (the PyTorch sidecar
//!   path) is unchanged.
//! - **GPU-residency policy** ([`gpu_policy`]): free VRAM is checked
//!   via `nvidia-smi` before any device allocation. Below the
//!   configured minimum (default
//!   [`gpu_policy::GPU_FREE_MIB_MIN_DEFAULT`] MiB) the backend is
//!   unavailable — a typed error, never a partial result — and the
//!   dispatch falls back to the pure-Rust reference in [`reference`],
//!   recording the fallback in the receipt.
//! - **Runtime deployment**: `build.rs` compiles the kernels with the
//!   pixi-pinned MAX 26.5 / Mojo 1.0 toolchain, then vendors the Mojo
//!   runtime libraries the kernel library needs into the build output
//!   and rewrites the kernel library's rpath to `$ORIGIN`. Built
//!   artifacts therefore run with no pixi env present; pixi is a
//!   build-time toolchain only. `scripts/install-runtime.sh` installs
//!   the CLI + libraries into a stable prefix with the same layout.
//!
//! # Bounds
//!
//! All work is bounded by the named limits below; inputs arriving
//! over the FFI boundary are validated before any device work is
//! queued.
//!
//! # Hardware scope
//!
//! Primo's RTX 4070 Laptop (8 GiB), shared with other jobs: see the
//! GPU-residency policy above. Measured runs record the free-memory
//! figure they observed.

/// Maximum tokens scored in one logprob-reduction launch.
pub const TOKENS_PER_LAUNCH_MAX: u32 = 65_536;
/// Maximum vocabulary width accepted by the kernels (Qwen2.5 is
/// 151,936; the cap leaves headroom without admitting absurd shapes).
pub const VOCAB_SIZE_MAX: u32 = 262_144;
/// Maximum completions in one RLOO group (trainlab's GROUP_SIZE_MAX is
/// 64; this cap is the kernel's own bound, stated independently).
pub const GROUP_COMPLETIONS_MAX: u32 = 1_024;
/// Maximum groups scored in one advantage-kernel launch.
pub const GROUPS_PER_LAUNCH_MAX: u32 = 4_096;
/// Maximum logits elements (rows * vocab) accepted per launch: caps the
/// device allocation at 1 GiB of f32 regardless of the individual shape
/// bounds.
pub const LOGITS_ELEMENTS_MAX: u32 = 268_435_456;
/// Maximum completions summed across all groups in one advantage call.
pub const COMPLETIONS_TOTAL_MAX: u32 = 1_048_576;
/// Maximum kernel re-launches in one bench call.
pub const REPEATS_MAX: u32 = 10_000;
/// Maximum candidates the sampling kernel extracts per row (the
/// top-k bound and the nucleus-extraction bound; see the kernel's
/// section header for the fallback when a nucleus cannot close).
pub const SAMPLE_CANDIDATES_MAX: u32 = 2_048;
/// Maximum rows in one sampling launch (candidate scratch is
/// rows * SAMPLE_CANDIDATES_MAX, so this caps device scratch at
/// 64 MiB of candidate data).
pub const SAMPLE_ROWS_PER_LAUNCH_MAX: u32 = 4_096;
/// Maximum draw index accepted by the sampling kernel.
pub const SAMPLE_DRAW_INDEX_MAX: u32 = 1_000_000;

#[cfg(feature = "trainlab-contract")]
pub mod backend;
pub mod error;
pub mod ffi;
pub mod gpu_policy;
pub mod reference;
pub mod scoring_receipt;
