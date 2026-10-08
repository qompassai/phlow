# ===----------------------------------------------------------------------=== #
# phlow-trainer-mojo scoring kernels (Mojo 1.0 / MAX 26.5, pixi-pinned).
#
# Purpose: the parity-critical scoring math of the phlow-trainlab trainer
# contract, as GPU kernels behind a small C ABI:
#
#   * per-token logprob of target tokens given rows of vocab logits
#     (log-softmax reduction over the vocabulary, f32 in / f32 out),
#   * RLOO leave-one-out advantages for reward groups,
#   * a bench entry that re-launches the logprob kernel on resident data.
#
# Contract (all exports):
#   * Inputs are borrowed host buffers owned by the caller; exports copy
#     in, compute, copy out, and never retain a pointer.
#   * Every shape is validated against the named bounds below BEFORE any
#     allocation or device work; violations return ERR_ARG.
#   * Return value is a status code (OK on success). Mojo exceptions never
#     cross the ABI: device failures are caught and returned as ERR_DEVICE.
#   * Numerics: logits are consumed as f32 exactly as the reference
#     backend's `log_softmax(pred.float(), dim=-1)` sees them; the row
#     reduction is max-shifted (log-sum-exp) so no intermediate overflows.
#     Rewards/advantages are f32 on device; trainlab's reference values
#     are f64, so advantage agreement is expected at f32 precision
#     (~1e-7 relative), and the check harness reports the actual diff.
#   * Cancellation: launches are stream-serialized on one DeviceContext
#     created per call; there is no cross-call state to cancel.
# ===----------------------------------------------------------------------=== #

from max.gpu import barrier
from max.gpu.host import DeviceContext, HostBuffer
from max.gpu.memory import AddressSpace
from std.gpu import block_dim, block_idx, thread_idx
from std.math import exp, log
from std.memory import stack_allocation, unsafe_memcpy
from std.sys import has_accelerator
from std.time import perf_counter_ns

# --- Bounds (named, with units) -------------------------------------------
comptime TOKENS_PER_LAUNCH_MAX = 65_536  # rows per logprob launch
comptime VOCAB_SIZE_MAX = 262_144  # logits width per row
comptime LOGITS_ELEMENTS_MAX = 268_435_456  # rows * vocab cap (1 GiB f32)
comptime GROUPS_PER_LAUNCH_MAX = 4_096  # reward groups per advantage launch
comptime GROUP_COMPLETIONS_MAX = 1_024  # completions per group (block size)
comptime COMPLETIONS_TOTAL_MAX = 1_048_576  # completions across all groups
comptime REPEATS_MAX = 10_000  # bench repetitions per call
comptime THREADS_PER_BLOCK = 1_024  # one block per row / per group

# --- Status codes returned across the C ABI ---------------------------------
comptime STATUS_OK = Int32(0)
comptime STATUS_ERR_ARG = Int32(1)
comptime STATUS_ERR_NO_DEVICE = Int32(2)
comptime STATUS_ERR_DEVICE = Int32(3)

comptime f32_dtype = DType.float32
comptime i32_dtype = DType.int32
# Most-negative finite f32, as a literal: logits are bounded far above it.
comptime F32_LOWEST = Float32(-3.4028234663852886e38)


