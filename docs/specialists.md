# Model specialists: running the local HuggingFace models by role

Phlow's agent loop has three roles — **planner**, **coder**, **reviewer** —
and each role can be bound to a different model. This document records how
the models Matt downloaded to `~/.local/share/models` (HuggingFace
safetensors, October 2026) are provisioned into the Ollama backend phlow
already talks to, which model is the specialist for which role and why,
and what the measured trials showed.

## The registry: `[specialists]` in config

`[models]` names the model per role, as before. A role may additionally
declare a *specialist* — the local directory its model was provisioned
from:

```toml
[ollama]
model = "hf-gemma-4-12b"              # default; also the fallback target

[models]
planner  = "hf-nemotron-3.5-lightning-30b"
coder    = "hf-qwen3-coder-30b"
reviewer = "hf-gemma-4-12b"

[specialists.planner]
source_dir = "/home/phaedrus/.local/share/models/nemotron-35-lightning-30b"

[specialists.coder]
source_dir = "/home/phaedrus/.local/share/models/qwen3-coder-30b"

[specialists.reviewer]
source_dir = "/home/phaedrus/.local/share/models/gemma-4-12b"
```

Semantics, enforced by `phlow-config` (schema) and `phlow-runtime`
(behavior):

- A declared specialist is **availability-checked once per run**: before
  the first specialist-bearing role call, the runtime fetches the
  backend's served-model list (`GET /api/tags`) and caches it.
- If a role's configured model is **not served**, the role falls back to
  `ollama.model` for that run. The run report records the substitution on
  the role entry: `"model"` (effective), `"model_configured"` (declared),
  `"model_fallback": "specialist_unavailable"`. Nothing falls back
  silently.
- A role **without** a `[specialists]` entry is never probed and never
  falls back — pre-specialist behavior is unchanged. A failed probe
  (backend down) also changes nothing: availability is unknown, so the
  configured model is attempted and fails the way it always has.
- Name matching follows Ollama's convention: a configured `name` matches
  a served `name:latest`.
- `/model <name>` still works exactly as before: it replaces the default
  and clears the `[models]` role overrides (which also retires the
  specialist bindings, since they annotate those overrides).

## Specialist mapping

| Model (HF repo) | Size on disk | phlow name | Role | Why |
|---|---|---|---|---|
| openai/gpt-oss-20b | 26 GB (MXFP4 MoE, 3.6B active) | `hf-gpt-oss-20b` | **planner** | Reasoning-first open-weight model; the low active-parameter count keeps planning latency sane on CPU-offload. |
| Qwen/Qwen3-Coder-30B-A3B-Instruct | 57 GB BF16 (MoE, 3B active) | `hf-qwen3-coder-30b` | **coder** (primary) | Purpose-built coding model; strongest tool-use of the set (see trials). |
| mistralai/Devstral-Small-2-24B (24B dense) | 49 GB BF16 | `hf-devstral-24b` | coder (alternate) | Trained for agentic software-engineering workflows; denser and slower per token than the MoE coder. |
| microsoft/phi-4 (14B dense) | 28 GB BF16 | `hf-phi-4` | **reviewer** | Reasoning/math-dense training suits critical review of a diff and a verification record. |
| ibm-granite/granite-4.2-8b | 17 GB BF16 | `hf-granite-4.2-8b` | default / fast fallback | Smallest and fastest of the set; the model every role can afford to fall back to. |
| google/gemma-4-12b-it | 23 GB BF16 | `hf-gemma-4-12b` | reviewer (alternate) | Instruction-tuned generalist with tool support; the alternate reviewer. |
| nvidia/NVIDIA-Nemotron-3.5-Lightning-30B-A3B-BF16 | 62 GB BF16 (MoE, 3B active) | `hf-nemotron-3.5-lightning-30b` | planner (alternate) | Fast reasoning MoE; alternate planner. |
| nvidia/NVIDIA-Nemotron-3-Super-120B-A12B-BF16 | 231 GB BF16 | — | heavyweight reviewer / oracle | **Not provisioned**: see hardware note. |

