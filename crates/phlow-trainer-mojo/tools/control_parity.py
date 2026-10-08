"""Control runs for the Mojo kernel track (Phase B/C evidence).

Runs under the PyTorch sidecar venv (the reference backend's own
environment). Loads Qwen/Qwen2.5-Coder-7B-Instruct in bf16 on CPU -- the
no-quantization control configuration, matching the Candle/Burn tracks'
control methodology -- and:

1. Recomputes the fixed parity batch's mean per-token logprobs
   (semantics copied from the sidecar's train.py: prompt and completion
   tokenized separately with add_special_tokens=False, concatenated;
   completion scored on exactly its own tokens; log_softmax in f32).
2. Dumps the f32 prediction-row logits + target ids for the parity
   batch and for the signal group of a real trainlab groups export
   (default: rl group 8, rl-even-000), plus torch-side mean logprobs,
   so the Mojo reduction kernel can be compared on identical inputs.

All outputs refuse to overwrite existing files. Work is bounded: the
parity batch is 4 pairs; the group dump is capped at GROUP_COMPLETIONS_MAX
completions.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
from pathlib import Path
from typing import Any

import torch
from transformers import AutoModelForCausalLM, AutoTokenizer

MODEL_ID = "Qwen/Qwen2.5-Coder-7B-Instruct"
GROUP_COMPLETIONS_MAX = 64
FILE_BYTES_MAX = 64 * 1024 * 1024


class ControlError(RuntimeError):
    """Raised on invalid input or state; no output file is written."""


def load_json(path: Path) -> dict[str, Any]:
    if path.stat().st_size > FILE_BYTES_MAX:
        raise ControlError(f"file too large: {path}")
    doc = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(doc, dict):
        raise ControlError(f"expected a JSON object in {path}")
    return doc


def encode_pair(tokenizer: Any, prompt: str, completion: str) -> tuple[list[int], int]:
    """Tokenize prompt+completion separately; return (ids, completion_len)."""
    prompt_ids = tokenizer(prompt, add_special_tokens=False)["input_ids"]
    completion_ids = tokenizer(completion, add_special_tokens=False)["input_ids"]
    if not prompt_ids or not completion_ids:
        raise ControlError("prompt and completion must both tokenize non-empty")
    return prompt_ids + completion_ids, len(completion_ids)


def score_pair(
    model: Any, tokenizer: Any, prompt: str, completion: str
) -> dict[str, Any]:
    """Score one pair; return token logprobs + the f32 prediction logits."""
    input_ids, completion_len = encode_pair(tokenizer, prompt, completion)
    ids = torch.tensor([input_ids], dtype=torch.long)
    with torch.no_grad():
        logits = model(input_ids=ids).logits
    start = len(input_ids) - completion_len
    pred = logits[0, start - 1 : -1, :].float()
    targets = ids[0, start:]
    log_probs = torch.log_softmax(pred, dim=-1)
    token_logps = log_probs.gather(1, targets.unsqueeze(1)).squeeze(1)
    return {
        "prompt_len": len(input_ids) - completion_len,
        "completion_len": completion_len,
        "targets": [int(t) for t in targets.tolist()],
        "token_logps": [float(v) for v in token_logps.tolist()],
        "mean_logprob": float(token_logps.sum().item()) / completion_len,
        "logits": pred.contiguous(),
    }


def write_outputs(out_dir: Path, stem: str, scored: list[dict[str, Any]],
                  meta: list[dict[str, Any]]) -> None:
    """Write <stem>.bin (f32 logits rows) + <stem>.json (manifest)."""
    bin_path = out_dir / f"{stem}.bin"
    json_path = out_dir / f"{stem}.json"
    for path in (bin_path, json_path):
        if path.exists():
            raise ControlError(f"refusing to overwrite {path}")
    vocab = None
    with open(bin_path, "wb") as handle:
        for entry in scored:
            logits = entry.pop("logits")
            rows, width = logits.shape
            if vocab is None:
                vocab = width
            if width != vocab:
                raise ControlError("vocab width changed between pairs")
            handle.write(struct.pack("<2i", rows, width))
            handle.write(logits.numpy().astype("<f4").tobytes())
    manifest = {"vocab": vocab, "entries": meta}
    json_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"wrote {bin_path} and {json_path}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--parity", required=True, help="parity.json path")
    parser.add_argument("--groups", required=True, help="groups export path")
    parser.add_argument("--group-index", type=int, default=8,
                        help="1-based group number to dump (default: 8)")
    parser.add_argument("--out-dir", required=True)
    args = parser.parse_args()

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    print(f"loading {MODEL_ID} (bf16, CPU) ...", flush=True)
    tokenizer = AutoTokenizer.from_pretrained(MODEL_ID)
    model = AutoModelForCausalLM.from_pretrained(MODEL_ID, dtype=torch.bfloat16)
    model.config.use_cache = False
    model.eval()
    print("model loaded", flush=True)

    # --- Parity batch ---
    parity_doc = load_json(Path(args.parity))
    scored: list[dict[str, Any]] = []
    meta: list[dict[str, Any]] = []
    control_values: list[dict[str, Any]] = []
    for pair in parity_doc["pairs"]:
        entry = score_pair(model, tokenizer, pair["prompt"], pair["completion"])
        control_values.append({
            "task_id": pair["task_id"], "variant": pair["variant"],
            "token_count": entry["completion_len"],
            "mean_logprob": entry["mean_logprob"],
        })
        meta.append({
            "task_id": pair["task_id"], "variant": pair["variant"],
            "prompt_len": entry["prompt_len"],
            "completion_len": entry["completion_len"],
            "targets": entry["targets"], "token_logps": entry["token_logps"],
            "mean_logprob": entry["mean_logprob"],
        })
        scored.append(entry)
        print(f"parity {pair['task_id']}/{pair['variant']}: "
              f"{entry['mean_logprob']:.5f}", flush=True)
    write_outputs(out_dir, "logits_parity", scored, meta)
    control_path = out_dir / "parity_control_bf16.json"
    if control_path.exists():
        raise ControlError(f"refusing to overwrite {control_path}")
    control_path.write_text(json.dumps(
        {"model": MODEL_ID, "config": "bf16 weights, CPU, log_softmax f32",
         "values": control_values}, indent=2), encoding="utf-8")

    # --- Real export group ---
    groups_doc = load_json(Path(args.groups))
    groups = groups_doc["groups"]
    if not 1 <= args.group_index <= len(groups):
        raise ControlError("group index out of range")
    group = groups[args.group_index - 1]
    completions = group["completions"]
    if len(completions) > GROUP_COMPLETIONS_MAX:
        raise ControlError("group exceeds completions cap")
    scored = []
    meta = []
    for index, completion in enumerate(completions):
        entry = score_pair(model, tokenizer, group["prompt"], completion)
        meta.append({
            "completion_index": index,
            "prompt_len": entry["prompt_len"],
            "completion_len": entry["completion_len"],
            "targets": entry["targets"], "token_logps": entry["token_logps"],
            "mean_logprob": entry["mean_logprob"],
            "reward": group["rewards"][index],
            "advantage": group["advantages"][index],
        })
        scored.append(entry)
        print(f"group {group['group']} completion {index}: "
              f"{entry['completion_len']} tok, mean logp "
              f"{entry['mean_logprob']:.5f}", flush=True)
    write_outputs(out_dir, "logits_group", scored, meta)
    print("done")
    return 0


if __name__ == "__main__":
    sys.exit(main())
