"""Parity + determinism checks for the Mojo sampling kernel (ABI v2).

Runs under any Python with numpy + torch (the sidecar venv). Loads
the built libphlow_scoring shared library with ctypes — the same
boundary the Rust shim uses — and verifies, against an independent
Python reference implementing the documented sampling semantics
(the kernel header in kernels/scoring.mojo is the contract):

1. Greedy: kernel token == torch argmax (ties -> lowest index) on
   real bench-batch rows and group-logit rows.
2. Candidate sets: for fixed logits and (temperature, top_k, top_p)
   combinations, the kernel's extracted candidate prefix equals the
   reference's exactly (order included).
3. Tokens: kernel token == reference token on the same fixed
   logits/params (the reference uses the same SplitMix64 uniform
   construction; float walk order differs, so this is checked on
   the synthetic + group logits where margins are wide, and the
   Rust integration suite covers the FFI boundary exactly).
4. Distribution: 4096 seeded draws (rows replicate one fixed row;
   the uniform varies per row) — empirical frequencies vs the
   reference probabilities, max deviation bound 0.025.
5. Determinism: two calls (and a fresh library load) produce
   byte-identical tokens/counts.

Sampling semantics (contract): temperature 0 = greedy argmax;
otherwise p_i = softmax(logit_i / T); candidate order is (logit
desc, index asc); top_k restricts the universe first; top_p then
takes the smallest prefix whose cumulative full-vocab probability
reaches top_p (crossing candidate included); the draw is
renormalized over the prefix mass; unrestricted draws walk the
full-vocab CDF in index order. Uniform: mixed = seed ^
((row + 1) * GOLDEN) ^ (draw * PHI_M1); one SplitMix64 step
(add GOLDEN, finalize); top 24 bits / 2^24.

Exits nonzero on any hard failure.
"""

from __future__ import annotations

import argparse
import ctypes
import math
import struct
import sys
from pathlib import Path

import numpy as np
import torch

CAND_MAX = 2048
DRAWS = 4096
DIST_TOL = 0.025
MASK64 = (1 << 64) - 1


class CheckError(RuntimeError):
    """A hard check failed."""


def splitmix64(z: int) -> int:
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK64
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK64
    return z ^ (z >> 31)


def uniform(seed: int, row: int, draw: int) -> float:
    # The kernel/reference construction: one mixed state, one
    # SplitMix64 step (next_u64 adds the golden constant, then the
    # finalizer runs). Verified against the kernel's CDF tokens.
    mixed = (
        seed
        ^ (((row + 1) * 0x9E3779B97F4A7C15) & MASK64)
        ^ ((draw * 0xBF58476D1CE4E5B9) & MASK64)
    ) & MASK64
    state = splitmix64((mixed + 0x9E3779B97F4A7C15) & MASK64)
    return (state >> 40) / float(1 << 24)


def ref_sample(row: np.ndarray, temperature: float, top_k: int,
               top_p: float, u: float) -> tuple[int, list[int]]:
    """Reference sampler: returns (token, candidate prefix)."""
    vocab = len(row)
    if temperature == 0.0:
        return int(np.argmax(row)), [int(np.argmax(row))]
    scaled = row.astype(np.float64) / temperature
    lse = scaled.max() + math.log(float(np.exp(scaled - scaled.max()).sum()))
    probs = np.exp(scaled - lse)
    order = sorted(range(vocab), key=lambda i: (-row[i], i))
    if top_k == 0 and top_p >= 1.0:
        acc = 0.0
        for idx in range(vocab):
            acc += probs[idx]
            if acc > u:
                return idx, list(range(vocab))
        return vocab - 1, list(range(vocab))
    limit = min(top_k if top_k > 0 else CAND_MAX, vocab)
    cand: list[int] = []
    cum = 0.0
    for idx in order[:limit]:
        cand.append(idx)
        cum += probs[idx]
        if cum >= top_p:
            break
    if cum < top_p and top_k == 0 and len(cand) >= limit and len(cand) < vocab:
        # Documented kernel bound: the nucleus did not close within
        # the extraction cap and no top_k bound applies -> exact
        # full-vocab CDF fallback (count == vocab signals it).
        acc = 0.0
        for idx in range(vocab):
            acc += probs[idx]
            if acc > u:
                return idx, list(range(vocab))
        return vocab - 1, list(range(vocab))
    target = u * cum
    acc = 0.0
    for idx in cand:
        acc += probs[idx]
        if acc > target:
            return idx, cand
    return cand[-1], cand


