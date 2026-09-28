//! Integration tests for `phlow-mojo-kernels`.
//!
//! Inventory: 20 tests, exactly 10 validation (v_*) and 10 adversarial
//! (a_*). Validation tests prove the contract accepts what it should;
//! adversarial tests throw hostile, boundary-violating, or oversized input
//! at every check and assert typed rejection without state corruption.

use phlow_mojo_kernels::{
    DeviceLimits, Dim3, KERNEL_NAME_CHARS_MAX, KernelDescriptor, KernelError, KernelName,
    KernelRegistry, KernelVersion, LaunchBudget, LaunchConfig, LaunchPlan, LaunchPlanner,
    PAYLOAD_BYTES_MAX, SHARED_MEMORY_BYTES_PER_BLOCK_MAX, SimulatedExecutor, THREADS_PER_BLOCK_MAX,
};
use phlow_mojo_kernels::{KernelExecutor, LaunchReceipt};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Registry holding one kernel: `vec_add` v1.0.0, 4 args max, 1 KiB
/// payload max, no shared memory required.
fn registry_with_vec_add() -> KernelRegistry {
    let mut registry = KernelRegistry::new();
    let descriptor = KernelDescriptor::new(
        KernelName::new("vec_add").unwrap(),
        KernelVersion::new(1, 0, 0),
        4,
        1024,
        0,
    )
    .unwrap();
    registry.register(descriptor).unwrap();
    registry
}

/// Minimal valid launch against `vec_add`: 1 block, 1 thread, no payload.
fn minimal_launch() -> LaunchConfig {
    LaunchConfig::new(
        KernelName::new("vec_add").unwrap(),
        Dim3::one_d(1),
        Dim3::one_d(1),
        0,
        0,
        0,
    )
}

fn validate_minimal() -> phlow_mojo_kernels::ValidatedLaunch {
    minimal_launch()
        .validate(&registry_with_vec_add(), &DeviceLimits::generic())
        .unwrap()
}

// ---------------------------------------------------------------------------
// Validation (10)
// ---------------------------------------------------------------------------

#[test]
fn v_register_lookup_roundtrip() {
    let registry = registry_with_vec_add();
    let name = KernelName::new("vec_add").unwrap();
    let descriptor = registry.get(&name).unwrap();
    assert_eq!(descriptor.version(), KernelVersion::new(1, 0, 0));
    assert_eq!(descriptor.arg_count_max(), 4);
    assert_eq!(descriptor.payload_bytes_max(), 1024);
    assert_eq!(descriptor.shared_memory_bytes(), 0);
    assert_eq!(registry.len(), 1);
}

#[test]
fn v_name_exact_limit_accepted() {
    let name: String = "a".repeat(KERNEL_NAME_CHARS_MAX);
    let kernel = KernelName::new(&name).unwrap();
    assert_eq!(kernel.as_str().len(), KERNEL_NAME_CHARS_MAX);
}

#[test]
fn v_minimal_1d_launch_validated() {
    let launch = validate_minimal();
    assert_eq!(launch.total_threads(), 1);
    assert_eq!(launch.grid(), Dim3::one_d(1));
    assert_eq!(launch.block(), Dim3::one_d(1));
}

#[test]
fn v_3d_launch_validated() {
    // Mirrors the Mojo docs' 2x2x1 grid of 4x4x2 blocks: 4 * 32 = 128 threads.
    let config = LaunchConfig::new(
        KernelName::new("vec_add").unwrap(),
        Dim3::new(2, 2, 1),
        Dim3::new(4, 4, 2),
        0,
        2,
        512,
    );
    let launch = config
        .validate(&registry_with_vec_add(), &DeviceLimits::generic())
        .unwrap();
    assert_eq!(launch.total_threads(), 128);
}

#[test]
fn v_threads_per_block_exact_limit() {
    let config = LaunchConfig::new(
        KernelName::new("vec_add").unwrap(),
        Dim3::one_d(1),
        Dim3::one_d(THREADS_PER_BLOCK_MAX),
        0,
        0,
        0,
    );
    let launch = config
        .validate(&registry_with_vec_add(), &DeviceLimits::generic())
        .unwrap();
    assert_eq!(launch.total_threads(), u64::from(THREADS_PER_BLOCK_MAX));
}

#[test]
fn v_shared_memory_exact_limit() {
    let mut registry = KernelRegistry::new();
    registry
        .register(
            KernelDescriptor::new(
                KernelName::new("shared_kernel").unwrap(),
                KernelVersion::new(1, 0, 0),
                1,
                64,
                SHARED_MEMORY_BYTES_PER_BLOCK_MAX,
            )
            .unwrap(),
        )
        .unwrap();
    let config = LaunchConfig::new(
        KernelName::new("shared_kernel").unwrap(),
        Dim3::one_d(1),
        Dim3::one_d(1),
        SHARED_MEMORY_BYTES_PER_BLOCK_MAX,
        1,
        64,
    );
    let launch = config
        .validate(&registry, &DeviceLimits::generic())
        .unwrap();
    assert_eq!(
        launch.shared_memory_bytes(),
        SHARED_MEMORY_BYTES_PER_BLOCK_MAX
    );
}

