//! Experimental Mojo kernel track for phlow-trainlab.
//!
//! This crate is the Rust host for a set of Mojo GPU kernels that implement
//! the parity-critical math of the shared trainer contract
//! (`phlow.trainlab.groups/v1` in, scoring out): per-token logprob
//! reduction over the vocabulary and RLOO leave-one-out advantage
//! computation. It is deliberately **not** a trainer: no gradient step is
//! taken here, no adapter is produced, and no trainer receipt is emitted
//! (a receipt asserts weights moved; these kernels move none).
//!
//! # Shape of the experiment
//!
//! - Kernels live in `kernels/` as Mojo sources, compiled by the
//!   pixi-pinned MAX 26.5 / Mojo 1.0 toolchain (`pixi.toml` + `pixi.lock`
//!   in this crate directory) into a shared library exporting a C ABI.
//! - `build.rs` drives that compilation; the FFI boundary is isolated in
//!   the `ffi` module with every safety obligation documented at the call site.
//! - All work is bounded by the named limits below; inputs arriving over
//!   the FFI boundary are validated before any device work is queued.
//!
//! # Hardware scope
//!
//! Primo's RTX 4070 Laptop (8 GiB). GPU memory state is checked before
//! any measured run: a co-resident process holding device memory
//! invalidates timing and can corrupt results on some stacks (the Burn
//! track's CubeCL find), so measured runs record the free-memory figure
//! they observed.

/// Maximum tokens scored in one logprob-reduction launch.
pub const TOKENS_PER_LAUNCH_MAX: u32 = 65_536;
/// Maximum vocabulary width accepted by the logprob kernel (Qwen2.5 is
/// 151,936; the cap leaves headroom without admitting absurd shapes).
pub const VOCAB_SIZE_MAX: u32 = 262_144;
/// Maximum completions in one RLOO group (trainlab's GROUP_SIZE_MAX is
/// 64; this cap is the kernel's own bound, stated independently).
pub const GROUP_COMPLETIONS_MAX: u32 = 1_024;
/// Maximum groups scored in one advantage-kernel launch.
pub const GROUPS_PER_LAUNCH_MAX: u32 = 4_096;