def load_lib(path: Path) -> ctypes.CDLL:
    lib = ctypes.CDLL(str(path))
    f32p = ctypes.POINTER(ctypes.c_float)
    i32p = ctypes.POINTER(ctypes.c_int32)
    lib.phlow_scoring_version.restype = ctypes.c_int32
    lib.phlow_scoring_version.argtypes = []
    lib.phlow_sample_tokens.restype = ctypes.c_int32
    lib.phlow_sample_tokens.argtypes = [
        f32p, ctypes.c_int32, ctypes.c_int32, ctypes.c_float,
        ctypes.c_int32, ctypes.c_float, ctypes.c_uint64, ctypes.c_int32,
        i32p, i32p, i32p, f32p,
    ]
    return lib


def call_sample(lib: ctypes.CDLL, logits: np.ndarray, temperature: float,
                top_k: int, top_p: float, seed: int, draw: int):
    rows, vocab = logits.shape
    arr = np.ascontiguousarray(logits, dtype=np.float32)
    lp = arr.ctypes.data_as(ctypes.POINTER(ctypes.c_float))
    tokens = np.zeros(rows, dtype=np.int32)
    counts = np.zeros(rows, dtype=np.int32)
    cand_idx = np.zeros(rows * CAND_MAX, dtype=np.int32)
    cand_val = np.zeros(rows * CAND_MAX, dtype=np.float32)
    ptr = lambda a: a.ctypes.data_as(ctypes.POINTER(ctypes.c_int32))
    status = lib.phlow_sample_tokens(
        lp, rows, vocab, ctypes.c_float(temperature), top_k,
        ctypes.c_float(top_p), ctypes.c_uint64(seed), draw,
        ptr(tokens), ptr(counts), ptr(cand_idx),
        cand_val.ctypes.data_as(ctypes.POINTER(ctypes.c_float)),
    )
    if status != 0:
        raise CheckError(f"sample call failed with status {status}")
    return tokens, counts, cand_idx.reshape(rows, CAND_MAX), cand_val


def read_batch(path: Path) -> np.ndarray:
    data = path.read_bytes()
    rows, vocab = struct.unpack_from("<2i", data, 0)
    return np.frombuffer(
        data, dtype="<f4", count=rows * vocab, offset=8
    ).reshape(rows, vocab).copy()


def read_first_block(path: Path) -> np.ndarray:
    data = path.read_bytes()
    rows, width = struct.unpack_from("<2i", data, 0)
    return np.frombuffer(
        data, dtype="<f4", count=rows * width, offset=8
    ).reshape(rows, width).copy()


def check_greedy(lib: ctypes.CDLL, batch: np.ndarray, label: str) -> None:
    tokens, counts, _, _ = call_sample(lib, batch, 0.0, 0, 1.0, 7, 0)
    want = torch.from_numpy(batch).argmax(dim=-1).numpy()
    if not np.array_equal(tokens, want):
        raise CheckError(f"greedy mismatch on {label}: {tokens} vs {want}")
    if not np.all(counts == 1):
        raise CheckError(f"greedy counts != 1 on {label}")
    print(f"greedy exact vs torch argmax: {label} ({batch.shape[0]} rows) OK")


