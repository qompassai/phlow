# ===----------------------------------------------------------------------=== #
# phlow-trainer-mojo scoring kernels (Mojo 1.0 / MAX 26.5, pixi-pinned).
#
# Purpose: the parity-critical scoring math of the phlow-trainlab trainer
# contract, as GPU kernels behind a small C ABI:
#
#   * per-token logprob of target tokens given rows of vocab logits
#     (log-softmax reduction over the vocabulary, f32 in / f32 out),
#   * RLOO leave-one-out advantages for reward groups,
#   * one composable sampling kernel (greedy / temperature / top-k /
#     top-p, seeded SplitMix64 — deterministic per (seed, row, draw)),
#   * bench entries that re-launch the logprob/sampling kernels on
#     resident data.
#
# ABI version 2 (phlow_scoring_version): version 1 exports unchanged,
# sampling exports added.
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


# --- Sampling ---------------------------------------------------------------
# One composable sampling kernel covers greedy, temperature, top-k, and
# top-p (nucleus) sampling. Semantics (the reference implementations in
# Rust and in tools/check_sampling.py mirror these exactly):
#
#   * temperature == 0 selects greedy argmax (ties: lowest index wins).
#   * Otherwise probabilities are p_i = exp(logit_i / T - LSE_T), where
#     LSE_T is the log-sum-exp of the temperature-scaled row.
#   * Candidates are ordered by (logit descending, index ascending).
#     top_k > 0 restricts the universe to the first top_k candidates.
#     top_p < 1 then takes the smallest prefix of that ordering whose
#     cumulative full-vocab probability reaches top_p (the crossing
#     candidate is included). The sample is drawn from that prefix,
#     renormalized over the prefix's total probability mass.
#   * With no restriction binding (top_k == 0 and top_p >= 1), the draw
#     walks the full-vocab CDF in index order (chunked prefix sums).
#   * Candidate extraction is bounded at SAMPLE_CANDIDATES_MAX; if the
#     nucleus has not closed by then and top_k == 0, the kernel falls
#     back to the full-vocab CDF walk. If top_k > 0 pushed extraction
#     to the cap, the extracted prefix is the candidate set (documented
#     bound; real model rows close their nucleus far below the cap).
#   * The per-row uniform is SplitMix64 (the same generator trainlab's
#     rng.rs uses) on a state mixed from (seed, row, draw_index), so a
#     receipt carrying those three values reproduces the draw exactly.

comptime SAMPLE_CANDIDATES_MAX = 2_048  # extracted candidates per row
comptime SAMPLE_ROWS_PER_LAUNCH_MAX = 4_096  # rows per sampling launch
comptime SAMPLE_DRAW_INDEX_MAX = 1_000_000  # draw_index bound per call


def splitmix64_next(state: UInt64) -> UInt64:
    """One SplitMix64 step (mirrors phlow-trainlab's rng.rs exactly)."""
    var z = state + UInt64(0x9E3779B97F4A7C15)
    z = (z ^ (z >> 30)) * UInt64(0xBF58476D1CE4E5B9)
    z = (z ^ (z >> 27)) * UInt64(0x94D049BB133111EB)
    return z ^ (z >> 31)


def sample_uniform(seed: UInt64, row: Int, draw_index: Int) -> Float32:
    """Deterministic uniform in [0, 1) for one (seed, row, draw)."""
    var mixed = (
        seed
        ^ (UInt64(row + 1) * UInt64(0x9E3779B97F4A7C15))
        ^ (UInt64(draw_index) * UInt64(0xBF58476D1CE4E5B9))
    )
    var z = splitmix64_next(mixed)
    # Top 24 bits / 2^24: exactly representable steps in f32.
    return Float32(Int(z >> 40)) * Float32(1.0 / 16777216.0)