That table is the assignment by identity. The **binding that actually
runs today** differs, because of the tool-capability limit measured
below: planner = `hf-nemotron-3.5-lightning-30b`, coder =
`hf-qwen3-coder-30b`, reviewer = `hf-gemma-4-12b`, default/fallback =
`hf-gemma-4-12b`. gpt-oss (planner by identity), phi-4 (reviewer) and
devstral (coder alternate) cannot serve any role until Ollama accepts
tools for their templates, because phlow attaches tool schemas to every
role call. The identity mapping stands as the target once that blocker
clears (an explicit Modelfile `TEMPLATE` per model is the likely cure;
it was not attempted here because template fidelity for the harmony and
granite formats is its own risk).

### Hardware note (primo, as currently attached)

Primo has 62 GB RAM and an RTX 4070 Laptop (8 GB VRAM). Models are served
by Ollama from Q4_K_M GGUF conversions (gpt-oss keeps its native MXFP4
expert tensors), spilling from VRAM into RAM. The 120B is the exception:
its BF16 weights are 231 GB, and even a Q4_K_M conversion is ~70 GB —
more than the machine's total RAM before KV cache and OS. It is therefore
declared in configs only as a fallback *demonstration*, never a binding.
With the RTX A6000 eGPU (48 GB) attached, a Q4 120B becomes feasible
(48 GB VRAM + RAM spill) and this verdict should be revisited.

## Provisioning runbook (what was done, reproducibly)

1. Convert each HF directory to GGUF with llama.cpp's
   `convert_hf_to_gguf.py` (`--outtype f16`), then `llama-quantize` to
   `Q4_K_M`. gpt-oss is the exception: its expert tensors are natively
   MXFP4 and llama-quantize refuses to requantize them; the converted
   GGUF is used as-is (13.8 GB, native format).
2. Two downloads needed a *tokenizer_config patch on a copy* (originals
   untouched; symlinks + one patched file under `~/workspace/convert-src`):
   gemma-4-12b ships `extra_special_tokens` as a list, which the installed
   transformers rejects; devstral-24b declares
   `tokenizer_class = "TokenizersBackend"`, which it cannot resolve.
   qwen3-coder-30b arrived **weights-only** — its `config.json`,
   tokenizer files and safetensors index were fetched from the same HF
   repo to complete it (shard sizes verified against the index).
3. Register with Ollama (`~/workspace/phlow-trials/create-ollama-models.sh`):
   a Modelfile per model, `FROM <gguf>` + `PARAMETER num_ctx 16384`,
   created as `hf-<name>`. Start the daemon with `ollama serve`.
4. Bind roles in the workspace `config.toml` as above and run
   `phlow run --trusted "<task>"`.

### Known limitation: tool capability is per-model

Phlow sends tool schemas with role calls, and Ollama refuses chat
requests with tools for models whose chat template it does not recognize
as tool-capable (HTTP 400, "does not support tools"). Measured on the
converted set: `hf-qwen3-coder-30b`, `hf-gemma-4-12b` and
`hf-nemotron-3.5-lightning-30b` are tool-capable; `hf-granite-4.2-8b`,
`hf-phi-4`, `hf-gpt-oss-20b` and `hf-devstral-24b` are
**completion-only** in Ollama today. Consequences:

- The **coder** role must be bound to a tool-capable model
  (qwen3-coder-30b primarily; gemma-4-12b as alternate) — it cannot edit
  files otherwise.
- Planner/reviewer bindings to completion-only models only work while
  phlow sends them an empty tool list; with tool schemas attached the
  call is refused. Binding planner/reviewer to tool-capable models is
  the safe configuration until Ollama's template detection catches up
  or the models get explicit Modelfile templates.

## Trials