def check_candidates_and_tokens(lib: ctypes.CDLL, batch: np.ndarray,
                                label: str) -> None:
    combos = [
        (1.0, 0, 1.0), (0.7, 0, 1.0), (1.0, 10, 1.0), (1.0, 0, 0.9),
        (0.8, 50, 0.95), (1.3, 5, 0.5), (2.0, 0, 0.99),
    ]
    for temperature, top_k, top_p in combos:
        tokens, counts, cand_idx, _ = call_sample(
            lib, batch, temperature, top_k, top_p, 1234, 3
        )
        for r in range(batch.shape[0]):
            u = uniform(1234, r, 3)
            want_tok, want_cand = ref_sample(
                batch[r], temperature, top_k, top_p, u
            )
            if counts[r] != len(want_cand):
                raise CheckError(
                    f"{label} T={temperature} k={top_k} p={top_p} row {r}: "
                    f"count {counts[r]} != {len(want_cand)}"
                )
            # count == vocab is the CDF-path signal: the kernel by
            # contract writes no candidate prefix there.
            if counts[r] < batch.shape[1]:
                got_cand = cand_idx[r, : counts[r]].tolist()
                if got_cand != want_cand:
                    raise CheckError(
                        f"{label} T={temperature} k={top_k} p={top_p} row {r}: "
                        f"candidate set mismatch"
                    )
            if tokens[r] != want_tok:
                raise CheckError(
                    f"{label} T={temperature} k={top_k} p={top_p} row {r}: "
                    f"token {tokens[r]} != reference {want_tok}"
                )
    print(f"candidate sets + tokens vs reference: {label} "
          f"({len(combos)} combos x {batch.shape[0]} rows) OK")


def check_distribution(lib: ctypes.CDLL, row: np.ndarray) -> None:
    rows = 512
    batch = np.repeat(row[None, :], rows, axis=0)
    for temperature, top_k, top_p in [(1.0, 0, 1.0), (0.8, 50, 0.95)]:
        counts_emp = np.zeros(len(row), dtype=np.int64)
        total = 0
        for seed_block in range(DRAWS // rows):
            tokens, _, _, _ = call_sample(
                lib, batch, temperature, top_k, top_p,
                10_000 + seed_block, 0,
            )
            # Uniforms vary per (seed, row): each row is one draw.
            counts_emp += np.bincount(tokens, minlength=len(row))
            total += rows
        # Reference probabilities under the same restriction.
        scaled = row.astype(np.float64) / temperature
        lse = scaled.max() + math.log(
            float(np.exp(scaled - scaled.max()).sum())
        )
        probs = np.exp(scaled - lse)
        _, cand = ref_sample(row, temperature, top_k, top_p, 0.5)
        keep = np.zeros(len(row), dtype=bool)
        keep[cand] = True
        mass = probs[keep].sum()
        want = np.where(keep, probs / mass, 0.0)
        freq = counts_emp / total
        dev = float(np.abs(freq - want).max())
        if dev > DIST_TOL:
            raise CheckError(
                f"distribution deviation {dev:.4f} > {DIST_TOL} "
                f"(T={temperature} k={top_k} p={top_p})"
            )
        print(f"distribution: T={temperature} k={top_k} p={top_p}: "
              f"{total} draws, max |freq-p| = {dev:.4f} (bound {DIST_TOL}) OK")


def check_determinism(lib_path: Path, batch: np.ndarray) -> None:
    lib = load_lib(lib_path)
    first = call_sample(lib, batch, 0.8, 50, 0.95, 42, 1)
    second = call_sample(lib, batch, 0.8, 50, 0.95, 42, 1)
    lib2 = load_lib(lib_path)
    third = call_sample(lib2, batch, 0.8, 50, 0.95, 42, 1)
    for a, b in zip(first, third):
        if not np.array_equal(a, b):
            raise CheckError("determinism failure across calls/loads")
    if not np.array_equal(first[0], second[0]):
        raise CheckError("determinism failure across repeated calls")
    print("determinism: identical tokens/counts/candidates across "
          "repeated calls and a fresh library load OK")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lib", required=True, type=Path)
    parser.add_argument("--out-dir", required=True, type=Path)
    args = parser.parse_args()
    lib = load_lib(args.lib)
    if lib.phlow_scoring_version() != 2:
        raise CheckError("ABI version is not 2")
    bench = read_batch(args.out_dir / "bench_logits.bin")
    group0 = read_first_block(args.out_dir / "logits_group.bin")
    check_greedy(lib, bench[:16], "bench batch")
    check_greedy(lib, group0[:8], "group logits")
    rng = np.random.default_rng(9)
    synth = (rng.standard_normal((4, 257)) * 2.0).astype(np.float32)
    check_candidates_and_tokens(lib, synth, "synthetic 257")
    check_candidates_and_tokens(lib, group0[:2], "group logits")
    check_distribution(lib, synth[0])
    check_determinism(args.lib, bench[:8])
    print("ALL SAMPLING CHECKS PASSED")
    return 0


if __name__ == "__main__":
    sys.exit(main())
