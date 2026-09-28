//! Adversarial tests: invalid PTX, descriptors, launch configs, and
//! simulator misuse are rejected with typed errors — never panics, never
//! silent corruption.

use phlow_compute_cuda::{
    BehaviorError, ComputeArch, CudaBackend, CudaError, DEVICE_MEMORY_BYTES_MAX, Dim3, EntryName,
    KernelName, KernelPolicy, LaunchArgs, LaunchConfig, PTX_BYTES_MAX, PtxModule, PtxVersion,
    SimulatedBackend, ValidatedLaunch, parse_descriptor_file,
};

const VALID_PTX: &str = include_str!("../../../kernels/ptx/valid_add.ptx");
const NO_VERSION_PTX: &str = include_str!("../../../kernels/ptx/invalid_no_version.ptx");
const NO_TARGET_PTX: &str = include_str!("../../../kernels/ptx/invalid_no_target.ptx");

fn descriptor() -> phlow_compute_cuda::KernelDescriptor {
    use phlow_compute_cuda::{KernelDescriptor, KernelSource};
    KernelDescriptor::new(
        KernelName::new("vector_add").expect("valid name"),
        EntryName::new("vector_add_kernel").expect("valid entry"),
        ComputeArch::Sm80,
        KernelSource::PtxText(PtxModule::from_text(VALID_PTX).expect("valid PTX")),
    )
}

fn launch_1d(blocks: u32) -> LaunchConfig {
    LaunchConfig {
        grid: Dim3::new(blocks, 1, 1),
        block: Dim3::new(256, 1, 1),
        shared_mem_bytes: 0,
    }
}

#[test]
fn unknown_arch_is_rejected() {
    assert!(matches!(
        ComputeArch::parse("sm_70"),
        Err(CudaError::UnknownArch { .. })
    ));
    assert!(matches!(
        ComputeArch::parse(""),
        Err(CudaError::UnknownArch { .. })
    ));
    assert!(matches!(
        ComputeArch::parse("blackwell"),
        Err(CudaError::UnknownArch { .. })
    ));
}

#[test]
fn ptx_version_out_of_range_is_rejected() {
    assert_eq!(
        PtxVersion::parse("99.0"),
        Err(CudaError::PtxVersionOutOfRange {
            major: 99,
            minor: 0
        })
    );
    assert_eq!(
        PtxVersion::parse("6.9"),
        Err(CudaError::PtxVersionOutOfRange { major: 6, minor: 9 })
    );
    assert!(matches!(
        PtxVersion::parse("eight"),
        Err(CudaError::PtxVersionMalformed { .. })
    ));
}

#[test]
fn ptx_missing_directives_are_rejected() {
    assert_eq!(
        PtxModule::from_text(NO_VERSION_PTX),
        Err(CudaError::PtxMissingVersion)
    );
    assert_eq!(
        PtxModule::from_text(NO_TARGET_PTX),
        Err(CudaError::PtxMissingTarget)
    );
}

#[test]
fn ptx_oversize_is_rejected_before_parsing() {
    let big = "x".repeat(PTX_BYTES_MAX + 1);
    assert_eq!(
        PtxModule::from_text(&big),
        Err(CudaError::PtxTooLarge {
            bytes: PTX_BYTES_MAX + 1
        })
    );
}

#[test]
fn invalid_entry_name_is_rejected() {
    let module = PtxModule::from_text(VALID_PTX).expect("valid PTX");
    assert!(matches!(
        module.has_entry("has space"),
        Err(CudaError::InvalidName { .. })
    ));
    assert!(matches!(
        EntryName::new("9lives"),
        Err(CudaError::InvalidName { .. })
    ));
    assert!(matches!(
        KernelName::new("has space"),
        Err(CudaError::InvalidName { .. })
    ));
}

#[test]
fn zero_block_dim_is_rejected() {
    let config = LaunchConfig {
        grid: Dim3::new(1, 1, 1),
        block: Dim3::new(0, 1, 1),
        shared_mem_bytes: 0,
    };
    assert_eq!(
        config.validate(),
        Err(CudaError::ZeroDim { what: "block.x" })
    );
}