# --- Kernels ---------------------------------------------------------------
def logprob_rows_kernel(
    logits: Pointer[Float32, MutAnyOrigin],
    targets: Pointer[Int32, MutAnyOrigin],
    out_logps: Pointer[Float32, MutAnyOrigin],
    vocab: Int32,
):
    """One block per row: out_logps[row] = log_softmax(logits[row])[target].

    Single-pass online log-sum-exp: each thread keeps a running
    (max, scaled sum) pair over its strided slice, rescaling the sum
    whenever the max moves; the block then tree-reduces the pairs in
    shared memory, combining with the same rescaling. One read of the
    row, no intermediate overflows, f32 throughout.
    """
    var row = Int(block_idx.x)
    var tid = Int(thread_idx.x)
    var width = Int(vocab)
    var base = row * width

    var local_max = F32_LOWEST
    var local_sum = Float32(0)
    var col = tid
    while col < width:
        var value = logits[unsafe_offset=base + col]
        if value > local_max:
            local_sum = local_sum * exp(local_max - value) + Float32(1)
            local_max = value
        else:
            local_sum += exp(value - local_max)
        col += THREADS_PER_BLOCK

    var shared_max = stack_allocation[
        THREADS_PER_BLOCK, Float32, address_space=AddressSpace.SHARED
    ]()
    var shared_sum = stack_allocation[
        THREADS_PER_BLOCK, Float32, address_space=AddressSpace.SHARED
    ]()
    shared_max[unsafe_offset=tid] = local_max
    shared_sum[unsafe_offset=tid] = local_sum
    barrier()
    var stride = THREADS_PER_BLOCK // 2
    while stride > 0:
        if tid < stride:
            var other_max = shared_max[unsafe_offset=tid + stride]
            var own_max = shared_max[unsafe_offset=tid]
            if other_max > own_max:
                shared_sum[unsafe_offset=tid] = (
                    shared_sum[unsafe_offset=tid] * exp(own_max - other_max)
                    + shared_sum[unsafe_offset=tid + stride]
                )
                shared_max[unsafe_offset=tid] = other_max
            else:
                shared_sum[unsafe_offset=tid] += shared_sum[
                    unsafe_offset=tid + stride
                ] * exp(other_max - own_max)
        barrier()
        stride //= 2

    if tid == 0:
        var target_logit = logits[unsafe_offset=base + Int(targets[unsafe_offset=row])]
        out_logps[unsafe_offset=row] = target_logit - (
            shared_max[unsafe_offset=0] + log(shared_sum[unsafe_offset=0])
        )


def rloo_advantages_kernel(
    rewards: Pointer[Float32, MutAnyOrigin],
    offsets: Pointer[Int32, MutAnyOrigin],
    out_advantages: Pointer[Float32, MutAnyOrigin],
):
    """One block per group: advantage_i = r_i - mean(rewards without i).

    Group size is validated host-side to [2, THREADS_PER_BLOCK], so one
    thread per completion suffices; each thread re-sums its group
    (<= 1024 adds), which keeps the kernel free of reduction state.
    """
    var group = Int(block_idx.x)
    var start = Int(offsets[unsafe_offset=group])
    var end = Int(offsets[unsafe_offset=group + 1])
    var size = end - start
    var tid = Int(thread_idx.x)
    if tid < size:
        var total = Float32(0)
        for j in range(start, end):
            total += rewards[unsafe_offset=j]
        var own = rewards[unsafe_offset=start + tid]
        out_advantages[unsafe_offset=start + tid] = own - (total - own) / Float32(size - 1)


