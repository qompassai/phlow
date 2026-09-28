# phlow-mojo-kernels design decisions

Lane `lane/mojo`, 2026-09-28. Adaptation of Mojo GPU-kernel concepts into
Tiger Style Rust. Concepts are re-expressed, never ported: no Mojo code was
translated, no Mojo was compiled, and no GPU is touched. All upstream facts
below were verified against primary sources on 2026-09-28; secondary
observations are labeled as such.

## 1. Upstream sources

- **modular/modular** (https://github.com/modular/modular): the open-source
  Modular Platform repo. Hosts the Mojo compiler (`/Mojo`), the Mojo
  standard library (`/Mojo/stdlib`), and the MAX accelerator library
  (`/max/kernels`). Licensed Apache 2.0 with LLVM Exceptions. Mojo went
  open source in August 2026; Modular 26.5 shipped "Mojo 1.0".
- **Mojo docs** (https://docs.modular.com/mojo, v1.1.0): the documentation
  index, including the GPU programming track and the Mojo manual.
- **GPU intro tutorial** (https://docs.modular.com/mojo/manual/gpu/intro-tutorial.md,
  fetched verbatim): the primary source for the launch model —
  - a GPU *kernel* is a function that runs on a GPU across thousands or
    millions of threads; a *grid* is the top-level organization of *thread
    blocks*, each block holding individual threads;
  - `DeviceContext` (from `max.gpu.host`) is the logical GPU device: it
    allocates device memory, copies host↔device, and compiles/runs kernels;
    `has_accelerator()` reports GPU availability;
  - launch: `ctx.enqueue_function[kernel](grid_dim=..., block_dim=...)` —
    the kernel is a *compile-time parameter*, grid/block dims are keyword
    arguments, each 1D/2D/3D (1D for vectors, 2D for matrices, 3D for video
    frames);
  - `enqueue_function`/`compile_function` type-check kernel arguments at
    *compile time*; a mismatch is a compile error, not a runtime failure;
  - launches are *asynchronous*: the context owns a stream of queued
    operations, and `synchronize()` blocks until the device drains it;
  - threads in one block share shared memory and can synchronize; blocks
    cannot communicate with each other.
- **Driver-limit behavior** (secondary, empirical): a workshop report on the
  official tutorial notes NVIDIA hardware rejects more than 1024 threads
  per block with a raw `CUDA_ERROR_INVALID_VALUE` from the driver, and
  explicitly wishes Mojo did parameter checking before the low-level API.
  This is the direct motivation for host-side launch validation here: the
  checks Mojo leaves to the driver become typed Rust errors *before* any
  plan is committed.

## 2. Crate placement: `kernels/mojo` (top-level, workspace member)

Not under `crates/`: the fifteen crates there form phlow's shared safe
runtime. Kernel contracts are an experimental extension lane, not core
runtime. The top-level `kernels/` directory mirrors upstream vocabulary
(MAX's accelerator library lives at `max/kernels` in modular/modular) and
keeps the experimental surface out of the audited core tree. Added to the
root workspace `members` alongside the existing crates.

## 3. What was adapted (concepts only)

| Mojo concept (source) | Rust adaptation in this crate |
|---|---|
| Kernel = a function passed as a compile-time parameter to `enqueue_function` | `KernelName` (validated) + `KernelVersion`, resolved via `KernelRegistry` |
| `grid_dim` / `block_dim` launch geometry, 1D/2D/3D | `Dim3` with `one_d` / `two_d` / `new`; range-checked per axis |
| Compile-time kernel argument type-checking | Descriptor `arg_count_max` / `payload_bytes_max` checked at `validate` time, before planning |
| Driver rejects oversized launches with raw errors | `LaunchConfig::validate`: checked arithmetic, per-axis limits, thread/shared-memory/total-thread budgets → typed `KernelError` |
| `enqueue_function` (async) + `synchronize()` | `KernelExecutor::launch` (enqueue) + `synchronize` (drain); `LaunchReceipt` as the acceptance proof |
| Shared memory per block | Descriptor declares required bytes; launch must provide ≥ required and ≤ device max |

## 4. Deliberate differences from Mojo

- **Host-side validation is stricter than Mojo's.** Mojo type-checks
  arguments at compile time but leaves dimension sanity to the driver
  (raw `CUDA_ERROR_INVALID_VALUE`). This crate rejects bad geometry with
  typed errors before planning — the check the workshop author wished
  existed.
- **`ValidatedLaunch` makes invalid launches unrepresentable.** Only
  `LaunchConfig::validate` constructs it; the planner and executor take
  `ValidatedLaunch`, so unvalidated launches cannot reach device code.
  Mojo has no equivalent host-side newtype: the guarantee here is
  stronger by construction.
- **Budgets are explicit.** `DeviceLimits::generic` (1024 threads/block,
  48 KiB shared/block, 2³¹ total threads/launch) and `LaunchBudget`
  (launch count, aggregate payload bytes) are named, operator-visible
  constants. Mojo leaves these to hardware/driver defaults.
- **The executor is a boundary, not a backend.** `KernelExecutor` is what
  a real device backend implements. The `simulated` feature's
  `SimulatedExecutor` re-derives the thread product with checked
  arithmetic and returns a deterministic FNV-1a receipt — a test double
  for the contract, never a device.
- **No autotuning, no layouts, no memory model.** Mojo's `LayoutTensor`,
  autotuning, and device memory management are out of scope: this crate
  covers launch *contracts*, not execution.

## 5. Bounds

`KERNEL_NAME_CHARS_MAX` (128), `KERNELS_MAX` (256), `ARG_COUNT_MAX` (32),
`PAYLOAD_BYTES_MAX` (16 MiB), `THREADS_PER_BLOCK_MAX` (1024),
`SHARED_MEMORY_BYTES_PER_BLOCK_MAX` (48 KiB), `GRID_DIM_MAX` (65535),
`TOTAL_THREADS_PER_LAUNCH_MAX` (2³¹), `LAUNCHES_MAX` (1024),
`BUDGET_PAYLOAD_BYTES_MAX` (1 GiB), `REASON_CHARS_MAX` (256). The 1024
threads/block figure is hardware-verified (see §1); the rest are
conservative host-side budgets documented at their definitions.

## 6. Test inventory

22 tests in `tests/kernels.rs`: **11 validation / 11 adversarial** (50/50).

Validation: register/lookup roundtrip; name at exact char limit; minimal
1D launch; 3D launch (2×2×1 grid of 4×4×2 blocks = 128 threads, mirroring
the Mojo docs example); threads-per-block at exactly 1024; shared memory
at exactly 48 KiB; total-thread computation (30×128); arg count at exact
limit; payload at exact limit; planner commits budget once; simulated
executor receipt matches plan with deterministic checksum.

Adversarial: empty name; 129-char name; invalid char (`-`, space);
duplicate registration keeps the original; unknown kernel; zero block
dim; 1025 threads/block; dimension-product overflow (u64, typed error not
panic); shared memory over 48 KiB; total threads over 2³¹; payload over
the kernel's 1 KiB limit (plus descriptor over the 16 MiB global cap).
