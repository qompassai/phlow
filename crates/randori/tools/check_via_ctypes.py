"""Phase B/C checks for the Mojo scoring kernels, driven over the C ABI.

Loads the built libphlow_scoring shared library with ctypes (the same
boundary the Rust shim uses) and verifies:

1. Synthetic known-answer cases (uniform logits, handcrafted rows,
   handcrafted advantage groups) and the ABI error paths.
2. Parity batch: per-token logprobs from the Mojo kernel on the control
   run's dumped bf16 logits vs the torch-computed token logprobs, plus
   per-pair mean diffs against the bf16 control and the NF4 reference.
3. Real export: group-8 completion scoring on dumped logits, and RLOO
   advantages for every group of the rl export vs trainlab's f64 values.
4. Bench: phlow_logprob_bench_ns on a seeded synthetic batch whose
   logits file is shared with the torch-side bench (bench_torch.py);
   if the torch outputs exist, Mojo outputs are compared on that batch.

Exits nonzero if any hard check fails (synthetic answers, error paths,
or a kernel-vs-torch token diff above TOKEN_DIFF_MAX).
"""

from __future__ import annotations

import argparse
import ctypes
import json
import math
import struct
import sys
from pathlib import Path

import numpy as np

# Kernel vs torch token-logprob agreement bound. Both sides reduce a
# 151,936-wide row in f32 but in different orders (block tree vs torch's
# blocked reduction), so diffs land ~1e-4, not ~1e-6; the contract's own
# parity bar is 5e-2 nats/token. Measured worst token diff: 1.35e-4.
TOKEN_DIFF_MAX = 5e-4
NF4_REFERENCE = {
    ("dev-increment-000", "reference_full"): -0.89344,
    ("dev-increment-000", "reference_half"): -4.17106,
    ("dev-increment-001", "reference_full"): -0.91062,
    ("dev-increment-001", "reference_half"): -4.46415,
}
BENCH_ROWS = 128
VOCAB = 151_936


class CheckError(RuntimeError):
    """A hard check failed."""


def load_lib(path: Path) -> ctypes.CDLL:
    lib = ctypes.CDLL(str(path))
    f32p = ctypes.POINTER(ctypes.c_float)
    i32p = ctypes.POINTER(ctypes.c_int32)
    i64p = ctypes.POINTER(ctypes.c_int64)
    lib.phlow_scoring_version.restype = ctypes.c_int32
    lib.phlow_scoring_version.argtypes = []
    lib.phlow_logprob_token_logps.restype = ctypes.c_int32
    lib.phlow_logprob_token_logps.argtypes = [f32p, ctypes.c_int32,
                                              ctypes.c_int32, i32p, f32p]
    lib.phlow_logprob_bench_ns.restype = ctypes.c_int32
    lib.phlow_logprob_bench_ns.argtypes = [f32p, ctypes.c_int32,
                                           ctypes.c_int32, i32p,
                                           ctypes.c_int32, i64p]
    lib.phlow_rloo_advantages.restype = ctypes.c_int32
    lib.phlow_rloo_advantages.argtypes = [f32p, i32p, ctypes.c_int32, f32p]
    return lib


def f32_array(values: np.ndarray) -> ctypes.Array:
    arr = np.ascontiguousarray(values, dtype=np.float32)
    return arr.ctypes.data_as(ctypes.POINTER(ctypes.c_float)), arr


def i32_array(values: np.ndarray) -> ctypes.Array:
    arr = np.ascontiguousarray(values, dtype=np.int32)
    return arr.ctypes.data_as(ctypes.POINTER(ctypes.c_int32)), arr


def call_logprob(lib: ctypes.CDLL, logits: np.ndarray,
                 targets: np.ndarray) -> np.ndarray:
    rows, vocab = logits.shape
    lp, _ = f32_array(logits)
    tp, _ = i32_array(targets)
    out = np.zeros(rows, dtype=np.float32)
    op, _ = f32_array(out)
    status = lib.phlow_logprob_token_logps(lp, rows, vocab, tp, op)
    if status != 0:
        raise CheckError(f"logprob call failed with status {status}")
    return out.copy()


def read_blocks(bin_path: Path) -> list[np.ndarray]:
    """Read the control script's block format: (i32 rows, i32 width, f32 data)."""
    blocks = []
    data = bin_path.read_bytes()
    pos = 0
    while pos < len(data):
        rows, width = struct.unpack_from("<2i", data, pos)
        pos += 8
        count = rows * width
        block = np.frombuffer(data, dtype="<f4", count=count,
                              offset=pos).reshape(rows, width)
        blocks.append(block.copy())
        pos += count * 4
    return blocks


