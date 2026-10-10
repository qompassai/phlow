#!/usr/bin/env python3
"""Bounded probe: can the pip `modular` distribution serve Qwen2
logits end-to-end as an OPTIONAL logits producer for trainlab?

Method (Tiger Style: explicit stages, bounded waits, evidence out):
  A. Reference — transformers (installed in the same venv) computes
     last-position logits for a fixed prompt on CPU/bf16; the top-8
     (token, logprob) pairs are the control.
  B. MAX serve — `max serve --devices=cpu` on the same HF snapshot;
     POST /v1/completions with logprobs=8, temperature=0; compare
     the served top logprobs against the control.

Verdict: exit 0 = producer proven (agreement: same top-1 token and
|dlogprob| <= 0.15 on the shared top tokens); exit 3 = walled, with
the stage and error recorded. Output: one JSON object on stdout.

Run with the probe venv:  ~/trainer-mojo-modular-probe/venv/bin/python
    tools/probe_modular.py
"""

from __future__ import annotations

import glob
import json
import os
import signal
import subprocess
import sys
import time
import urllib.request

PROMPT = "def quicksort(arr):\n    if len(arr) <= 1:\n        return arr\n    pivot = arr[len(arr) // 2]\n    left = [x for x in arr if x < pivot]\n    middle = [x for x in arr if x == pivot]\n    right = [x for x in arr if x > pivot]\n    return "
PORT = 8011
SERVE_WAIT_S_MAX = 900
TOP_N = 8
# MAX serve caps the completions `logprobs` parameter at 7
# (HTTP 400: "`logprobs` must be in [0, 7]"); the comparison uses
# the served top-7 against the reference top-8.
SERVE_TOP_N = 7
LOGPROB_TOL = 0.15


def snapshot_dir() -> str:
    hits = glob.glob(
        os.path.expanduser(
            "~/.cache/huggingface/hub/models--Qwen--Qwen2.5-Coder-7B-Instruct/snapshots/*"
        )
    )
    assert hits, "HF snapshot for Qwen2.5-Coder-7B-Instruct not found"
    return sorted(hits)[0]


def reference_top(snap: str) -> list[dict]:
    # The pip modular venv carries no torch; the reference comes
    # from tools/probe_reference.py run under the sidecar venv
    # (the environments must not be merged). Its output path is
    # passed as argv[1].
    if len(sys.argv) < 2:
        raise RuntimeError(
            "usage: probe_modular.py <reference.json> "
            "(produce it with tools/probe_reference.py under the "
            "sidecar venv)"
        )
    with open(sys.argv[1]) as fh:
        data = json.load(fh)
    assert data["snapshot"] == snap, "reference snapshot mismatch"
    return data["top"]


def http_json(url: str, payload: dict | None, timeout: float) -> dict:
    data = None if payload is None else json.dumps(payload).encode()
    req = urllib.request.Request(
        url, data=data, headers={"Content-Type": "application/json"}
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return json.loads(resp.read().decode())
    except urllib.error.HTTPError as error:
        body = error.read().decode(errors="replace")[:500]
        raise RuntimeError(f"HTTP {error.code} from {url}: {body}") from error


def serve_top(snap: str, evidence: dict) -> list[dict]:
    venv_bin = os.path.dirname(sys.executable)
    # bf16 (the checkpoint's encoding) is rejected on CPU by MAX
    # 26.6 serve ("encoding 'bfloat16' is not compatible with the
    # selected device type 'cpu'"); float32 is the offered CPU
    # encoding. GPU is not an option here: the bf16 checkpoint is
    # 15 GB and the card has 8 GB.
    cmd = [
        os.path.join(venv_bin, "max"),
        "serve",
        "--model-path",
        snap,
        "--devices=cpu",
        "--quantization-encoding",
        "float32",
        "--port",
        str(PORT),
    ]
    evidence["serve_cmd"] = " ".join(cmd)
    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        start_new_session=True,
    )
    try:
        deadline = time.time() + SERVE_WAIT_S_MAX
        model_name = None
        while time.time() < deadline:
            if proc.poll() is not None:
                tail = ""
                if proc.stdout is not None:
                    tail = proc.stdout.read()[-4000:]
                raise RuntimeError(f"max serve exited early: {tail}")
            try:
                models = http_json(
                    f"http://127.0.0.1:{PORT}/v1/models", None, 5
                )
                if models.get("data"):
                    model_name = models["data"][0]["id"]
                    break
            except Exception:
                time.sleep(10)
        assert model_name, f"server not ready within {SERVE_WAIT_S_MAX}s"
        evidence["served_model"] = model_name
        resp = http_json(
            f"http://127.0.0.1:{PORT}/v1/completions",
            {
                "model": model_name,
                "prompt": PROMPT,
                "max_tokens": 1,
                "temperature": 0,
                "logprobs": SERVE_TOP_N,
            },
            300,
        )
        top = resp["choices"][0]["logprobs"]["top_logprobs"][0]
        return [
            {"token": token, "logprob": value} for token, value in top.items()
        ]
    finally:
        try:
            os.killpg(proc.pid, signal.SIGTERM)
            proc.wait(timeout=30)
        except Exception:
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except Exception:
                pass


def main() -> int:
    evidence: dict = {"prompt_tokens_note": "fixed code prompt, see source"}
    snap = snapshot_dir()
    evidence["snapshot"] = snap
    try:
        ref = reference_top(snap)
    except Exception as error:  # reference itself failed: cannot judge
        evidence["verdict"] = "reference_failed"
        evidence["error"] = repr(error)
        print(json.dumps(evidence, indent=2))
        return 3
    evidence["reference_top"] = ref
    try:
        served = serve_top(snap, evidence)
    except Exception as error:
        evidence["verdict"] = "walled"
        evidence["error"] = repr(error)[:2000]
        print(json.dumps(evidence, indent=2))
        return 3
    evidence["served_top"] = served
    ref_by_token = {entry["token"]: entry["logprob"] for entry in ref}
    shared = [
        (token, ref_by_token[token], value)
        for token, value in ((s["token"], s["logprob"]) for s in served)
        if token in ref_by_token
    ]
    worst = max((abs(a - b) for _, a, b in shared), default=999.0)
    top1_match = bool(served) and served[0]["token"] == ref[0]["token"]
    evidence["shared_top_tokens"] = len(shared)
    evidence["worst_shared_dlogprob"] = worst
    evidence["top1_match"] = top1_match
    proven = top1_match and len(shared) >= 4 and worst <= LOGPROB_TOL
    evidence["verdict"] = "proven" if proven else "logits_disagree"
    print(json.dumps(evidence, indent=2))
    return 0 if proven else 3


if __name__ == "__main__":
    raise SystemExit(main())