_Trial method:_ `~/workspace/phlow-trials/run_trial.py` runs one bounded
task per binding — implement `solve.py` (`median3`, `clamp`) so a fixed
`check_solution.py` passes — through the real `phlow run` loop
(planner → coder → host check → reviewer), plus a uniform raw-generation
probe per model (`probe_model.py`, 128 tokens, Ollama's own counters).

Raw generation probe (uniform prompt, 128-token cap, Ollama counters):

| Model | tok/s | Load |
|---|---|---|
| hf-nemotron-3.5-lightning-30b | 30.2 | 11.9 s |
| hf-granite-4.2-8b | 25.2 | 7.1 s |
| hf-gpt-oss-20b | 23.8 | 15.1 s |
| hf-qwen3-coder-30b | 21.2 | 10.3 s |
| hf-gemma-4-12b | 15.7 | 9.2 s |
| hf-phi-4 | 9.7 | 6.7 s |
| hf-devstral-24b | 5.0 | 10.2 s |

Loop trials (task: `solve.py` with `median3`/`clamp`; host check =
`check_solution.py` exit 0):

| Trial | planner | coder | reviewer | Result |
|---|---|---|---|---|
| T0 baseline (pre-specialist model) | qwen2.5-coder:7b | qwen2.5-coder:7b | qwen2.5-coder:7b | Loop completes mechanically; task fails — every role replies `PAUSE: I need your input…` and the PAUSE cascades. |
| T1 primary | nemotron-3.5-lightning | qwen3-coder-30b | gemma-4-12b | Planner plans ✓, coder writes a correct `solve.py` ✓, host check **passes both cycles** ✓. Run still unverified: reviewer's approval JSON is wrapped in ```` ```json ```` fences, which the (Python-faithful) verdict parser rejects. |
| T2 swap | gemma-4-12b | qwen3-coder-30b | nemotron-3.5-lightning | Planner ✓, coder ✓ (host check passes). Reviewer never returns a verdict — it keeps calling read tools until the iteration budget exhausts. |
| T3 fallback demo | nemotron-3.5-lightning | qwen3-coder-30b | declared nemotron-3-super-120b (unserved) | **Fallback fired live**: report records `model_configured: hf-nemotron-3-super-120b`, `model_fallback: "specialist_unavailable"`, effective model `hf-gemma-4-12b`. The fallback reviewer then errored on an over-long generation (737 s wall). |
| T4 capability probe | gpt-oss-20b | — | — | Immediate refusal: HTTP 400 "does not support tools" — the capability limit, measured directly. |
| T5 single-model | qwen3-coder-30b | qwen3-coder-30b | qwen3-coder-30b | Fails at the planner: qwen3-coder ignores the plan-in-text instruction and calls `file_write` (unavailable to planners) until the budget exhausts. It is an executor, not a planner. |

Findings, in order of leverage:

1. **The specialist mechanism works**: registry, per-role binding,
   availability probe, recorded fallback — proven by unit tests and by
   T3's live fallback record.
2. **Only three of the seven provisionable models can serve at all**
   today (tool capability, above).
3. **No tested binding completes a fully-verified run yet**, and the
   blockers are loop-discipline behaviors, not code correctness — the
   coder's `solve.py` passed the host check in every cycle it finished:
   the vendored prompts make small models PAUSE instead of acting
   (T0); planners that try to *execute* deadlock on the role-tool
   matrix unless the task explicitly says "plan in text only" (T5,
   and T1/T2 before the framing fix); qwen3-coder sometimes keeps
   polishing past done until its budget exhausts; gemma fences its
   reviewer JSON and nemotron's reviewer never verdicts.
4. Per-model role verdicts: **planner — nemotron-lightning** (plans
   cleanly, fastest big model); **coder — qwen3-coder-30b**
   (uncontested; correct code every finished attempt); **reviewer —
   gemma-4-12b, conditionally** (it does review and approve correctly;
   its verdict is lost to fence-wrapping in the strict parser — the
   single highest-leverage follow-up is a product decision on verdict
   parsing, deliberately not changed here because the Rust port mirrors
   the Python reference's strictness).
