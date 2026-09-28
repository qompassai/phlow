//! Validation tests: well-formed descriptors, configs, policies, and
//! simulator runs behave as the contracts promise.

use phlow_compute_cuda::{
    BLOCK_THREADS_MAX, BehaviorError, ComputeArch, CudaBackend, CudaError, DEVICE_MEMORY_BYTES_MAX,
    DeviceAlloc, Dim3, EntryName, GRID_DIM_AXIS_MAX, KernelBehavior, KernelDescriptor, KernelName,
    KernelPath, KernelPolicy, KernelSource, LaunchArgs, LaunchConfig, PTX_BYTES_MAX, PtxModule,
    PtxVersion, SHARED_MEM_BYTES_MAX, SimulatedBackend, Specialization, ValidatedLaunch,
    parse_descriptor_file,
};

const VALID_PTX: &str = include_str!("../../../kernels/ptx/valid_add.ptx");
const VECTOR_ADD_DESC: &str = include_str!("../../../kernels/cuda/vector_add.desc");
const RMS_NORM_DESC: &str = include_str!("../../../kernels/cuda/rms_norm.desc");

fn vector_add_descriptor() -> KernelDescriptor {
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

/// Element-wise f32 vector add: buffers = [a, b, out].
fn vector_add_behavior(
    buffers: &mut [Vec<u8>],
    _launch: &ValidatedLaunch,
) -> Result<(), BehaviorError> {
    if buffers.len() != 3 {
        return Err(BehaviorError {
            detail: "vector_add needs three buffers",
        });
    }
    let elements = buffers[2].len() / 4;
    for index in 0..elements {
        let read = |buffer: &[u8]| {
            f32::from_le_bytes([
                buffer[4 * index],
                buffer[4 * index + 1],
                buffer[4 * index + 2],
                buffer[4 * index + 3],
            ])
        };
        let sum = read(&buffers[0]) + read(&buffers[1]);
        buffers[2][4 * index..4 * index + 4].copy_from_slice(&sum.to_le_bytes());
    }
    Ok(())
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[test]
fn arch_parses_known_names() {
    assert_eq!(ComputeArch::parse("sm_80"), Ok(ComputeArch::Sm80));
    assert_eq!(ComputeArch::parse("compute_80"), Ok(ComputeArch::Sm80));
    assert_eq!(ComputeArch::parse("SM_90"), Ok(ComputeArch::Sm90));
    assert_eq!(ComputeArch::parse("100"), Ok(ComputeArch::Sm100));
    assert_eq!(ComputeArch::Sm80.name(), "sm_80");
    assert_eq!(ComputeArch::Sm100.name(), "sm_100");
}

#[test]
fn ptx_version_parses_and_orders() {
    let v80 = PtxVersion::parse("8.0").expect("valid version");
    let v85 = PtxVersion::parse("8.5").expect("valid version");
    assert!(v80 < v85);
    assert!(PtxVersion::MIN <= v80 && v85 <= PtxVersion::MAX);
    assert_eq!(v80, PtxVersion { major: 8, minor: 0 });
}

#[test]
fn valid_ptx_module_is_accepted() {
    let module = PtxModule::from_text(VALID_PTX).expect("fixture is valid PTX");
    assert_eq!(module.version(), PtxVersion { major: 8, minor: 0 });
    assert_eq!(module.target(), "sm_80");
    assert!(module.text().len() < PTX_BYTES_MAX);
}

#[test]
fn ptx_entry_discovery_finds_the_kernel() {
    let module = PtxModule::from_text(VALID_PTX).expect("valid PTX");
    assert!(module.has_entry("vector_add_kernel").expect("valid name"));
    assert!(!module.has_entry("missing_kernel").expect("valid name"));
    assert!(
        module
            .entry_names()
            .contains(&"vector_add_kernel".to_string())
    );
}

#[test]
fn ptx_keeps_the_first_version_directive() {
    let text = ".version 8.0\n.target sm_80\n.version 9.9\n";
    let module = PtxModule::from_text(text).expect("valid PTX");
    assert_eq!(module.version(), PtxVersion { major: 8, minor: 0 });
}

#[test]
fn entry_name_allows_ptx_sigils() {
    assert!(EntryName::new("kernel$v1.func").is_ok());
    assert!(EntryName::new(".entry").is_ok());
}

#[test]
fn kernel_descriptor_assembles() {
    let descriptor = vector_add_descriptor();
    assert_eq!(descriptor.name.as_str(), "vector_add");
    assert_eq!(descriptor.entry.as_str(), "vector_add_kernel");
    assert_eq!(descriptor.arch, ComputeArch::Sm80);
    assert!(matches!(descriptor.source, KernelSource::PtxText(_)));
}

#[test]
fn launch_config_1d_is_valid() {
    let validated: ValidatedLaunch = launch_1d(8).validate().expect("valid config");
    assert_eq!(validated.block_threads, 256);
    assert_eq!(validated.total_threads, 8 * 256);
}

#[test]
fn launch_config_max_block_is_valid() {
    let config = LaunchConfig {
        grid: Dim3::new(1, 1, 1),
        block: Dim3::new(32, 32, 1),
        shared_mem_bytes: 0,
    };
    let validated = config.validate().expect("1024 threads is the max");
    assert_eq!(validated.block_threads, BLOCK_THREADS_MAX);
}

#[test]
fn launch_config_shared_mem_boundary_is_valid() {
    let config = LaunchConfig {
        grid: Dim3::new(1, 1, 1),
        block: Dim3::new(1, 1, 1),
        shared_mem_bytes: SHARED_MEM_BYTES_MAX,
    };
    assert!(config.validate().is_ok());
    let zero = LaunchConfig {
        shared_mem_bytes: 0,
        ..config
    };
    assert!(zero.validate().is_ok());
}

#[test]
fn launch_config_grid_x_max_is_valid() {
    let config = LaunchConfig {
        grid: Dim3::new(GRID_DIM_AXIS_MAX, 1, 1),
        block: Dim3::new(1, 1, 1),
        shared_mem_bytes: 0,
    };
    let validated = config.validate().expect("2^31 - 1 is the max");
    assert_eq!(validated.total_threads, u64::from(GRID_DIM_AXIS_MAX));
}

#[test]
fn descriptor_file_parses_vector_add() {
    let descriptor = parse_descriptor_file(VECTOR_ADD_DESC).expect("valid descriptor");
    assert_eq!(descriptor.name.as_str(), "vector_add");
    assert_eq!(descriptor.entry.as_str(), "vector_add_kernel");
    assert_eq!(descriptor.arch, ComputeArch::Sm80);
    assert!(matches!(
        descriptor.source,
        KernelSource::PtxFile(ref path) if path.as_str() == "kernels/ptx/valid_add.ptx"
    ));
}

#[test]
fn descriptor_file_parses_cubin_source() {
    let descriptor = parse_descriptor_file(RMS_NORM_DESC).expect("valid descriptor");
    assert_eq!(descriptor.arch, ComputeArch::Sm90);
    assert!(matches!(descriptor.source, KernelSource::CubinFile(_)));
}

#[test]
fn policy_variant_key_is_deterministic() {
    let policy = KernelPolicy::new(16, 4, true).expect("valid policy");
    let first = Specialization {
        descriptor: vector_add_descriptor(),
        policy,
    };
    let second = Specialization {
        descriptor: vector_add_descriptor(),
        policy: KernelPolicy::new(16, 4, true).expect("valid policy"),
    };
    assert_eq!(first.variant_key(), second.variant_key());
    let different = Specialization {
        descriptor: vector_add_descriptor(),
        policy: KernelPolicy::new(16, 4, false).expect("valid policy"),
    };
    assert_ne!(first.variant_key(), different.variant_key());
    assert!(first.variant_key().contains("tile_rows=16"));
}

#[test]
fn simulator_alloc_write_read_roundtrip() {
    let mut backend = SimulatedBackend::new();
    let alloc: DeviceAlloc = backend.alloc(16).expect("fits the budget");
    assert_eq!(alloc.bytes, 16);
    let data = f32_bytes(&[1.0, 2.0, 3.0, 4.0]);
    backend.write(&alloc, &data).expect("exact-size write");
    let mut out = vec![0u8; 16];
    backend.read(&alloc, &mut out).expect("exact-size read");
    assert_eq!(out, data);
    backend.free(alloc).expect("free once");
    assert_eq!(backend.used_bytes(), 0);
}

#[test]
fn simulator_free_reclaims_budget() {
    let mut backend = SimulatedBackend::new();
    let alloc = backend.alloc(1024).expect("fits");
    backend.free(alloc).expect("free");
    // The full budget is available again.
    let full = backend
        .alloc(DEVICE_MEMORY_BYTES_MAX)
        .expect("full budget fits after free");
    backend.free(full).expect("free");
}

#[test]
fn simulator_launch_runs_vector_add() {
    let mut backend = SimulatedBackend::new();
    backend.register_behavior("vector_add", vector_add_behavior as KernelBehavior);
    let a = backend.alloc(16).expect("alloc a");
    let b = backend.alloc(16).expect("alloc b");
    let out = backend.alloc(16).expect("alloc out");
    backend
        .write(&a, &f32_bytes(&[1.0, 2.0, 3.0, 4.0]))
        .expect("write a");
    backend
        .write(&b, &f32_bytes(&[10.0, 20.0, 30.0, 40.0]))
        .expect("write b");
    let args = LaunchArgs::new(vec![a.id, b.id, out.id], vec![4]).expect("valid args");
    let receipt = backend
        .launch(&vector_add_descriptor(), &launch_1d(1), &args)
        .expect("launch runs");
    assert_eq!(receipt.id, 1);
    backend.synchronize().expect("sync is a no-op");
    let mut result = vec![0u8; 16];
    backend.read(&out, &mut result).expect("read out");
    assert_eq!(result, f32_bytes(&[11.0, 22.0, 33.0, 44.0]));
}

#[test]
fn simulator_receipts_increase_per_launch() {
    fn noop(_buffers: &mut [Vec<u8>], _launch: &ValidatedLaunch) -> Result<(), BehaviorError> {
        Ok(())
    }
    let mut backend = SimulatedBackend::new();
    backend.register_behavior("noop", noop);
    let descriptor = phlow_compute_cuda::KernelDescriptor::new(
        KernelName::new("noop").expect("valid name"),
        EntryName::new("noop_kernel").expect("valid entry"),
        ComputeArch::Sm80,
        KernelSource::PtxText(PtxModule::from_text(VALID_PTX).expect("valid PTX")),
    );
    let args = LaunchArgs::new(vec![], vec![]).expect("valid args");
    let first = backend
        .launch(&descriptor, &launch_1d(1), &args)
        .expect("first launch");
    let second = backend
        .launch(&descriptor, &launch_1d(1), &args)
        .expect("second launch");
    assert!(second.id > first.id);
}

#[test]
fn policy_boundary_values_are_valid() {
    let policy = KernelPolicy::new(64, 16, true).expect("max values valid");
    assert_eq!(policy.tile_rows(), 64);
    assert_eq!(policy.unroll_factor(), 16);
    assert!(policy.use_tensor_cores());
    assert!(KernelPolicy::new(1, 1, false).is_ok());
}

#[test]
fn kernel_path_accepts_bounded_paths() {
    assert!(KernelPath::new("kernels/ptx/valid_add.ptx").is_ok());
    assert!(matches!(
        KernelPath::new(""),
        Err(CudaError::InvalidPath { .. })
    ));
}

#[test]
fn launch_args_accept_max_counts() {
    use phlow_compute_cuda::{LAUNCH_ARG_PTRS_MAX, LAUNCH_ARG_WORDS_MAX};
    let ptrs: Vec<u64> = (0..LAUNCH_ARG_PTRS_MAX as u64).collect();
    let words: Vec<u64> = (0..LAUNCH_ARG_WORDS_MAX as u64).collect();
    let args = LaunchArgs::new(ptrs, words).expect("max counts are valid");
    assert_eq!(args.device_ptrs.len(), LAUNCH_ARG_PTRS_MAX);
    assert_eq!(args.scalar_words.len(), LAUNCH_ARG_WORDS_MAX);
}