def check_synthetic(lib: ctypes.CDLL) -> None:
    if lib.phlow_scoring_version() != 2:
        raise CheckError("ABI version is not 2")
    # Uniform logits: every target has logp = -log(vocab).
    logits = np.zeros((2, 8), dtype=np.float32)
    got = call_logprob(lib, logits, np.array([0, 7], dtype=np.int32))
    want = -math.log(8.0)
    if np.max(np.abs(got - want)) > 1e-5:
        raise CheckError(f"uniform case: got {got}, want {want}")
    # Handcrafted row: logsumexp([1,2,3,4]) and target 3.
    row = np.array([[1.0, 2.0, 3.0, 4.0]], dtype=np.float32)
    got = call_logprob(lib, row, np.array([3], dtype=np.int32))
    lse = math.log(sum(math.exp(v) for v in (1.0, 2.0, 3.0, 4.0)))
    if abs(float(got[0]) - (4.0 - lse)) > 1e-5:
        raise CheckError(f"handcrafted case: got {got[0]}, want {4.0 - lse}")
    # Advantages, handcrafted: groups [1,0] and [1,1,0].
    rewards = np.array([1, 0, 1, 1, 0], dtype=np.float32)
    offsets = np.array([0, 2, 5], dtype=np.int32)
    rp, _ = f32_array(rewards)
    xp, _ = i32_array(offsets)
    out = np.zeros(5, dtype=np.float32)
    op, _ = f32_array(out)
    status = lib.phlow_rloo_advantages(rp, xp, 2, op)
    if status != 0:
        raise CheckError(f"advantages call failed with status {status}")
    want_adv = np.array([1.0, -1.0, 0.5, 0.5, -1.0], dtype=np.float32)
    if np.max(np.abs(out - want_adv)) > 1e-6:
        raise CheckError(f"advantages case: got {out}, want {want_adv}")
    # Error paths: rows = 0, bad target, singleton group.
    lp, _ = f32_array(logits)
    tp, _ = i32_array(np.array([0, 7], dtype=np.int32))
    op2, _ = f32_array(np.zeros(2, dtype=np.float32))
    if lib.phlow_logprob_token_logps(lp, 0, 8, tp, op2) != 1:
        raise CheckError("rows=0 was not rejected with ERR_ARG")
    bad_tp, _ = i32_array(np.array([0, 8], dtype=np.int32))
    if lib.phlow_logprob_token_logps(lp, 2, 8, bad_tp, op2) != 1:
        raise CheckError("out-of-range target was not rejected")
    one = np.array([0, 1], dtype=np.int32)
    xp1, _ = i32_array(one)
    rp1, _ = f32_array(np.array([1.0], dtype=np.float32))
    op3, _ = f32_array(np.zeros(1, dtype=np.float32))
    if lib.phlow_rloo_advantages(rp1, xp1, 1, op3) != 1:
        raise CheckError("singleton group was not rejected")
    print("synthetic checks: PASS (uniform, handcrafted, advantages, 3 error paths)")


def check_manifest(lib: ctypes.CDLL, bin_path: Path, manifest_path: Path,
                   label: str) -> float:
    blocks = read_blocks(bin_path)
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    entries = manifest["entries"]
    if len(blocks) != len(entries):
        raise CheckError(f"{label}: block/entry count mismatch")
    worst_token = 0.0
    worst_mean = 0.0
    for block, entry in zip(blocks, entries):
        got = call_logprob(lib, block,
                           np.array(entry["targets"], dtype=np.int32))
        want = np.array(entry["token_logps"], dtype=np.float32)
        token_diff = float(np.max(np.abs(got - want))) if len(want) else 0.0
        mean_diff = abs(float(np.mean(got)) - entry["mean_logprob"])
        worst_token = max(worst_token, token_diff)
        worst_mean = max(worst_mean, mean_diff)
        note = ""
        key = (entry.get("task_id"), entry.get("variant"))
        if key in NF4_REFERENCE:
            note = (f" | vs NF4 ref {NF4_REFERENCE[key]:.5f}: "
                    f"|d|={abs(float(np.mean(got)) - NF4_REFERENCE[key]):.5f}")
        print(f"  {label} {entry.get('task_id', 'completion ' + str(entry.get('completion_index')))}"
              f"{('/' + entry['variant']) if 'variant' in entry else ''}: "
              f"mojo mean {float(np.mean(got)):.5f} vs control "
              f"{entry['mean_logprob']:.5f} (|d|={mean_diff:.2e}), "
              f"max token |d|={token_diff:.2e}{note}")
    if worst_token > TOKEN_DIFF_MAX:
        raise CheckError(f"{label}: token diff {worst_token} above {TOKEN_DIFF_MAX}")
    print(f"{label}: PASS (worst token |d|={worst_token:.2e}, "
          f"worst mean |d|={worst_mean:.2e})")
    return worst_token