#[test]
fn v_total_threads_computed_correctly() {
    let config = LaunchConfig::new(
        KernelName::new("vec_add").unwrap(),
        Dim3::new(3, 5, 2),  // 30 blocks
        Dim3::new(16, 4, 2), // 128 threads/block
        0,
        0,
        0,
    );
    let launch = config
        .validate(&registry_with_vec_add(), &DeviceLimits::generic())
        .unwrap();
    assert_eq!(launch.total_threads(), 30 * 128);
}

#[test]
fn v_arg_count_exact_limit_accepted() {
    let config = LaunchConfig::new(
        KernelName::new("vec_add").unwrap(),
        Dim3::one_d(1),
        Dim3::one_d(1),
        0,
        4,
        0,
    );
    let launch = config
        .validate(&registry_with_vec_add(), &DeviceLimits::generic())
        .unwrap();
    assert_eq!(launch.arg_count(), 4);
}

#[test]
fn v_payload_exact_limit_accepted() {
    // vec_add allows exactly 1024 payload bytes; the boundary is accepted.
    let config = LaunchConfig::new(
        KernelName::new("vec_add").unwrap(),
        Dim3::one_d(1),
        Dim3::one_d(1),
        0,
        0,
        1024,
    );
    let launch = config
        .validate(&registry_with_vec_add(), &DeviceLimits::generic())
        .unwrap();
    assert_eq!(launch.payload_bytes(), 1024);
}

#[test]
fn v_plan_commits_budget_once() {
    let budget = LaunchBudget::new(2, 2048).unwrap();
    let mut planner = LaunchPlanner::new(budget);
    let plan: LaunchPlan = planner.plan(validate_minimal()).unwrap();
    assert_eq!(plan.sequence(), 0);
    assert_eq!(planner.launches_used(), 1);
    assert_eq!(planner.payload_bytes_used(), 0);
}

#[test]
fn v_simulated_executor_receipt_matches_plan() {
    let mut planner = LaunchPlanner::new(LaunchBudget::new(4, 4096).unwrap());
    let plan = planner.plan(validate_minimal()).unwrap();
    let mut executor = SimulatedExecutor::new();
    let receipt: LaunchReceipt = executor.launch(&plan).unwrap();
    executor.synchronize().unwrap();
    assert_eq!(receipt.plan_sequence(), 0);
    assert_eq!(receipt.kernel(), "vec_add");
    assert_eq!(receipt.total_threads(), 1);
    // Deterministic: the same plan always yields the same checksum.
    let mut executor2 = SimulatedExecutor::new();
    let receipt2 = executor2.launch(&plan).unwrap();
    assert_eq!(receipt.checksum(), receipt2.checksum());
    assert_eq!(executor.launches(), 1);
}

// ---------------------------------------------------------------------------
// Adversarial (10)
// ---------------------------------------------------------------------------

#[test]
fn a_empty_name_rejected() {
    assert_eq!(KernelName::new(""), Err(KernelError::NameEmpty));
}

#[test]
fn a_name_too_long_rejected() {
    let name: String = "b".repeat(KERNEL_NAME_CHARS_MAX + 1);
    assert_eq!(
        KernelName::new(&name),
        Err(KernelError::NameTooLong {
            chars: KERNEL_NAME_CHARS_MAX + 1,
            max: KERNEL_NAME_CHARS_MAX,
        })
    );
}

#[test]
fn a_name_invalid_char_rejected() {
    assert_eq!(
        KernelName::new("vec-add"),
        Err(KernelError::NameInvalidChar { ch: '-' })
    );
    assert_eq!(
        KernelName::new("vec add"),
        Err(KernelError::NameInvalidChar { ch: ' ' })
    );
}

#[test]
fn a_duplicate_registration_keeps_original() {
    let mut registry = registry_with_vec_add();
    let v2 = KernelDescriptor::new(
        KernelName::new("vec_add").unwrap(),
        KernelVersion::new(2, 0, 0),
        4,
        1024,
        0,
    )
    .unwrap();
    assert_eq!(
        registry.register(v2),
        Err(KernelError::DuplicateKernel {
            name: "vec_add".to_owned(),
        })
    );
    // The original descriptor is untouched.
    let kept = registry.get(&KernelName::new("vec_add").unwrap()).unwrap();
    assert_eq!(kept.version(), KernelVersion::new(1, 0, 0));
    assert_eq!(registry.len(), 1);
}