def sample_rows_kernel(
    logits: Pointer[Float32, MutAnyOrigin],
    cand_idx: Pointer[Int32, MutAnyOrigin],
    cand_val: Pointer[Float32, MutAnyOrigin],
    out_tokens: Pointer[Int32, MutAnyOrigin],
    out_counts: Pointer[Int32, MutAnyOrigin],
    vocab: Int32,
    temperature: Float32,
    top_k: Int32,
    top_p: Float32,
    seed: UInt64,
    draw_index: Int32,
):
    """One block per row: sample one token per row (see section header).

    cand_idx/cand_val are per-row scratch of SAMPLE_CANDIDATES_MAX
    entries holding the extracted candidate prefix in sampling order;
    out_counts[row] receives the candidate-set size actually used
    (vocab for the unrestricted CDF walk, 1 for greedy).

    Threading contract: pick state that every thread reads (last picked
    value/index, loop outcome) lives in shared memory and is written
    only by thread 0 between barriers; cumprob/count are thread-0
    locals used solely by thread 0's final walk.
    """
    var row = Int(block_idx.x)
    var tid = Int(thread_idx.x)
    var width = Int(vocab)
    var base = row * width
    var u = sample_uniform(seed, row, Int(draw_index))

    var shared_val = stack_allocation[
        THREADS_PER_BLOCK, Float32, address_space=AddressSpace.SHARED
    ]()
    var shared_idx = stack_allocation[
        THREADS_PER_BLOCK, Int32, address_space=AddressSpace.SHARED
    ]()
    var shared_max = stack_allocation[
        THREADS_PER_BLOCK, Float32, address_space=AddressSpace.SHARED
    ]()
    var shared_sum = stack_allocation[
        THREADS_PER_BLOCK, Float32, address_space=AddressSpace.SHARED
    ]()
    # Scalar shared slots: [0]=lse/residual, [1]=last picked value.
    var shared_scalar = stack_allocation[
        2, Float32, address_space=AddressSpace.SHARED
    ]()
    # Flag slots: [0]=loop outcome / path decision, [1]=last picked idx.
    var shared_flag = stack_allocation[
        2, Int32, address_space=AddressSpace.SHARED
    ]()

    # --- Greedy: block argmax, lowest index wins ties. ---
    if temperature == Float32(0):
        var best_val = F32_LOWEST
        var best_idx = width
        var col = tid
        while col < width:
            var value = logits[unsafe_offset=base + col]
            if value > best_val or (value == best_val and col < best_idx):
                best_val = value
                best_idx = col
            col += THREADS_PER_BLOCK
        shared_val[unsafe_offset=tid] = best_val
        shared_idx[unsafe_offset=tid] = Int32(best_idx)
        barrier()
        var stride = THREADS_PER_BLOCK // 2
        while stride > 0:
            if tid < stride:
                var other_val = shared_val[unsafe_offset=tid + stride]
                var own_val = shared_val[unsafe_offset=tid]
                var other_idx = shared_idx[unsafe_offset=tid + stride]
                var own_idx = shared_idx[unsafe_offset=tid]
                if other_val > own_val or (
                    other_val == own_val and other_idx < own_idx
                ):
                    shared_val[unsafe_offset=tid] = other_val
                    shared_idx[unsafe_offset=tid] = other_idx
            barrier()
            stride //= 2
        if tid == 0:
            out_tokens[unsafe_offset=row] = shared_idx[unsafe_offset=0]
            out_counts[unsafe_offset=row] = Int32(1)
            cand_idx[unsafe_offset=row * SAMPLE_CANDIDATES_MAX] = (
                shared_idx[unsafe_offset=0]
            )
            cand_val[unsafe_offset=row * SAMPLE_CANDIDATES_MAX] = (
                shared_val[unsafe_offset=0]
            )
        return

    # --- Scaled log-sum-exp of the row (online, as in logprob_rows). ---
    var local_max = F32_LOWEST
    var local_sum = Float32(0)
    var col2 = tid
    while col2 < width:
        var scaled = logits[unsafe_offset=base + col2] / temperature
        if scaled > local_max:
            local_sum = local_sum * exp(local_max - scaled) + Float32(1)
            local_max = scaled
        else:
            local_sum += exp(scaled - local_max)
        col2 += THREADS_PER_BLOCK
    shared_max[unsafe_offset=tid] = local_max
    shared_sum[unsafe_offset=tid] = local_sum
    barrier()
    var stride2 = THREADS_PER_BLOCK // 2
    while stride2 > 0:
        if tid < stride2:
            var other_max = shared_max[unsafe_offset=tid + stride2]
            var own_max = shared_max[unsafe_offset=tid]
            if other_max > own_max:
                shared_sum[unsafe_offset=tid] = (
                    shared_sum[unsafe_offset=tid] * exp(own_max - other_max)
                    + shared_sum[unsafe_offset=tid + stride2]
                )
                shared_max[unsafe_offset=tid] = other_max
            else:
                shared_sum[unsafe_offset=tid] += shared_sum[
                    unsafe_offset=tid + stride2
                ] * exp(other_max - own_max)
        barrier()
        stride2 //= 2
    if tid == 0:
        shared_scalar[unsafe_offset=0] = (
            shared_max[unsafe_offset=0] + log(shared_sum[unsafe_offset=0])
        )
        shared_scalar[unsafe_offset=1] = Float32(0)
        shared_flag[unsafe_offset=0] = Int32(0)
        shared_flag[unsafe_offset=1] = Int32(-1)
    barrier()
    var lse = shared_scalar[unsafe_offset=0]

    var restricted = top_k > 0 or top_p < Float32(1)
    var use_cdf = not restricted

    # --- Restricted path: ordered candidate extraction. ---
    if restricted:
        var limit = SAMPLE_CANDIDATES_MAX
        if top_k > 0 and Int(top_k) < limit:
            limit = Int(top_k)
        if width < limit:
            limit = width
        var cand_base = row * SAMPLE_CANDIDATES_MAX
        var cumprob = Float32(0)
        var count = 0
        for _ in range(limit):
            var have_pick = shared_flag[unsafe_offset=1] >= 0
            var last_val = shared_scalar[unsafe_offset=1]
            var last_idx = Int(shared_flag[unsafe_offset=1])
            # Next candidate: the maximum element strictly after the
            # last pick in (value desc, index asc) order -- earlier
            # picks all precede the last one, so this excludes exactly
            # the picked set.
            var pick_val = F32_LOWEST
            var pick_idx = width
            var col3 = tid
            while col3 < width:
                var value = logits[unsafe_offset=base + col3]
                var after = True
                if have_pick:
                    after = value < last_val or (
                        value == last_val and col3 > last_idx
                    )
                if after and (
                    value > pick_val or (value == pick_val and col3 < pick_idx)
                ):
                    pick_val = value
                    pick_idx = col3
                col3 += THREADS_PER_BLOCK
            shared_val[unsafe_offset=tid] = pick_val
            shared_idx[unsafe_offset=tid] = Int32(pick_idx)
            barrier()
            var stride3 = THREADS_PER_BLOCK // 2
            while stride3 > 0:
                if tid < stride3:
                    var other_val = shared_val[unsafe_offset=tid + stride3]
                    var own_val = shared_val[unsafe_offset=tid]
                    var other_idx = shared_idx[unsafe_offset=tid + stride3]
                    var own_idx = shared_idx[unsafe_offset=tid]
                    if other_val > own_val or (
                        other_val == own_val and other_idx < own_idx
                    ):
                        shared_val[unsafe_offset=tid] = other_val
                        shared_idx[unsafe_offset=tid] = other_idx
                barrier()
                stride3 //= 2
            if tid == 0:
                var chosen_idx = Int(shared_idx[unsafe_offset=0])
                var chosen_val = shared_val[unsafe_offset=0]
                if chosen_idx >= width:
                    # Universe exhausted before the loop bound.
                    shared_flag[unsafe_offset=0] = Int32(1)
                else:
                    cand_idx[unsafe_offset=cand_base + count] = Int32(
                        chosen_idx
                    )
                    cand_val[unsafe_offset=cand_base + count] = chosen_val
                    cumprob += exp(chosen_val / temperature - lse)
                    count += 1
                    shared_scalar[unsafe_offset=1] = chosen_val
                    shared_flag[unsafe_offset=1] = Int32(chosen_idx)
                    if cumprob >= top_p:
                        shared_flag[unsafe_offset=0] = Int32(2)
                    else:
                        shared_flag[unsafe_offset=0] = Int32(0)
            barrier()
            if shared_flag[unsafe_offset=0] != Int32(0):
                break
        # Thread 0 decides the path and, on the extraction path, walks
        # the candidate prefix. Outcome codes: 0/2 = sample from the
        # prefix (loop bound reached / nucleus closed / exhausted);
        # 1 after the loop also means exhausted -- only the flag value
        # 3 selects the CDF fallback, written here by thread 0.
        if tid == 0:
            out_counts[unsafe_offset=row] = Int32(count)
            var nucleus_open = cumprob < top_p and count < width
            if nucleus_open and top_k == 0 and count >= limit:
                shared_flag[unsafe_offset=0] = Int32(3)
            else:
                shared_flag[unsafe_offset=0] = Int32(0)
                var target = u * cumprob
                var acc = Float32(0)
                var chosen = Int(
                    cand_idx[unsafe_offset=cand_base + count - 1]
                )
                for j in range(count):
                    acc += exp(
                        cand_val[unsafe_offset=cand_base + j] / temperature
                        - lse
                    )
                    if acc > target:
                        chosen = Int(cand_idx[unsafe_offset=cand_base + j])
                        break
                out_tokens[unsafe_offset=row] = Int32(chosen)
        barrier()
        if shared_flag[unsafe_offset=0] == Int32(3):
            use_cdf = True
        else:
            use_cdf = False

    # --- Unrestricted path: chunked CDF walk in index order. ---
    if use_cdf:
        var chunk = (width + THREADS_PER_BLOCK - 1) // THREADS_PER_BLOCK
        var start = tid * chunk
        var end = start + chunk
        if end > width:
            end = width
        var chunk_sum = Float32(0)
        var col4 = start
        while col4 < end:
            chunk_sum += exp(
                logits[unsafe_offset=base + col4] / temperature - lse
            )
            col4 += 1
        shared_sum[unsafe_offset=tid] = chunk_sum
        barrier()
        if tid == 0:
            var total = Float32(0)
            for t in range(THREADS_PER_BLOCK):
                total += shared_sum[unsafe_offset=t]
            var target = u * total
            var acc = Float32(0)
            var chosen_chunk = THREADS_PER_BLOCK - 1
            var residual = target
            for t in range(THREADS_PER_BLOCK):
                var part = shared_sum[unsafe_offset=t]
                if acc + part > target:
                    chosen_chunk = t
                    residual = target - acc
                    break
                acc += part
            shared_idx[unsafe_offset=0] = Int32(chosen_chunk)
            shared_scalar[unsafe_offset=0] = residual
        barrier()
        if tid == Int(shared_idx[unsafe_offset=0]):
            var residual = shared_scalar[unsafe_offset=0]
            var acc2 = Float32(0)
            var chosen = width - 1
            var col5 = start
            while col5 < end:
                acc2 += exp(
                    logits[unsafe_offset=base + col5] / temperature - lse
                )
                if acc2 > residual:
                    chosen = col5
                    break
                col5 += 1
            out_tokens[unsafe_offset=row] = Int32(chosen)
            out_counts[unsafe_offset=row] = Int32(width)


