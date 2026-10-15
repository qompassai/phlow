"""Torch-side like-for-like bench for the Mojo logprob kernel.

Runs under the PyTorch sidecar venv. Reads the seeded synthetic bench
batch written by check_via_ctypes.py (bench_logits.bin: one block header,
f32 logits, then i32 targets), computes per-token logprobs with
log_softmax in f32 + gather on the GPU (the reference reduction, exactly
as the sidecar's completion_logprob_sum performs it), saves the outputs
for cross-checking, and times the reduction loop on resident tensors.

GPU memory state is printed before and after: a co-resident process
holding the card invalidates timing (standing rule), so the numbers are
only meaningful next to the recorded free-memory figure.
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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", required=True)
    args = parser.parse_args()
    out_dir = Path(args.out_dir)
    bin_path = out_dir / "bench_logits.bin"
    data = bin_path.read_bytes()
    rows, vocab = struct.unpack_from("<2i", data, 0)
    logits = np.frombuffer(data, dtype="<f4", count=rows * vocab,
                           offset=8).reshape(rows, vocab).copy()
    targets = np.frombuffer(data, dtype="<i4", count=rows,
                            offset=8 + rows * vocab * 4).copy()
    print(f"batch: {rows}x{vocab}; torch {torch.__version__}; "
          f"gpu free before: {torch.cuda.mem_get_info()[0] / 2**20:.0f} MiB")
    x = torch.from_numpy(logits).cuda()
    t = torch.from_numpy(targets.astype(np.int64)).cuda()

    def reduce_once() -> torch.Tensor:
        log_probs = torch.log_softmax(x.float(), dim=-1)
        return log_probs.gather(1, t.unsqueeze(1)).squeeze(1)

    out = reduce_once()
    torch.cuda.synchronize()
    out_path = out_dir / "bench_torch_out.npy"
    if not out_path.exists():
        np.save(out_path, out.cpu().numpy())
        print(f"wrote {out_path}")
    for _ in range(WARMUP):
        reduce_once()
    torch.cuda.synchronize()
    started = time.perf_counter()
    for _ in range(REPEATS):
        reduce_once()
    torch.cuda.synchronize()
    elapsed = time.perf_counter() - started
    print(f"torch bench: {REPEATS} launches = {elapsed / REPEATS * 1e6:.1f} "
          f"us/launch; peak vram {torch.cuda.max_memory_allocated() / 2**20:.0f} MiB; "
          f"gpu free after: {torch.cuda.mem_get_info()[0] / 2**20:.0f} MiB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
