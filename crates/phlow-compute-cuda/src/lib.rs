//! CUDA kernel-target model for phlow's GPU lanes.
//!
//! Plain words: before a kernel can run, the host must describe it — which
//! PTX or cubin holds it, which entry point to call, which architecture it
//! targets, and how to launch it (grid, block, shared memory). This crate
//! provides validated types for all of that, plus a compile-time
//! specialization policy (one kernel source, several tuned variants) and a
//! bounded [`CudaBackend`] trait with a deterministic CPU simulator, so the
//! whole launch pipeline compiles and tests with no GPU present.
//!
//! Concept provenance: the kernel-descriptor / PTX-artifact / launch-config
//! split, the host-runtime shape, and compile-time kernel policies are
//! adapted from NVlabs `cuda-oxide`
//! (https://github.com/nvlabs/cuda-oxide, Apache-2.0). Every line of code
//! and prose here is original; no upstream source was copied.
//!
//! # Bounds
//!
//! - [`PTX_BYTES_MAX`]: largest PTX text accepted.
//! - [`BLOCK_THREADS_MAX`]: most threads in one thread block (1024, the CUDA
//!   limit).
//! - [`SHARED_MEM_BYTES_MAX`]: most dynamic shared memory per launch.
//! - [`GRID_DIM_AXIS_MAX`]: largest single grid axis (`2^31 - 1`).
//! - [`DEVICE_MEMORY_BYTES_MAX`]: simulated device memory budget.
//!
//! # Unsafe policy
//!
//! This crate forbids unsafe code outright. There is no CUDA driver binding
//! here — only descriptors, validation, and the simulator.

#![forbid(unsafe_code)]

mod arch;
mod backend;
mod descriptor_file;
mod error;
mod kernel;
mod launch;
mod policy;
mod ptx;

pub use arch::{ComputeArch, PtxVersion};
pub use backend::{
    BehaviorError, CudaBackend, DEVICE_MEMORY_BYTES_MAX, DeviceAlloc, KernelBehavior,
    LAUNCH_ARG_PTRS_MAX, LAUNCH_ARG_WORDS_MAX, LaunchArgs, LaunchReceipt, SimulatedBackend,
};
pub use descriptor_file::{DESCRIPTOR_BYTES_MAX, parse_descriptor_file};
pub use error::CudaError;
pub use kernel::{EntryName, KernelDescriptor, KernelName, KernelPath, KernelSource};
pub use launch::{
    Axis, BLOCK_THREADS_MAX, Dim3, GRID_DIM_AXIS_MAX, LaunchConfig, SHARED_MEM_BYTES_MAX,
    ValidatedLaunch,
};
pub use policy::{KernelPolicy, Specialization};
pub use ptx::{PTX_BYTES_MAX, PTX_ENTRY_COUNT_MAX, PtxModule};