def check_advantages_export(lib: ctypes.CDLL, groups_path: Path) -> None:
    doc = json.loads(groups_path.read_text(encoding="utf-8"))
    rewards: list[float] = []
    offsets = [0]
    want: list[float] = []
    for group in doc["groups"]:
        rewards.extend(group["rewards"])
        want.extend(group["advantages"])
        offsets.append(len(rewards))
    rp, _ = f32_array(np.array(rewards, dtype=np.float32))
    xp, _ = i32_array(np.array(offsets, dtype=np.int32))
    out = np.zeros(len(rewards), dtype=np.float32)
    op, _ = f32_array(out)
    status = lib.phlow_rloo_advantages(rp, xp, len(doc["groups"]), op)
    if status != 0:
        raise CheckError(f"export advantages call failed: status {status}")
    diff = float(np.max(np.abs(out - np.array(want, dtype=np.float64))))
    print(f"export advantages ({len(doc['groups'])} groups, {len(rewards)} "
          f"completions): max |d| vs trainlab f64 = {diff:.2e}")
    if diff > 1e-5:
        raise CheckError("export advantages diff above 1e-5")


def bench(lib: ctypes.CDLL, out_dir: Path) -> None:
    rng = np.random.default_rng(20261008)
    logits = rng.standard_normal((BENCH_ROWS, VOCAB), dtype=np.float32) * 4.0
    targets = rng.integers(0, VOCAB, size=BENCH_ROWS).astype(np.int32)
    bin_path = out_dir / "bench_logits.bin"
    if not bin_path.exists():
        with open(bin_path, "wb") as handle:
            handle.write(struct.pack("<2i", BENCH_ROWS, VOCAB))
            handle.write(logits.astype("<f4").tobytes())
            handle.write(targets.astype("<i4").tobytes())
        print(f"wrote {bin_path} (seeded synthetic bench batch)")
    lp, _ = f32_array(logits)
    tp, _ = i32_array(targets)
    total_ns = np.zeros(1, dtype=np.int64)
    ns_ptr = total_ns.ctypes.data_as(ctypes.POINTER(ctypes.c_int64))
    repeats = 50
    status = lib.phlow_logprob_bench_ns(lp, BENCH_ROWS, VOCAB, tp, repeats, ns_ptr)
    if status != 0:
        raise CheckError(f"bench call failed with status {status}")
    per_launch_us = float(total_ns[0]) / repeats / 1_000.0
    print(f"mojo bench: {repeats} launches of {BENCH_ROWS}x{VOCAB} f32 "
          f"= {per_launch_us:.1f} us/launch "
          f"({BENCH_ROWS * VOCAB * 4 / 1e6:.0f} MB read/launch)")
    torch_out = out_dir / "bench_torch_out.npy"
    if torch_out.exists():
        want = np.load(torch_out)
        got = call_logprob(lib, logits, targets)
        diff = float(np.max(np.abs(got - want)))
        print(f"bench-batch outputs vs torch: max |d| = {diff:.2e}")
        if diff > TOKEN_DIFF_MAX:
            raise CheckError("bench-batch diff above threshold")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lib", required=True)
    parser.add_argument("--out-dir", required=True)
    parser.add_argument("--groups", required=True)
    args = parser.parse_args()
    out_dir = Path(args.out_dir)
    lib = load_lib(Path(args.lib))
    check_synthetic(lib)
    check_manifest(lib, out_dir / "logits_parity.bin",
                   out_dir / "logits_parity.json", "parity")
    check_manifest(lib, out_dir / "logits_group.bin",
                   out_dir / "logits_group.json", "group8")
    check_advantages_export(lib, Path(args.groups))
    bench(lib, out_dir)
    print("ALL CTYPES CHECKS PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
