# phlow-compute-cuda: placement and adaptation decisions

## Why this crate exists

`phlow-compute` models tile ownership; it says nothing about kernels.
`phlow-compute-cuda` is the kernel-target model: what a GPU kernel is
(PTX text or CUBIN file plus metadata), what a valid launch looks like,
and how host code talks to a device through a bounded trait. It is
CPU-only — the "device" is a deterministic simulator — so the whole
contract is testable without CUDA hardware or the CUDA toolkit.

## What was adapted (from NVlabs cuda-oxide, Apache-2.0 — concepts only)

- **Kernel policies.** Compile-time specialization choices (tile rows,
  unroll factor, tensor-core use) fixed at build time, with a
  deterministic variant key for caching. (`policy.rs`)
- **Host runtime as a trait.** Kernel submission goes through a bounded
  `CudaBackend` trait; the simulator is one implementor, a real driver
  binding would be another. (`backend.rs`)
- **Structural PTX validation.** `.version` / `.target` presence checks
  and entry discovery, without compiling anything. (`ptx.rs`)
- **Launch limits.** Block threads ≤ 1024, dynamic shared memory ≤ 48 KiB,
  grid axes ≤ 2³¹−1, all checked with checked arithmetic. (`launch.rs`)

No cuda-oxide code was ported. The custom rustc codegen backend, MIR →
Pliron → LLVM → PTX lowering, and the `#[kernel]` proc-macro machinery
were studied as concepts and deliberately left out (see below).

## Deliberately excluded

- **The compiler.** There is no PTX codegen here — no rustc backend, no
  MIR lowering, no `#[kernel]` macro. Kernels arrive as PTX text or CUBIN
  paths; this crate validates their shape and metadata only.
- **A real driver binding.** `SimulatedBackend` is a deterministic CPU
  double with bounded memory. A future `phlow-compute-cuda-driver` crate
  could implement `CudaBackend` against the real CUDA driver API; nothing
  in this crate's design precludes it, and nothing here depends on it.
- **Async launches and streams.** The simulator is synchronous. Stream /
  event semantics are a driver-crate concern.
- **CUBIN parsing.** CUBIN sources are accepted as file paths with
  bounded metadata; the binary format is never parsed.

## License basis

cuda-oxide is Apache-2.0. Only its concepts were adapted; no upstream
code, text, or API surface was copied. This crate is original work.
