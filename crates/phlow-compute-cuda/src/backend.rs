//! The [`CudaBackend`] trait and its deterministic CPU simulator.
//!
//! [`CudaBackend`] is the single GPU interaction point: allocation, copies,
//! launch, and synchronization. Every method is bounded and fallible. The
//! [`SimulatedBackend`] implements the trait against a plain memory model
//! with registered per-kernel behaviors, so the launch pipeline —
//! descriptor, config validation, argument marshaling — is exercised with
//! no CUDA hardware.

use crate::error::CudaError;
use crate::kernel::KernelDescriptor;
use crate::launch::{LaunchConfig, ValidatedLaunch};
use std::collections::HashMap;

/// Simulated device memory budget, in bytes: 1 GiB.
pub const DEVICE_MEMORY_BYTES_MAX: u64 = 1 << 30;

/// Most device pointers per launch.
pub const LAUNCH_ARG_PTRS_MAX: usize = 16;

/// Most scalar words per launch.
pub const LAUNCH_ARG_WORDS_MAX: usize = 64;

/// Largest single simulated allocation, in bytes.
const ALLOC_BYTES_MAX: u64 = 1 << 30;

/// An opaque device allocation: an id plus its byte size.
///
/// The id is meaningful only to the backend that issued it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceAlloc {
    /// Backend-issued allocation id.
    pub id: u64,
    /// Allocation size in bytes.
    pub bytes: u64,
}

/// Launch arguments: device pointers plus scalar words.
///
/// Pointers are allocation ids issued by the same backend; scalars are
/// passed by value. Both lists are bounded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchArgs {
    /// Device allocation ids, in kernel parameter order.
    pub device_ptrs: Vec<u64>,
    /// Scalar arguments as 64-bit words, in parameter order.
    pub scalar_words: Vec<u64>,
}

impl LaunchArgs {
    /// Bundles launch arguments.
    ///
    /// # Contract
    /// - Accepts: at most [`LAUNCH_ARG_PTRS_MAX`] pointers and
    ///   [`LAUNCH_ARG_WORDS_MAX`] scalar words.
    /// - Rejects: longer lists — with a typed error.
    pub fn new(device_ptrs: Vec<u64>, scalar_words: Vec<u64>) -> Result<Self, CudaError> {
        if device_ptrs.len() > LAUNCH_ARG_PTRS_MAX {
            return Err(CudaError::TooManyArgs {
                what: "device_ptrs",
                count: device_ptrs.len(),
            });
        }
        if scalar_words.len() > LAUNCH_ARG_WORDS_MAX {
            return Err(CudaError::TooManyArgs {
                what: "scalar_words",
                count: scalar_words.len(),
            });
        }
        Ok(Self {
            device_ptrs,
            scalar_words,
        })
    }
}

/// Proof that a launch was accepted by the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchReceipt {
    /// Backend-issued launch id, increasing per launch.
    pub id: u64,
}

/// The GPU interaction point. All implementors — real driver bindings or
/// the simulator — obey these bounds: allocations inside the device budget,
/// copies that exactly match allocation sizes, launches of validated
/// configs only.
pub trait CudaBackend {
    /// The backend's error type.
    type Error: std::error::Error;

    /// Allocates `bytes` of device memory.
    fn alloc(&mut self, bytes: u64) -> Result<DeviceAlloc, Self::Error>;

    /// Frees a previous allocation. Double-free is an error.
    fn free(&mut self, alloc: DeviceAlloc) -> Result<(), Self::Error>;

    /// Copies host bytes into a device allocation. Sizes must match
    /// exactly.
    fn write(&mut self, dst: &DeviceAlloc, src: &[u8]) -> Result<(), Self::Error>;

    /// Copies a device allocation out to host bytes. Sizes must match
    /// exactly.
    fn read(&self, src: &DeviceAlloc, dst: &mut [u8]) -> Result<(), Self::Error>;

    /// Launches `kernel` with `config` and `args`.
    fn launch(
        &mut self,
        kernel: &KernelDescriptor,
        config: &LaunchConfig,
        args: &LaunchArgs,
    ) -> Result<LaunchReceipt, Self::Error>;

    /// Waits for previously launched work to complete.
    fn synchronize(&mut self) -> Result<(), Self::Error>;
}

/// Failure reported by a registered simulator behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BehaviorError {
    /// Static reason; never device memory contents.
    pub detail: &'static str,
}

/// A simulated kernel behavior: receives the launch's buffers as owned
/// byte vectors plus the validated launch, and mutates the buffers the way
/// the kernel would.
///
/// The signature is a plain function pointer — `Copy`, deterministic, and
/// free of closure-captured state that could make replays diverge.
pub type KernelBehavior = fn(&mut [Vec<u8>], &ValidatedLaunch) -> Result<(), BehaviorError>;

/// Deterministic CPU simulator of a CUDA device.
///
/// Memory model: a map from allocation id to byte vector, with an aggregate
/// budget of [`DEVICE_MEMORY_BYTES_MAX`]. Launches look up the kernel's
/// registered behavior by name and run it against the launch's buffers.
/// Buffers are temporarily removed from the map while the behavior runs and
/// restored afterwards — on both success and failure — so a failed launch
/// never loses device memory.
pub struct SimulatedBackend {
    memory: HashMap<u64, Vec<u8>>,
    used_bytes: u64,
    next_alloc_id: u64,
    next_launch_id: u64,
    behaviors: HashMap<String, KernelBehavior>,
}