# --- Host-side launch helpers (borrowed ctx, owned buffers) -----------------
def _run_logprob(
    mut ctx: DeviceContext,
    logits: Pointer[Float32, MutAnyOrigin],
    rows: Int,
    vocab: Int,
    targets: Pointer[Int32, MutAnyOrigin],
    out_host: HostBuffer[f32_dtype],
    repeats: Int,
) raises -> Int64:
    """Upload once, launch `repeats` times, download once.

    Returns elapsed nanoseconds for the launch loop (enqueue + one
    synchronize), so the bench export measures device-side work rather
    than per-call context setup. With repeats == 1 this is the plain
    scoring call. Buffers are owned by this scope and released on exit.
    """
    var elements = rows * vocab
    var logits_host = ctx.enqueue_create_host_buffer[f32_dtype](elements)
    var targets_host = ctx.enqueue_create_host_buffer[i32_dtype](rows)
    ctx.synchronize()
    unsafe_memcpy(dest=logits_host.unsafe_ptr(), src=logits, count=elements)
    unsafe_memcpy(dest=targets_host.unsafe_ptr(), src=targets, count=rows)

    var logits_dev = ctx.enqueue_create_buffer[f32_dtype](elements)
    var targets_dev = ctx.enqueue_create_buffer[i32_dtype](rows)
    var out_dev = ctx.enqueue_create_buffer[f32_dtype](rows)
    ctx.enqueue_copy(dst_buf=logits_dev, src_buf=logits_host)
    ctx.enqueue_copy(dst_buf=targets_dev, src_buf=targets_host)

    var started = perf_counter_ns()
    for _ in range(repeats):
        ctx.enqueue_function[logprob_rows_kernel](
            logits_dev.unsafe_ptr(),
            targets_dev.unsafe_ptr(),
            out_dev.unsafe_ptr(),
            Int32(vocab),
            grid_dim=rows,
            block_dim=THREADS_PER_BLOCK,
        )
    ctx.synchronize()
    var elapsed = perf_counter_ns() - started

    ctx.enqueue_copy(dst_buf=out_host, src_buf=out_dev)
    ctx.synchronize()
    return Int64(elapsed)


def _run_advantages(
    mut ctx: DeviceContext,
    rewards: Pointer[Float32, MutAnyOrigin],
    offsets: Pointer[Int32, MutAnyOrigin],
    groups: Int,
    total: Int,
    out_advantages: Pointer[Float32, MutAnyOrigin],
) raises:
    """Upload rewards + offsets, launch one block per group, download."""
    var rewards_host = ctx.enqueue_create_host_buffer[f32_dtype](total)
    var offsets_host = ctx.enqueue_create_host_buffer[i32_dtype](groups + 1)
    var out_host = ctx.enqueue_create_host_buffer[f32_dtype](total)
    ctx.synchronize()
    unsafe_memcpy(dest=rewards_host.unsafe_ptr(), src=rewards, count=total)
    unsafe_memcpy(dest=offsets_host.unsafe_ptr(), src=offsets, count=groups + 1)

    var rewards_dev = ctx.enqueue_create_buffer[f32_dtype](total)
    var offsets_dev = ctx.enqueue_create_buffer[i32_dtype](groups + 1)
    var out_dev = ctx.enqueue_create_buffer[f32_dtype](total)
    ctx.enqueue_copy(dst_buf=rewards_dev, src_buf=rewards_host)
    ctx.enqueue_copy(dst_buf=offsets_dev, src_buf=offsets_host)
    ctx.enqueue_function[rloo_advantages_kernel](
        rewards_dev.unsafe_ptr(),
        offsets_dev.unsafe_ptr(),
        out_dev.unsafe_ptr(),
        grid_dim=groups,
        block_dim=THREADS_PER_BLOCK,
    )
    ctx.enqueue_copy(dst_buf=out_host, src_buf=out_dev)
    ctx.synchronize()
    unsafe_memcpy(dest=out_advantages, src=out_host.unsafe_ptr(), count=total)


# --- C ABI exports -----------------------------------------------------------
@export
def phlow_scoring_version() abi("C") -> Int32:
    """ABI smoke entry: returns the scoring ABI version (1)."""
    return Int32(1)


@export
def phlow_logprob_token_logps(
    logits: Pointer[Float32, MutAnyOrigin],
    rows: Int32,
    vocab: Int32,
    targets: Pointer[Int32, MutAnyOrigin],
    out_logps: Pointer[Float32, MutAnyOrigin],
) abi("C") -> Int32:
    """Per-token logprobs for `rows` logit rows of width `vocab`.

    logits: row-major f32 [rows, vocab]; targets: one token id per row,
    each in [0, vocab); out_logps: f32 [rows], written on STATUS_OK only.
    """
    if (
        rows < 1
        or rows > TOKENS_PER_LAUNCH_MAX
        or vocab < 1
        or vocab > VOCAB_SIZE_MAX
        or Int(rows) * Int(vocab) > LOGITS_ELEMENTS_MAX
    ):
        return STATUS_ERR_ARG
    for r in range(Int(rows)):
        var target = Int(targets[unsafe_offset=r])
        if target < 0 or target >= Int(vocab):
            return STATUS_ERR_ARG
    comptime if not has_accelerator():
        return STATUS_ERR_NO_DEVICE
    try:
        var ctx = DeviceContext()
        var out_host = ctx.enqueue_create_host_buffer[f32_dtype](Int(rows))
        ctx.synchronize()
        _ = _run_logprob(
            ctx, logits, Int(rows), Int(vocab), targets, out_host, 1
        )
        for i in range(Int(rows)):
            out_logps[unsafe_offset=i] = out_host[i]
    except:
        return STATUS_ERR_DEVICE
    return STATUS_OK