def _run_sample(
    mut ctx: DeviceContext,
    logits: Pointer[Float32, MutAnyOrigin],
    rows: Int,
    vocab: Int,
    temperature: Float32,
    top_k: Int32,
    top_p: Float32,
    seed: UInt64,
    draw_index: Int32,
    out_tokens: Pointer[Int32, MutAnyOrigin],
    out_counts: Pointer[Int32, MutAnyOrigin],
    cand_idx: Pointer[Int32, MutAnyOrigin],
    cand_val: Pointer[Float32, MutAnyOrigin],
    repeats: Int,
) raises -> Int64:
    """Upload once, launch `repeats` times, download tokens/counts once.

    Candidate buffers are device scratch owned by this scope; the
    caller-visible candidate prefix is copied out only when repeats
    == 1 (bench launches reuse the same seed, so their candidate sets
    are identical and uninteresting). Returns launch-loop nanoseconds.
    """
    var elements = rows * vocab
    var logits_host = ctx.enqueue_create_host_buffer[f32_dtype](elements)
    ctx.synchronize()
    unsafe_memcpy(dest=logits_host.unsafe_ptr(), src=logits, count=elements)

    var logits_dev = ctx.enqueue_create_buffer[f32_dtype](elements)
    var cand_idx_dev = ctx.enqueue_create_buffer[i32_dtype](
        rows * SAMPLE_CANDIDATES_MAX
    )
    var cand_val_dev = ctx.enqueue_create_buffer[f32_dtype](
        rows * SAMPLE_CANDIDATES_MAX
    )
    var out_dev = ctx.enqueue_create_buffer[i32_dtype](rows)
    var counts_dev = ctx.enqueue_create_buffer[i32_dtype](rows)
    ctx.enqueue_copy(dst_buf=logits_dev, src_buf=logits_host)

    var started = perf_counter_ns()
    for _ in range(repeats):
        ctx.enqueue_function[sample_rows_kernel](
            logits_dev.unsafe_ptr(),
            cand_idx_dev.unsafe_ptr(),
            cand_val_dev.unsafe_ptr(),
            out_dev.unsafe_ptr(),
            counts_dev.unsafe_ptr(),
            Int32(vocab),
            temperature,
            top_k,
            top_p,
            seed,
            draw_index,
            grid_dim=rows,
            block_dim=THREADS_PER_BLOCK,
        )
    ctx.synchronize()
    var elapsed = perf_counter_ns() - started

    var out_host = ctx.enqueue_create_host_buffer[i32_dtype](rows)
    var counts_host = ctx.enqueue_create_host_buffer[i32_dtype](rows)
    ctx.enqueue_copy(dst_buf=out_host, src_buf=out_dev)
    ctx.enqueue_copy(dst_buf=counts_host, src_buf=counts_dev)
    ctx.synchronize()
    unsafe_memcpy(dest=out_tokens, src=out_host.unsafe_ptr(), count=rows)
    unsafe_memcpy(dest=out_counts, src=counts_host.unsafe_ptr(), count=rows)
    if repeats == 1:
        var cand_idx_host = ctx.enqueue_create_host_buffer[i32_dtype](
            rows * SAMPLE_CANDIDATES_MAX
        )
        var cand_val_host = ctx.enqueue_create_host_buffer[f32_dtype](
            rows * SAMPLE_CANDIDATES_MAX
        )
        ctx.enqueue_copy(dst_buf=cand_idx_host, src_buf=cand_idx_dev)
        ctx.enqueue_copy(dst_buf=cand_val_host, src_buf=cand_val_dev)
        ctx.synchronize()
        unsafe_memcpy(
            dest=cand_idx,
            src=cand_idx_host.unsafe_ptr(),
            count=rows * SAMPLE_CANDIDATES_MAX,
        )
        unsafe_memcpy(
            dest=cand_val,
            src=cand_val_host.unsafe_ptr(),
            count=rows * SAMPLE_CANDIDATES_MAX,
        )
    return Int64(elapsed)


