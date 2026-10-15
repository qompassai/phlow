"""Torch-side like-for-like bench for the Mojo sampling kernel.

Runs under the PyTorch sidecar venv. The torch equivalent of the
composable sampling kernel on resident GPU tensors: temperature
scaling -> top-k restriction -> nucleus (top-p) filter over the
sorted distribution -> renormalize -> one multinomial draw per row.
Same resident batch as the scoring bench (bench_logits.bin,
128x151,936) and the same sampling parameters as the Mojo-side
`sample-bench` run it is compared against (T=0.8, top_k=50,
top_p=0.95). Timing only: the RNG streams differ by design, so no
output comparison is made here (parity lives in check_sampling.py).

GPU memory state is printed before and after: a co-resident process
holding the card invalidates timing (standing rule).
"""

from __future__ import annotations

import argparse
import struct
import sys
import time
from pathlib import Path

import numpy as np
import torch

REPEATS = 50
WARMUP = 5
TEMPERATURE = 0.8
TOP_K = 50
TOP_P = 0.95


def sample_once(x: torch.Tensor) -> torch.Tensor:
    probs = torch.softmax(x.float() / TEMPERATURE, dim=-1)
    topv, topi = torch.topk(probs, TOP_K, dim=-1)
    sorted_p, order = torch.sort(topv, descending=True, dim=-1)
    cum = torch.cumsum(sorted_p, dim=-1)
    keep = (cum - sorted_p) < TOP_P  # crossing candidate included
    filtered = torch.where(keep, sorted_p, torch.zeros_like(sorted_p))
    filtered = filtered / filtered.sum(dim=-1, keepdim=True)
    pick = torch.multinomial(filtered, 1).squeeze(1)
    return topi.gather(1, order.gather(1, pick.unsqueeze(1))).squeeze(1)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", required=True)
    args = parser.parse_args()
    bin_path = Path(args.out_dir) / "bench_logits.bin"
    data = bin_path.read_bytes()
    rows, vocab = struct.unpack_from("<2i", data, 0)
    logits = np.frombuffer(
        data, dtype="<f4", count=rows * vocab, offset=8
    ).reshape(rows, vocab).copy()
    print(f"batch: {rows}x{vocab}; torch {torch.__version__}; "
          f"gpu free before: {torch.cuda.mem_get_info()[0] / 2**20:.0f} MiB")
    x = torch.from_numpy(logits).cuda()
    out = sample_once(x)
    torch.cuda.synchronize()
    assert out.shape == (rows,)
    for _ in range(WARMUP):
        sample_once(x)
    torch.cuda.synchronize()
    started = time.perf_counter()
    for _ in range(REPEATS):
        sample_once(x)
    torch.cuda.synchronize()
    elapsed = time.perf_counter() - started
    print(f"torch sample bench: {REPEATS} launches = "
          f"{elapsed / REPEATS * 1e6:.1f} us/launch "
          f"(T={TEMPERATURE} top_k={TOP_K} top_p={TOP_P}); "
          f"peak vram {torch.cuda.max_memory_allocated() / 2**20:.0f} MiB; "
          f"gpu free after: {torch.cuda.mem_get_info()[0] / 2**20:.0f} MiB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