impl SimulatedBackend {
    /// Creates an empty simulator: no allocations, no behaviors.
    pub fn new() -> Self {
        Self {
            memory: HashMap::new(),
            used_bytes: 0,
            next_alloc_id: 1,
            next_launch_id: 1,
            behaviors: HashMap::new(),
        }
    }

    /// Registers the behavior for a kernel name, replacing any previous
    /// one. Registration is host-side setup, not device state.
    pub fn register_behavior(&mut self, kernel_name: &str, behavior: KernelBehavior) {
        self.behaviors.insert(kernel_name.to_string(), behavior);
    }

    /// Device bytes currently allocated.
    pub fn used_bytes(&self) -> u64 {
        self.used_bytes
    }

    /// Restores buffers to the memory map after a behavior ran. Private:
    /// every launch path must call it exactly once per removed buffer.
    fn restore(&mut self, ids: &[u64], buffers: Vec<Vec<u8>>) {
        for (id, buffer) in ids.iter().zip(buffers) {
            self.memory.insert(*id, buffer);
        }
    }
}

impl Default for SimulatedBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl CudaBackend for SimulatedBackend {
    type Error = CudaError;

    fn alloc(&mut self, bytes: u64) -> Result<DeviceAlloc, Self::Error> {
        if bytes == 0 || bytes > ALLOC_BYTES_MAX {
            return Err(CudaError::InvalidAllocSize { bytes });
        }
        let available = DEVICE_MEMORY_BYTES_MAX.saturating_sub(self.used_bytes);
        if bytes > available {
            return Err(CudaError::OutOfMemory {
                requested: bytes,
                available,
            });
        }
        let id = self.next_alloc_id;
        self.next_alloc_id = self
            .next_alloc_id
            .checked_add(1)
            .ok_or(CudaError::OutOfMemory {
                requested: 1,
                available: 0,
            })?;
        self.memory.insert(id, vec![0u8; bytes as usize]);
        // bytes <= 1 GiB and used_bytes + bytes <= DEVICE_MEMORY_BYTES_MAX.
        self.used_bytes += bytes;
        Ok(DeviceAlloc { id, bytes })
    }

    fn free(&mut self, alloc: DeviceAlloc) -> Result<(), Self::Error> {
        match self.memory.remove(&alloc.id) {
            Some(buffer) => {
                // The stored buffer always has the issued size.
                debug_assert_eq!(buffer.len() as u64, alloc.bytes);
                self.used_bytes = self.used_bytes.saturating_sub(alloc.bytes);
                Ok(())
            }
            None => Err(CudaError::DoubleFree { id: alloc.id }),
        }
    }

    fn write(&mut self, dst: &DeviceAlloc, src: &[u8]) -> Result<(), Self::Error> {
        let buffer = self
            .memory
            .get_mut(&dst.id)
            .ok_or(CudaError::UnknownAlloc { id: dst.id })?;
        if buffer.len() as u64 != src.len() as u64 || buffer.len() as u64 != dst.bytes {
            return Err(CudaError::CopySizeMismatch {
                alloc_bytes: dst.bytes,
                copy_bytes: src.len() as u64,
            });
        }
        buffer.copy_from_slice(src);
        Ok(())
    }

    fn read(&self, src: &DeviceAlloc, dst: &mut [u8]) -> Result<(), Self::Error> {
        let buffer = self
            .memory
            .get(&src.id)
            .ok_or(CudaError::UnknownAlloc { id: src.id })?;
        if buffer.len() != dst.len() || buffer.len() as u64 != src.bytes {
            return Err(CudaError::CopySizeMismatch {
                alloc_bytes: src.bytes,
                copy_bytes: dst.len() as u64,
            });
        }
        dst.copy_from_slice(buffer);
        Ok(())
    }

    fn launch(
        &mut self,
        kernel: &KernelDescriptor,
        config: &LaunchConfig,
        args: &LaunchArgs,
    ) -> Result<LaunchReceipt, Self::Error> {
        let validated = config.validate()?;
        let behavior = self
            .behaviors
            .get(kernel.name.as_str())
            .copied()
            .ok_or_else(|| CudaError::unknown_kernel(kernel.name.as_str()))?;
        // Remove buffers so the behavior owns them; restore on every path.
        let mut buffers: Vec<Vec<u8>> = Vec::with_capacity(args.device_ptrs.len());
        for id in &args.device_ptrs {
            match self.memory.remove(id) {
                Some(buffer) => buffers.push(buffer),
                None => {
                    self.restore(&args.device_ptrs[..buffers.len()], buffers);
                    return Err(CudaError::UnknownAlloc { id: *id });
                }
            }
        }
        let outcome = behavior(&mut buffers, &validated);
        self.restore(&args.device_ptrs, buffers);
        if let Err(failure) = outcome {
            return Err(CudaError::kernel_behavior_failed(
                kernel.name.as_str(),
                failure.detail,
            ));
        }
        let id = self.next_launch_id;
        // The id space holds 2^64 launches; exhaustion is reported rather
        // than allowed to wrap and hand out a duplicate receipt.
        self.next_launch_id = self
            .next_launch_id
            .checked_add(1)
            .ok_or(CudaError::OutOfMemory {
                requested: 1,
                available: 0,
            })?;
        Ok(LaunchReceipt { id })
    }

    fn synchronize(&mut self) -> Result<(), Self::Error> {
        // The simulator runs behaviors inline: there is never pending work.
        Ok(())
    }
}