def _validate_sample_args(
    rows: Int32,
    vocab: Int32,
    temperature: Float32,
    top_k: Int32,
    top_p: Float32,
    draw_index: Int32,
) -> Bool:
    """Shared ABI validation for the sampling exports."""
    if (
        rows < 1
        or rows > SAMPLE_ROWS_PER_LAUNCH_MAX
        or vocab < 1
        or vocab > VOCAB_SIZE_MAX
        or Int(rows) * Int(vocab) > LOGITS_ELEMENTS_MAX
    ):
        return False
    if temperature != temperature or temperature < 0 or temperature > 2:
        return False
    if top_k < 0 or top_k > SAMPLE_CANDIDATES_MAX:
        return False
    if top_p != top_p or top_p <= 0 or top_p > 1:
        return False
    if draw_index < 0 or draw_index > SAMPLE_DRAW_INDEX_MAX:
        return False
    return True


# --- C ABI exports -----------------------------------------------------------
@export
def phlow_scoring_version() abi("C") -> Int32:
    """ABI smoke entry: returns the scoring ABI version (2 since sampling)."""
    return Int32(2)


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


@export
def phlow_sample_tokens(
    logits: Pointer[Float32, MutAnyOrigin],
    rows: Int32,
    vocab: Int32,
    temperature: Float32,
    top_k: Int32,
    top_p: Float32,
    seed: UInt64,
    draw_index: Int32,
    out_tokens: Pointer[Int32, MutAnyOrigin],
    out_counts: Pointer[Int32, MutAnyOrigin],
    cand_idx: Pointer[Int32, MutAnyOrigin],
    cand_val: Pointer[Float32, MutAnyOrigin],
) abi("C") -> Int32:
    """Sample one token per logit row (composable sampling kernel).

    temperature == 0 is greedy; top_k == 0 disables the top-k limit;
    top_p == 1 disables the nucleus limit. cand_idx/cand_val receive
    the extracted candidate prefix per row (rows *
    SAMPLE_CANDIDATES_MAX entries each; only the first
    out_counts[row] entries are meaningful, and only on the restricted
    path). Outputs are written on STATUS_OK only. The draw is a pure
    function of (logits, temperature, top_k, top_p, seed, draw_index).
    """
    if not _validate_sample_args(rows, vocab, temperature, top_k, top_p,
                                  draw_index):
        return STATUS_ERR_ARG
    comptime if not has_accelerator():
        return STATUS_ERR_NO_DEVICE
    try:
        var ctx = DeviceContext()
        _ = _run_sample(
            ctx, logits, Int(rows), Int(vocab), temperature, top_k, top_p,
            seed, draw_index, out_tokens, out_counts, cand_idx, cand_val, 1
        )
    except:
        return STATUS_ERR_DEVICE
    return STATUS_OK


