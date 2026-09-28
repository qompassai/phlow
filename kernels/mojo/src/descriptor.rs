//! Kernel descriptors and the kernel registry.
//!
//! A [`KernelDescriptor`] is the Rust-side identity of a Mojo kernel function:
//! its name (the compile-time function identity Mojo's `enqueue_function`
//! takes as a parameter), its version, and the resource contract it declares
//! (argument count, payload bytes, shared memory). The [`KernelRegistry`]
//! holds validated descriptors; launches resolve against it, so an unknown
//! or mistyped kernel fails before any device work is planned.

use std::collections::BTreeMap;

use crate::error::KernelError;

/// Maximum kernel name length, in characters.
pub const KERNEL_NAME_CHARS_MAX: usize = 128;
/// Maximum kernels the registry holds.
pub const KERNELS_MAX: usize = 256;
/// Maximum arguments any kernel may declare.
pub const ARG_COUNT_MAX: u8 = 32;
/// Global per-launch payload byte cap (16 MiB).
pub const PAYLOAD_BYTES_MAX: u32 = 16 * 1024 * 1024;

/// A validated kernel name.
///
/// The constructor enforces: non-empty, at most [`KERNEL_NAME_CHARS_MAX`]
/// characters, ASCII alphanumeric or `_` only. Validation happens before any
/// allocation beyond the name string itself.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KernelName(String);

impl KernelName {
    /// Validate `name` and wrap it.
    ///
    /// # Errors
    ///
    /// [`KernelError::NameEmpty`], [`KernelError::NameTooLong`], or
    /// [`KernelError::NameInvalidChar`] on rejected input. The registry and
    /// all other state are untouched.
    pub fn new(name: &str) -> Result<Self, KernelError> {
        if name.is_empty() {
            return Err(KernelError::NameEmpty);
        }
        let chars = name.chars().count();
        if chars > KERNEL_NAME_CHARS_MAX {
            return Err(KernelError::NameTooLong {
                chars,
                max: KERNEL_NAME_CHARS_MAX,
            });
        }
        // Check the charset before allocating the owned copy.
        for ch in name.chars() {
            if !(ch.is_ascii_alphanumeric() || ch == '_') {
                return Err(KernelError::NameInvalidChar { ch });
            }
        }
        Ok(Self(name.to_owned()))
    }

    /// The validated name text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Semantic version of a kernel implementation.
///
/// Kernels evolve (tuned variants, fixed indexing bugs); the version keeps a
/// stale launch plan from silently binding to a new implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KernelVersion {
    /// Major version.
    pub major: u16,
    /// Minor version.
    pub minor: u16,
    /// Patch version.
    pub patch: u16,
}

impl KernelVersion {
    /// Build a version triple. Plain data; no validation needed.
    #[must_use]
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

/// The resource contract a Mojo kernel declares.
///
/// This mirrors what a kernel's signature plus its compile-time parameters
/// tell the host: how many arguments it takes, the largest payload it can
/// consume per launch, and how much shared memory it requires. The launch
/// validator checks the launch against these before planning device work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelDescriptor {
    name: KernelName,
    version: KernelVersion,
    arg_count_max: u8,
    payload_bytes_max: u32,
    shared_memory_bytes: u32,
}

impl KernelDescriptor {
    /// Build a descriptor, validating the numeric contract.
    ///
    /// # Errors
    ///
    /// [`KernelError::ArgCountExceeded`] if `arg_count_max` tops
    /// [`ARG_COUNT_MAX`]; [`KernelError::PayloadBytesExceeded`] if
    /// `payload_bytes_max` tops [`PAYLOAD_BYTES_MAX`].
    pub fn new(
        name: KernelName,
        version: KernelVersion,
        arg_count_max: u8,
        payload_bytes_max: u32,
        shared_memory_bytes: u32,
    ) -> Result<Self, KernelError> {
        if arg_count_max > ARG_COUNT_MAX {
            return Err(KernelError::ArgCountExceeded {
                requested: arg_count_max,
                max: ARG_COUNT_MAX,
            });
        }
        if payload_bytes_max > PAYLOAD_BYTES_MAX {
            return Err(KernelError::PayloadBytesExceeded {
                requested: payload_bytes_max,
                limit: PAYLOAD_BYTES_MAX,
            });
        }
        Ok(Self {
            name,
            version,
            arg_count_max,
            payload_bytes_max,
            shared_memory_bytes,
        })
    }

    /// The kernel's validated name.
    #[must_use]
    pub fn name(&self) -> &KernelName {
        &self.name
    }

    /// The kernel's version triple.
    #[must_use]
    pub fn version(&self) -> KernelVersion {
        self.version
    }

    /// Maximum arguments this kernel accepts per launch.
    #[must_use]
    pub fn arg_count_max(&self) -> u8 {
        self.arg_count_max
    }

    /// Largest payload, in bytes, this kernel accepts per launch.
    #[must_use]
    pub fn payload_bytes_max(&self) -> u32 {
        self.payload_bytes_max
    }

    /// Shared memory, in bytes, this kernel requires per block.
    #[must_use]
    pub fn shared_memory_bytes(&self) -> u32 {
        self.shared_memory_bytes
    }
}

/// Registry of validated kernel descriptors.
///
/// Backed by a `BTreeMap` so iteration order is deterministic (sorted by
/// name). Registration is validate-then-commit: a rejected registration
/// leaves every previously registered descriptor untouched.
#[derive(Debug, Default)]
pub struct KernelRegistry {
    kernels: BTreeMap<KernelName, KernelDescriptor>,
}

impl KernelRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a validated descriptor.
    ///
    /// # Errors
    ///
    /// [`KernelError::DuplicateKernel`] if the name is taken (the original
    /// descriptor is kept); [`KernelError::RegistryFull`] at
    /// [`KERNELS_MAX`] entries. Either way the registry is unchanged.
    pub fn register(&mut self, descriptor: KernelDescriptor) -> Result<(), KernelError> {
        if self.kernels.contains_key(descriptor.name()) {
            return Err(KernelError::DuplicateKernel {
                name: descriptor.name().as_str().to_owned(),
            });
        }
        if self.kernels.len() >= KERNELS_MAX {
            return Err(KernelError::RegistryFull { max: KERNELS_MAX });
        }
        self.kernels.insert(descriptor.name().clone(), descriptor);
        Ok(())
    }

    /// Look up a descriptor by name.
    #[must_use]
    pub fn get(&self, name: &KernelName) -> Option<&KernelDescriptor> {
        self.kernels.get(name)
    }

    /// Number of registered kernels.
    #[must_use]
    pub fn len(&self) -> usize {
        self.kernels.len()
    }

    /// Whether the registry holds no kernels.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kernels.is_empty()
    }
}
