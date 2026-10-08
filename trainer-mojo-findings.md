# Mojo kernel track — findings (2026-10-08)

Experimental fourth backend for phlow-trainlab, scoped to **kernels, not a
trainer**: per-token logprob over the vocab and RLOO advantage computation,
callable from Rust over a C ABI. Branch `pax/trainlab-mojo-20261008`, crate
`crates/phlow-trainer-mojo/`. All runs on primo (RTX 4070 Laptop, 8 GB).

<details>
<summary>Environment (Phase A)</summary>

- Pixi env in the crate resolves and locks: **MAX 26.5.0** (max + max-core,
  channel `https://conda.modular.com/max`), **Mojo 1.0.0 (ed45d567)**,
  Python 3.12.15, CUDA toolkit 13.4 (conda-forge), pixi 0.81.0.
- `kernels/probe_gpu.mojo`: vector-add kernel compiled and ran on the GPU,
  device named by the tool's own output, all 4096 elements host-verified.
- GPU residency on primo is shared with Ollama and changes minute to
  minute; every measurement below records the free VRAM at run time.
  Working sets were ≤ 250 MB, so no run was memory-starved.
</details>

<details>
<summary>Parity (Phase B) — contract bar: |Δ| ≤ 0.05 nats/token same-precision</summary>

Reference semantics copied exactly from the PyTorch sidecar's `train.py`
(prompt/completion tokenized separately, f32 `log_softmax`, mean over
completion tokens). Logits for both parity batches were produced by the
PyTorch control run (7B bf16 CPU); the kernel owns the log-softmax
reduction and the advantages. This track produces no adapter and no
trainer receipt — kernels move no weights.

- Parity batch (dev-increment-000/001 × reference + truncated completion),
  kernel vs torch on identical bf16 logits: worst per-token |Δ| **1.35e-04**,
  worst mean |Δ| **9.68e-05** — ~500× inside the contract bar.
- Real exported group (`rl-even-000`, 8 completions, 230 tokens): worst
  per-token |Δ| 1.49e-04, worst mean |Δ| 4.18e-05.
- RLOO advantages vs trainlab f64 over all 16 exported groups (128
  completions): max |Δ| **8.51e-09** — near-exact, as expected for pure
  arithmetic. Formula mirrors trainlab `group.rs` (leave-one-out mean,
  group size ≥ 2).
- The bf16 control's own gap to the NF4-quantized reference values in
  `parity.json` (0.030–0.102 nats) is attributable to the reference's NF4
  quantization — the same attribution the Candle/Burn tracks documented.
- Check threshold in the tools is 5e-4, not the kernel's accuracy limit:
  different f32 reduction orders over a 151,936-wide row land ~1e-4.
</details>

<details>
<summary>Like-for-like bench (Phase B/C) — scoring-only, identical resident batch</summary>

Batch: 128 rows × 151,936 vocab, f32, resident on GPU; 50 launches,
enqueue + sync per launch. Mojo kernel = single-pass online log-sum-exp.

| Run | µs/launch | GPU free at run |
|---|---|---|
| Mojo (ctypes) | 538.0 / 619.2 / 688.9 / 778.7 | 1526–7576 MiB |
| Mojo (Rust FFI driver) | 712.6 / 738.3 | ~1526 MiB |
| PyTorch `log_softmax` + gather | 1004.9 / 1316.1 | ~7443 MiB |

Every Mojo run beat every torch run (≈1.3–1.9×) despite torch enjoying
the freer GPU in its runs. Bench outputs agree to 1.91e-06 max |Δ|.
Torch bench peak VRAM 148 MiB; Rust driver process peak RSS 1.68 GB
(mostly the mapped MAX runtime). Never compared against the 72.5 s full
training step — that number includes the forward/backward this track
does not do.
</details>

<details>
<summary>Integration (Phase C)</summary>

- `build.rs` compiles `kernels/scoring.mojo` to `libphlow_scoring.so`
  via `pixi run --locked mojo build --emit shared-lib` and links it;
  `src/ffi.rs` is the crate's only unsafe (typed `ScoringError`, all
  shapes validated against crate bounds before any FFI call).
