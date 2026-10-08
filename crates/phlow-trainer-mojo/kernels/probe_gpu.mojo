# ===----------------------------------------------------------------------=== #
# phlow-trainer-mojo Phase A probe: prove a Mojo kernel compiles and runs
# on primo's RTX 4070 through the pixi-pinned MAX 26.5 / Mojo 1.0 toolchain.
#
# Contract: adds two f32 vectors element-wise on the device and verifies
# every element on the host. Work is bounded by VECTOR_SIZE (compile-time);
# the device context owns all buffers, which are released when their last
# use ends (Mojo ASAP destruction). Prints the device name and a PASS/FAIL
# line; exits nonzero on any mismatch via `raises`.
# ===----------------------------------------------------------------------=== #

from std.gpu import block_dim, block_idx, thread_idx
from max.gpu.host import DeviceContext
from layout import TileTensor, row_major
from std.math import ceildiv
from std.sys import has_accelerator

comptime float_dtype = DType.float32
comptime vector_size = 4096
comptime block_size = 256
comptime layout = row_major(vector_size)


def vector_add(
    lhs: TileTensor[float_dtype, type_of(layout), ImmutAnyOrigin],
    rhs: TileTensor[float_dtype, type_of(layout), ImmutAnyOrigin],
    result: TileTensor[float_dtype, type_of(layout), MutAnyOrigin],
):
    """Element-wise sum: result[i] = lhs[i] + rhs[i] for i < vector_size (one thread per element)."""
    var tid = block_idx.x * block_dim.x + thread_idx.x
    if tid < vector_size:
        result[tid] = lhs[tid] + rhs[tid]


def main() raises:
    comptime if not has_accelerator():
        print("PROBE FAIL: no compatible GPU found")
        return

    var ctx = DeviceContext()
    print("device:", ctx.name())

    var lhs_host = ctx.enqueue_create_host_buffer[float_dtype](vector_size)
    var rhs_host = ctx.enqueue_create_host_buffer[float_dtype](vector_size)
    ctx.synchronize()
    for i in range(vector_size):
        lhs_host[i] = Float32(i)
        rhs_host[i] = Float32(i) * Float32(0.5)

    var lhs_dev = ctx.enqueue_create_buffer[float_dtype](vector_size)
    var rhs_dev = ctx.enqueue_create_buffer[float_dtype](vector_size)
    var out_dev = ctx.enqueue_create_buffer[float_dtype](vector_size)
    ctx.enqueue_copy(dst_buf=lhs_dev, src_buf=lhs_host)
    ctx.enqueue_copy(dst_buf=rhs_dev, src_buf=rhs_host)

    var lhs_tensor = TileTensor(lhs_dev, layout)
    var rhs_tensor = TileTensor(rhs_dev, layout)
    var out_tensor = TileTensor(out_dev, layout)

    comptime num_blocks = ceildiv(vector_size, block_size)
    ctx.enqueue_function[vector_add](
        lhs_tensor, rhs_tensor, out_tensor,
        grid_dim=num_blocks, block_dim=block_size,
    )

    var out_host = ctx.enqueue_create_host_buffer[float_dtype](vector_size)
    ctx.enqueue_copy(dst_buf=out_host, src_buf=out_dev)
    ctx.synchronize()

    var mismatches = 0
    for i in range(vector_size):
        var expected = Float32(i) * Float32(1.5)
        if abs(out_host[i] - expected) > Float32(1e-5):
            mismatches += 1
    if mismatches != 0:
        print("PROBE FAIL: mismatches =", mismatches)
        raise Error("probe verification failed")
    print("PROBE PASS: vector_add", vector_size, "elements verified on host")