#[test]
fn block_over_1024_threads_is_rejected() {
    let config = LaunchConfig {
        grid: Dim3::new(1, 1, 1),
        block: Dim3::new(33, 32, 1),
        shared_mem_bytes: 0,
    };
    assert_eq!(
        config.validate(),
        Err(CudaError::BlockThreadsTooMany { threads: 1056 })
    );
}

#[test]
fn block_thread_product_overflow_is_checked_not_panicking() {
    let config = LaunchConfig {
        grid: Dim3::new(1, 1, 1),
        block: Dim3::new(u32::MAX, u32::MAX, u32::MAX),
        shared_mem_bytes: 0,
    };
    assert_eq!(config.validate(), Err(CudaError::BlockThreadsOverflow));
}

#[test]
fn shared_mem_over_bound_is_rejected() {
    let config = LaunchConfig {
        grid: Dim3::new(1, 1, 1),
        block: Dim3::new(1, 1, 1),
        shared_mem_bytes: 48 * 1024 + 1,
    };
    assert_eq!(
        config.validate(),
        Err(CudaError::SharedMemTooLarge {
            bytes: 48 * 1024 + 1
        })
    );
}

#[test]
fn grid_x_over_limit_is_rejected() {
    let config = LaunchConfig {
        grid: Dim3::new(2_147_483_648, 1, 1),
        block: Dim3::new(1, 1, 1),
        shared_mem_bytes: 0,
    };
    assert!(matches!(
        config.validate(),
        Err(CudaError::GridDimTooLarge { .. })
    ));
}

#[test]
fn simulator_rejects_bad_alloc_sizes() {
    let mut backend = SimulatedBackend::new();
    assert_eq!(
        backend.alloc(0),
        Err(CudaError::InvalidAllocSize { bytes: 0 })
    );
    // Fill the whole budget, then one more byte must fail.
    let full = backend
        .alloc(DEVICE_MEMORY_BYTES_MAX)
        .expect("exact budget fits");
    assert_eq!(
        backend.alloc(1),
        Err(CudaError::OutOfMemory {
            requested: 1,
            available: 0
        })
    );
    backend.free(full).expect("free");
}

#[test]
fn simulator_double_free_is_rejected() {
    let mut backend = SimulatedBackend::new();
    let alloc = backend.alloc(64).expect("alloc");
    backend.free(alloc).expect("first free");
    assert_eq!(
        backend.free(alloc),
        Err(CudaError::DoubleFree { id: alloc.id })
    );
}

#[test]
fn simulator_copy_size_mismatch_is_rejected() {
    let mut backend = SimulatedBackend::new();
    let alloc = backend.alloc(16).expect("alloc");
    assert_eq!(
        backend.write(&alloc, &[0u8; 8]),
        Err(CudaError::CopySizeMismatch {
            alloc_bytes: 16,
            copy_bytes: 8
        })
    );
    let mut out = vec![0u8; 32];
    assert_eq!(
        backend.read(&alloc, &mut out),
        Err(CudaError::CopySizeMismatch {
            alloc_bytes: 16,
            copy_bytes: 32
        })
    );
    // A forged handle is unknown, not a size mismatch.
    let forged = phlow_compute_cuda::DeviceAlloc { id: 999, bytes: 16 };
    assert_eq!(
        backend.write(&forged, &[0u8; 16]),
        Err(CudaError::UnknownAlloc { id: 999 })
    );
}

#[test]
fn simulator_launch_of_unknown_kernel_is_rejected() {
    let mut backend = SimulatedBackend::new();
    let args = LaunchArgs::new(vec![], vec![]).expect("valid args");
    assert!(matches!(
        backend.launch(&descriptor(), &launch_1d(1), &args),
        Err(CudaError::UnknownKernel { .. })
    ));
}

