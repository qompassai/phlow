//! Mojo kernel contracts for phlow: descriptors, launch validation, budgets.
//!
//! This crate adapts the *concepts* of Mojo's GPU programming model into
//! Tiger Style Rust. It compiles no Mojo, touches no GPU, and contains no
//! Mojo code: what it provides is the host-side contract a kernel must
//! satisfy before device work is planned.
//!
//! # What is real vs. interface
//!
//! - **Real:** [`KernelName`] validation, [`KernelDescriptor`] contracts,
//!   the [`KernelRegistry`], [`LaunchConfig`] validation against
//!   [`DeviceLimits`] (checked arithmetic throughout), [`LaunchBudget`]
//!   accounting in [`LaunchPlanner`], and the typed [`KernelError`].
//! - **Interface:** [`KernelExecutor`] is the boundary a real device backend
//!   implements (`launch` + `synchronize`, mirroring Mojo's
//!   `enqueue_function` + `DeviceContext.synchronize()`). The `simulated`
//!   feature's [`SimulatedExecutor`] is an in-process test double: it
//!   performs no GPU work and is never a substitute for a device.
//!
//! # Adapted Mojo concepts
//!
//! - *Kernel as a function identity.* In Mojo a kernel is a function passed
//!   as a compile-time parameter to `enqueue_function`; here it is a
//!   [`KernelName`] plus a [`KernelVersion`], resolved through the registry.
//! - *`grid_dim` / `block_dim` launch geometry.* [`Dim3`] models Mojo's
//!   1D/2D/3D grid and block dimensions; [`LaunchConfig::validate`] enforces
//!   the device limits the driver would otherwise report as raw errors.
//! - *Asynchronous enqueue + synchronize.* The executor boundary keeps
//!   Mojo's two-phase shape: `launch` enqueues, `synchronize` drains.
//! - *Compile-time argument checking.* Mojo type-checks kernel arguments at
//!   compile time; here the descriptor's `arg_count_max` /
//!   `payload_bytes_max` are checked at validation time, before planning.
//!
//! # Limits
//!
//! Every bound is a named constant next to the code it guards:
//! [`KERNEL_NAME_CHARS_MAX`], [`KERNELS_MAX`], [`ARG_COUNT_MAX`],
//! [`PAYLOAD_BYTES_MAX`], [`THREADS_PER_BLOCK_MAX`],
//! [`SHARED_MEMORY_BYTES_PER_BLOCK_MAX`], [`GRID_DIM_MAX`],
//! [`TOTAL_THREADS_PER_LAUNCH_MAX`], [`LAUNCHES_MAX`],
//! [`BUDGET_PAYLOAD_BYTES_MAX`], [`REASON_CHARS_MAX`].
//!
//! # Layout
//!
//! [`error`] owns the error type; [`descriptor`] owns kernel identity and
//! the registry; [`launch`] owns launch geometry and validation;
//! [`budget`] owns planning budgets; [`executor`] owns the device boundary
//! and the simulated executor.

#![forbid(unsafe_code)]

pub mod budget;
pub mod descriptor;
pub mod error;
pub mod executor;
pub mod launch;

pub use budget::{BUDGET_PAYLOAD_BYTES_MAX, LAUNCHES_MAX, LaunchBudget, LaunchPlan, LaunchPlanner};
pub use descriptor::{
    ARG_COUNT_MAX, KERNEL_NAME_CHARS_MAX, KERNELS_MAX, KernelDescriptor, KernelName,
    KernelRegistry, KernelVersion, PAYLOAD_BYTES_MAX,
};
pub use error::{DimAxis, KernelError};
#[cfg(feature = "simulated")]
pub use executor::SimulatedExecutor;
pub use executor::{KernelExecutor, LaunchReceipt, REASON_CHARS_MAX};
pub use launch::{
    DeviceLimits, Dim3, GRID_DIM_MAX, LaunchConfig, SHARED_MEMORY_BYTES_PER_BLOCK_MAX,
    THREADS_PER_BLOCK_MAX, TOTAL_THREADS_PER_LAUNCH_MAX, ValidatedLaunch,
};