#[test]
fn a_unknown_kernel_rejected() {
    let registry = registry_with_vec_add();
    let config = LaunchConfig::new(
        KernelName::new("nope_missing").unwrap(),
        Dim3::one_d(1),
        Dim3::one_d(1),
        0,
        0,
        0,
    );
    assert_eq!(
        config.validate(&registry, &DeviceLimits::generic()),
        Err(KernelError::UnknownKernel {
            name: "nope_missing".to_owned(),
        })
    );
}

#[test]
fn a_zero_block_dim_rejected() {
    let config = LaunchConfig::new(
        KernelName::new("vec_add").unwrap(),
        Dim3::one_d(1),
        Dim3::one_d(0),
        0,
        0,
        0,
    );
    let result = config.validate(&registry_with_vec_add(), &DeviceLimits::generic());
    assert!(matches!(result, Err(KernelError::ZeroDim { .. })));
}

#[test]
fn a_threads_per_block_exceeded() {
    let config = LaunchConfig::new(
        KernelName::new("vec_add").unwrap(),
        Dim3::one_d(1),
        Dim3::one_d(THREADS_PER_BLOCK_MAX + 1),
        0,
        0,
        0,
    );
    assert_eq!(
        config.validate(&registry_with_vec_add(), &DeviceLimits::generic()),
        Err(KernelError::ThreadsPerBlockExceeded {
            requested: u64::from(THREADS_PER_BLOCK_MAX) + 1,
            limit: THREADS_PER_BLOCK_MAX,
        })
    );
}

#[test]
fn a_dim_product_overflow_rejected() {
    // With a device profile that allows enormous blocks, grid-blocks x
    // block-threads can overflow u64. Must be a typed error, never a panic
    // or wrap. (With the generic 1024-thread limit the per-block check
    // fires first; the overflow path is defense in depth for wider
    // profiles.)
    let grid = Dim3::new(65_535, 65_535, 65_535); // 65535^3 blocks
    let block = Dim3::new(65_535, 65_535, 1); // 65535^2 <= u32::MAX threads
    let config = LaunchConfig::new(KernelName::new("vec_add").unwrap(), grid, block, 0, 0, 0);
    let limits = DeviceLimits {
        threads_per_block_max: u32::MAX,
        total_threads_per_launch_max: u64::MAX,
        ..DeviceLimits::generic()
    };
    assert_eq!(
        config.validate(&registry_with_vec_add(), &limits),
        Err(KernelError::ArithmeticOverflow {
            what: "total thread product",
        })
    );
}

#[test]
fn a_shared_memory_exceeded() {
    let config = LaunchConfig::new(
        KernelName::new("vec_add").unwrap(),
        Dim3::one_d(1),
        Dim3::one_d(1),
        SHARED_MEMORY_BYTES_PER_BLOCK_MAX + 1,
        0,
        0,
    );
    assert_eq!(
        config.validate(&registry_with_vec_add(), &DeviceLimits::generic()),
        Err(KernelError::SharedMemoryExceeded {
            requested: SHARED_MEMORY_BYTES_PER_BLOCK_MAX + 1,
            limit: SHARED_MEMORY_BYTES_PER_BLOCK_MAX,
        })
    );
}

#[test]
fn a_total_threads_exceeded() {
    // 65535^2 blocks x 1 thread: 4,294,836,225 > 2^31 budget.
    let config = LaunchConfig::new(
        KernelName::new("vec_add").unwrap(),
        Dim3::new(65_535, 65_535, 1),
        Dim3::one_d(1),
        0,
        0,
        0,
    );
    let result = config.validate(&registry_with_vec_add(), &DeviceLimits::generic());
    assert!(matches!(
        result,
        Err(KernelError::TotalThreadsExceeded { .. })
    ));
}

#[test]
fn a_payload_bytes_exceeded() {
    // vec_add allows 1024 payload bytes; the global cap is far larger, so
    // the kernel's own limit is the binding one.
    let config = LaunchConfig::new(
        KernelName::new("vec_add").unwrap(),
        Dim3::one_d(1),
        Dim3::one_d(1),
        0,
        0,
        1025,
    );
    assert_eq!(
        config.validate(&registry_with_vec_add(), &DeviceLimits::generic()),
        Err(KernelError::PayloadBytesExceeded {
            requested: 1025,
            limit: 1024,
        })
    );
    // And a descriptor cannot even declare more than the global cap.
    assert_eq!(
        KernelDescriptor::new(
            KernelName::new("greedy").unwrap(),
            KernelVersion::new(1, 0, 0),
            1,
            PAYLOAD_BYTES_MAX + 1,
            0,
        ),
        Err(KernelError::PayloadBytesExceeded {
            requested: PAYLOAD_BYTES_MAX + 1,
            limit: PAYLOAD_BYTES_MAX,
        })
    );
}