#[test]
fn simulator_behavior_failure_restores_buffers_and_reports() {
    fn failing(_buffers: &mut [Vec<u8>], _launch: &ValidatedLaunch) -> Result<(), BehaviorError> {
        Err(BehaviorError {
            detail: "injected failure",
        })
    }
    let mut backend = SimulatedBackend::new();
    backend.register_behavior("vector_add", failing);
    let alloc = backend.alloc(8).expect("alloc");
    backend.write(&alloc, &[7u8; 8]).expect("write");
    let args = LaunchArgs::new(vec![alloc.id], vec![]).expect("valid args");
    let err = backend
        .launch(&descriptor(), &launch_1d(1), &args)
        .expect_err("behavior fails");
    assert_eq!(
        err,
        CudaError::KernelBehaviorFailed {
            name: "vector_add".to_string(),
            detail: "injected failure",
        }
    );
    // The buffer survived the failed launch: restore ran before the error.
    let mut out = vec![0u8; 8];
    backend.read(&alloc, &mut out).expect("buffer restored");
    assert_eq!(out, vec![7u8; 8]);
    backend.free(alloc).expect("free still works");
}

#[test]
fn simulator_launch_restores_buffers_on_unknown_arg() {
    let mut backend = SimulatedBackend::new();
    backend.register_behavior("vector_add", |_, _| Ok(()));
    let good = backend.alloc(8).expect("alloc");
    backend.write(&good, &[3u8; 8]).expect("write");
    // Second arg is unknown: buffers removed so far must be restored.
    let args = LaunchArgs::new(vec![good.id, 4242], vec![]).expect("valid args");
    assert_eq!(
        backend.launch(&descriptor(), &launch_1d(1), &args),
        Err(CudaError::UnknownAlloc { id: 4242 })
    );
    let mut out = vec![0u8; 8];
    backend.read(&good, &mut out).expect("buffer restored");
    assert_eq!(out, vec![3u8; 8]);
}

#[test]
fn descriptor_rejects_unknown_key() {
    let text = "name = k\nentry = e\narch = sm_80\nsource = ptx:x.ptx\nmystery = 1\n";
    assert!(matches!(
        parse_descriptor_file(text),
        Err(CudaError::UnknownDescriptorKey { line: 5, .. })
    ));
}

#[test]
fn descriptor_rejects_duplicate_key() {
    let text = "name = k\nentry = e\narch = sm_80\nsource = ptx:x.ptx\nname = k2\n";
    assert_eq!(
        parse_descriptor_file(text),
        Err(CudaError::DuplicateDescriptorKey { key: "name" })
    );
}

#[test]
fn descriptor_rejects_missing_key() {
    let text = "name = k\nentry = e\narch = sm_80\n";
    assert_eq!(
        parse_descriptor_file(text),
        Err(CudaError::MissingDescriptorKey { key: "source" })
    );
}

#[test]
fn policy_rejects_invalid_fields() {
    assert!(matches!(
        KernelPolicy::new(0, 4, false),
        Err(CudaError::InvalidPolicy { .. })
    ));
    assert!(matches!(
        KernelPolicy::new(65, 4, false),
        Err(CudaError::InvalidPolicy { .. })
    ));
    assert!(matches!(
        KernelPolicy::new(16, 0, false),
        Err(CudaError::InvalidPolicy { .. })
    ));
    assert!(matches!(
        KernelPolicy::new(16, 17, false),
        Err(CudaError::InvalidPolicy { .. })
    ));
    // The same invalid config fails the same way every time.
    assert_eq!(
        KernelPolicy::new(0, 4, false).err(),
        KernelPolicy::new(0, 4, false).err()
    );
}

#[test]
fn launch_rejects_too_many_args() {
    let ptrs: Vec<u64> = (0..17).collect();
    assert!(matches!(
        LaunchArgs::new(ptrs, vec![]),
        Err(CudaError::TooManyArgs { .. })
    ));
    let words: Vec<u64> = (0..65).collect();
    assert!(matches!(
        LaunchArgs::new(vec![], words),
        Err(CudaError::TooManyArgs { .. })
    ));
}