@export
def phlow_sample_bench_ns(
    logits: Pointer[Float32, MutAnyOrigin],
    rows: Int32,
    vocab: Int32,
    temperature: Float32,
    top_k: Int32,
    top_p: Float32,
    seed: UInt64,
    repeats: Int32,
    out_total_ns: Pointer[Int64, MutAnyOrigin],
) abi("C") -> Int32:
    """Re-launch the sampling kernel `repeats` times on resident data.

    Every launch draws draw_index 0 with the given seed (bench timing
    only; outputs are discarded except the timing). Writes the total
    launch-loop nanoseconds to out_total_ns[0].
    """
    if (
        not _validate_sample_args(rows, vocab, temperature, top_k, top_p,
                                  Int32(0))
        or repeats < 1
        or repeats > REPEATS_MAX
    ):
        return STATUS_ERR_ARG
    comptime if not has_accelerator():
        return STATUS_ERR_NO_DEVICE
    try:
        var ctx = DeviceContext()
        var scratch_tokens = ctx.enqueue_create_host_buffer[i32_dtype](
            Int(rows)
        )
        var scratch_counts = ctx.enqueue_create_host_buffer[i32_dtype](
            Int(rows)
        )
        var scratch_idx = ctx.enqueue_create_host_buffer[i32_dtype](1)
        var scratch_val = ctx.enqueue_create_host_buffer[f32_dtype](1)
        ctx.synchronize()
        var elapsed = _run_sample(
            ctx, logits, Int(rows), Int(vocab), temperature, top_k, top_p,
            seed, Int32(0),
            rebind[Pointer[Int32, MutAnyOrigin]](
                scratch_tokens.unsafe_ptr()
            ),
            rebind[Pointer[Int32, MutAnyOrigin]](
                scratch_counts.unsafe_ptr()
            ),
            rebind[Pointer[Int32, MutAnyOrigin]](scratch_idx.unsafe_ptr()),
            rebind[Pointer[Float32, MutAnyOrigin]](
                scratch_val.unsafe_ptr()
            ),
            Int(repeats),
        )
        out_total_ns[unsafe_offset=0] = elapsed
    except:
        return STATUS_ERR_DEVICE
    return STATUS_OK
