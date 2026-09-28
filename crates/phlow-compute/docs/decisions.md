# phlow-compute: placement and adaptation decisions

## Why this crate exists

Tile-based GPU compute needs a memory-safety story that survives the trip
from host code to device kernels. `phlow-compute` holds the host-side half
of that story — shapes, partitions, and launch-ownership discipline — as
pure safe Rust with no GPU dependency, so it can be tested and reasoned
about on any machine.

## What was adapted (from NVlabs cutile-rs, Apache-2.0 — concepts only)

- Mutable outputs are partitioned into disjoint pieces; read-only inputs
  are shared. (`ownership.rs`)
- A tensor is covered by an exact grid of tiles; every tile has a disjoint
  origin. (`partition.rs`, `tile.rs`)
- Launch arguments separate exclusive outputs from shared inputs, and
  aliasing a handle as both is rejected. (`executor.rs`)

No cutile-rs code was ported. The PTX/CUBIN JIT path, the CUDA Tile IR
backend, async/graph-replay execution, and external-kernel
interoperability were studied as concepts and deliberately left out (see
below).

## Deliberately excluded

- **Device execution.** This crate never touches a GPU. Execution lives in
  `phlow-compute-cuda` (kernel model) and `phlow-gpu-worker` (dispatch).
- **Partial / halo tiles.** Partitions must divide the tensor exactly;
  edge tiles with bounds checks are a real cutile-rs concern but add an
  overlap-safety burden this crate's exact-partition invariant refuses.
- **Async and graph replay.** The deterministic CPU executor is
  synchronous by design; replay semantics belong to a future executor
  crate, not to the ownership model.
- **JIT compilation.** No PTX is generated or compiled here; structural
  PTX validation lives in `phlow-compute-cuda`.

## License basis

cutile-rs is Apache-2.0. Only its concepts were adapted; no upstream code,
text, or API surface was copied. This crate is original work.