- Gates: `cargo build` clean; `cargo clippy --all-targets -D warnings`
  clean; 9 tests pass (4 unit + 5 FFI integration incl. adversarial
  shapes); the driver reproduces the Phase B numbers through the FFI
  (score parity worst 1.35e-04, group 1.49e-04, advantages 8.51e-09).
- Built binaries carry rpaths into the pixi env (the .so needs MAX
  runtime libraries), so they run where the env exists — documented in
  the crate docs; productionizing would vendor or statically pin them.
</details>

<details>
<summary>Docs/toolchain discrepancies (toolchain won every time)</summary>

1. GPU index globals live in `std.gpu` in 26.5, not `max.gpu` as the
   current docs say. `DeviceContext` is `max.gpu.host`; `barrier` is
   `max.gpu`; `AddressSpace` is `max.gpu.memory`; `stack_allocation` is
   `std.memory`; pointers are `Pointer` with `ptr[unsafe_offset=i]`.
2. `out` is a reserved argument-convention keyword in Mojo 1.0.
3. Pixi version specifiers are bare globs (`"26.5.*"`), not `==` specs.
4. C ABI exports: `@export def f(...) abi("C") -> T:` — `abi("C")`
   precedes the return arrow (pattern from MAX's shipped
   `kv_cache_ops.mojo`).
5. `unsafe_alloc`/`unsafe_free` are not exported from `std.memory` in
   26.5 despite the compiler's deprecation hint naming them.
6. `mojo format` cannot resolve its formatter in the conda package.
7. `mojo build` of plain executables hits a conda-ld link error
   (`dlerror@GLIBC_2.34`); `--emit shared-lib` and `mojo run` work.
8. Primo carries two pixis (system 0.81.0, stale 0.59.0 in ~/.pixi/bin
   and ~/.local/bin); cargo's build-script PATH can pick the stale one,
   which cannot read the v7 lockfile. build.rs resolves pixi
   deterministically and passes `--locked` (see its module docs).
</details>

<details>
<summary>MAX forward probe — the wall, precisely located</summary>

Question: can MAX's own Qwen2 pipeline own the forward pass (all-token
logits) so the kernels have a Mojo-native producer? Probe:
`crates/phlow-trainer-mojo/tools/probe_max.py`, time-boxed.

- The capability exists in the library: `Qwen2Model` is registered,
  takes `return_logits`, `ReturnLogits.ALL` exists, and `Qwen2Model`
  inherits `LogProbabilitiesMixin` (`compute_log_probabilities`) — the
  machinery behind MAX's OpenAI-compatible logprobs serving.
- But: importing the model class from the conda package needed six
  undeclared PyPI deps (requests, pydantic, msgspec, pillow, av,
  llguidance); assembly requires the serving factory layer
  (`TextGenerationPipeline` needs a pre-built pipeline_model,
  weight_adapters, tokenizer, memory_plan); and `max.entrypoints`
  does not exist in the conda distribution at all.
- Verdict: end-to-end MAX forward scoring was not landed in the time
  box. The wall is packaging/factory surface, not architecture or
  math. Production paths: adopt the pip `modular` distribution and
  drive its serving stack, or invest in executor-level assembly.
</details>

<details>
<summary>Verdict</summary>

- **On the scoring path, Mojo is competitive today**: parity ~500×
  inside the contract bar, advantages near-exact, reduction 1.3–1.9×
  faster than torch eager on identical resident data, callable from
  Rust over a small audited FFI.
- **The wall is the trainer, as predicted**: no PEFT/QLoRA ecosystem
  in Mojo, and the forward pass currently belongs to the PyTorch
  sidecar (MAX's forward is capable but factory-gated, above).
  Sampling primitives were not attempted; budget went to parity and
  integration, which are the contract-critical parts.
- Productionizing the scoring path needs: a logits producer decision
  (sidecar stays, or MAX via the pip distribution), vendored runtime
  libraries instead of pixi-env rpaths, and a GPU-residency policy on
  primo (the 8 GB card is shared with Ollama).
</details>