@export
def phlow_logprob_bench_ns(
    logits: Pointer[Float32, MutAnyOrigin],
    rows: Int32,
    vocab: Int32,
    targets: Pointer[Int32, MutAnyOrigin],
    repeats: Int32,
    out_total_ns: Pointer[Int64, MutAnyOrigin],
) abi("C") -> Int32:
    """Re-launch the logprob kernel `repeats` times on resident data.

    Writes the total launch-loop nanoseconds (enqueue + synchronize) to
    out_total_ns[unsafe_offset=0]. Same shape validation as phlow_logprob_token_logps.
    """
    if (
        rows < 1
        or rows > TOKENS_PER_LAUNCH_MAX
        or vocab < 1
        or vocab > VOCAB_SIZE_MAX
        or Int(rows) * Int(vocab) > LOGITS_ELEMENTS_MAX
        or repeats < 1
        or repeats > REPEATS_MAX
    ):
        return STATUS_ERR_ARG
    comptime if not has_accelerator():
        return STATUS_ERR_NO_DEVICE
    try:
        var ctx = DeviceContext()
        var scratch = ctx.enqueue_create_host_buffer[f32_dtype](Int(rows))
        ctx.synchronize()
        var elapsed = _run_logprob(
            ctx, logits, Int(rows), Int(vocab), targets, scratch, Int(repeats)
        )
        out_total_ns[unsafe_offset=0] = elapsed
    except:
        return STATUS_ERR_DEVICE
    return STATUS_OK


@export
def phlow_rloo_advantages(
    rewards: Pointer[Float32, MutAnyOrigin],
    offsets: Pointer[Int32, MutAnyOrigin],
    groups: Int32,
    out_advantages: Pointer[Float32, MutAnyOrigin],
) abi("C") -> Int32:
    """RLOO leave-one-out advantages for `groups` reward groups.

    offsets: i32 [groups + 1] prefix offsets into rewards (offsets[unsafe_offset=0] = 0,
    non-decreasing); every group size must be in [2, GROUP_COMPLETIONS_MAX]
    (leave-one-out is undefined below 2, mirroring trainlab). Output is
    f32 [offsets[unsafe_offset=groups]], written on STATUS_OK only.
    """
    if groups < 1 or groups > GROUPS_PER_LAUNCH_MAX:
        return STATUS_ERR_ARG
    if Int(offsets[unsafe_offset=0]) != 0:
        return STATUS_ERR_ARG
    var total = Int(offsets[unsafe_offset=Int(groups)])
    if total < 1 or total > COMPLETIONS_TOTAL_MAX:
        return STATUS_ERR_ARG
    for g in range(Int(groups)):
        var size = Int(offsets[unsafe_offset=g + 1]) - Int(offsets[unsafe_offset=g])
        if size < 2 or size > GROUP_COMPLETIONS_MAX:
            return STATUS_ERR_ARG
    comptime if not has_accelerator():
        return STATUS_ERR_NO_DEVICE
    try:
        var ctx = DeviceContext()
        _run_advantages(ctx, rewards, offsets, Int(groups), total, out_advantages)
    except:
        return STATUS_ERR_DEVICE
    return STATUS_OK
