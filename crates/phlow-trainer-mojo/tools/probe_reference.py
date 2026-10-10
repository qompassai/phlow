#!/usr/bin/env python3
"""Reference logits for tools/probe_modular.py, computed with the
PyTorch sidecar stack (the production producer).

Runs under the sidecar venv (torch + transformers). Writes
{"top": [{id, token, logprob}, ...]} — the top-8 last-position
logprobs for probe_modular.py's fixed prompt — to the path given
as argv[1]. Kept separate because the pip `modular` venv carries
no torch: the two environments must not be merged.
"""

from __future__ import annotations

import glob
import json
import os
import sys

PROMPT = "def quicksort(arr):\n    if len(arr) <= 1:\n        return arr\n    pivot = arr[len(arr) // 2]\n    left = [x for x in arr if x < pivot]\n    middle = [x for x in arr if x == pivot]\n    right = [x for x in arr if x > pivot]\n    return "
TOP_N = 8


def main() -> int:
    import torch
    from transformers import AutoModelForCausalLM, AutoTokenizer

    hits = glob.glob(
        os.path.expanduser(
            "~/.cache/huggingface/hub/models--Qwen--Qwen2.5-Coder-7B-Instruct/snapshots/*"
        )
    )
    assert hits, "HF snapshot not found"
    snap = sorted(hits)[0]
    tok = AutoTokenizer.from_pretrained(snap)
    model = AutoModelForCausalLM.from_pretrained(snap, dtype=torch.bfloat16)
    model.eval()
    ids = tok(PROMPT, return_tensors="pt").input_ids
    with torch.no_grad():
        logits = model(ids).logits[0, -1].float()
    logps = torch.log_softmax(logits, dim=-1)
    top = torch.topk(logps, TOP_N)
    out = {
        "snapshot": snap,
        "top": [
            {"id": idx, "token": tok.decode([idx]), "logprob": value}
            for value, idx in zip(top.values.tolist(), top.indices.tolist())
        ],
    }
    with open(sys.argv[1], "w") as fh:
        json.dump(out, fh, indent=2)
    print(f"reference written to {sys.argv[1]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
