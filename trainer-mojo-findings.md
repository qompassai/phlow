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

# Productization pass (2026-10-09)

The three blockers named in the verdict above are resolved or
documented below. Branch `pax/trainlab-mojo-20261008`, local
commits only.

<details>
<summary>Logits producer decision — sidecar ships; pip modular probed, walled on tail-logprob accuracy</summary>

- **Shipped:** the PyTorch sidecar stays the logits producer /
  trainer (per the trainer-backend verdict). The Mojo crate is the
  accelerated scoring + sampling backend behind the trainlab
  contract; trainlab's default backend remains PyTorch, and the
  Mojo backend is invoked explicitly through this crate.
- **Bounded shot at pip `modular` (26.6.0 / max 26.6.0 / mojo
  1.1.0, isolated venv `~/trainer-mojo-modular-probe`):**
  `tools/probe_modular.py` + `tools/probe_reference.py`.
  - Install: clean (`pip install modular`).
  - `max serve` runs Qwen2.5-Coder-7B end-to-end on CPU with
    `--quantization-encoding float32` (~30 s to ready). bf16 is
    rejected on CPU ("encoding 'bfloat16' is not compatible with
    the selected device type 'cpu'"); the 15 GB bf16 checkpoint
    cannot fit the 8 GB GPU. `logprobs` is capped at 7.
  - Accuracy: on a 5-token prompt, served top-7 logprobs agree
    with transformers to ≤0.008 nats (bf16 and f32 references).
    On a 73-token code prompt, top-1 and the top-7 set agree but
    tail logprobs deviate up to ~2.6 nats (MAX systematically
    flatter), with or without a trailing space in the prompt.
  - Verdict: **not wired** — ~50× outside the contract's parity
    bar (5e-2 nats/token). A documented wall; the mechanism
    (serve-side logprob computation on longer prompts) was not
    diagnosed further within the budget.
</details>

<details>
<summary>Runtime deployment — vendored Mojo runtime, no pixi rpath in the shipped artifact</summary>

`build.rs` copies the six runtime libraries the kernel `.so`
needs (libKGENCompilerRTShared, libAsyncRTMojoBindings,
libMSupportGlobals, libAsyncRTRuntimeGlobals, libstdc++.so.6,
libgcc_s.so.1) next to it and rewrites the kernel's rpath to
`$ORIGIN`; binaries link the build-dir copy in dev and
`$ORIGIN/../lib` for the installed layout. The pixi env rpath is
gone. `scripts/install-runtime.sh` installs `bin/` + `lib/` to a
prefix (default `~/.local/share/phlow-trainlab-mojo`).

Proof (primo, 2026-10-09): with the crate's `.pixi` renamed away
and the build OUT_DIR hidden, the installed binary under `env -i`
resolves every library from its own `lib/` (`ldd` shows
`bin/../lib/...`) and completes the production `run` below with
identical numbers.
</details>

<details>
<summary>GPU-residency policy — fail closed on the shared 8 GB card</summary>

`GPU_FREE_MIB_MIN_DEFAULT = 1024` MiB (named constant in
`gpu_policy.rs`; working set measured ≤ ~250 MiB). Before any
allocation the backend queries `nvidia-smi`; a missing tool,
unparseable output, an ABI mismatch, or free VRAM below the
threshold makes the Mojo path unavailable with a structured
reason. With fallback allowed (the default) the run proceeds on
the pure-Rust reference path and the scoring receipt records
`scoring_path = "reference"` + the reason + the observed free
VRAM; with `--no-fallback` the run errors
(`mojo backend unavailable: GPU free VRAM 7712 MiB below required
999999 MiB (total 8188 MiB)` — the forced-threshold proof).
Kernel/shape errors never fall back.
</details>

<details>
<summary>Production surface — backend dispatch, scoring receipt, end-to-end run</summary>

- Surface: `backend::{score_blocks, rloo_advantages_backend,
  sample, availability, run_scoring}`; trainlab stays
  backend-agnostic (Candle/Burn pattern) — this crate depends on
  `phlow-trainlab` behind the default `trainlab-contract`
  feature and verifies the export↔receipt tie (`run_id` +
  `config_sha256`, whole export hashed into the receipt).
- Receipt: `phlow.trainer-mojo.scoring-receipt/v1` — backend id,
  kernel ABI version + version label, scoring path, fallback
  reason, GPU figures, per-group mean logprobs + recomputed
  advantages, optional sampler record (temperature, top_k,
  top_p, seed, draw_index). Written atomically, refuse-overwrite.
- CLI: `phlow-trainer-mojo run|sample|sample-bench|backend-info`
  alongside the original modes.
- End-to-end (rl run `rl-qwen25-coder-7b-t10-20261008`, group 8,
  8 completions, fixtures now in `tests/fixtures/`): path=mojo,
  worst token |Δ| 1.49e-4 (bound 5e-4), worst mean |Δ| 4.18e-5,
  worst advantage |Δ| 8.51e-9 (bound 1e-5), sampler draw recorded
  (T=0.8, k=50, p=0.95, seed 42). GPU free at run: 7712/8188 MiB.
</details>

<details>
<summary>Sampling primitives — one composable kernel (ABI v2), parity + bench</summary>

`phlow_sample_tokens` / `phlow_sample_bench_ns`: temperature 0 =
greedy argmax (ties → lowest index); otherwise one composable
pass — temperature scaling, top-k universe restriction, top-p
nucleus prefix (smallest prefix reaching top_p, crossing
candidate included), draw renormalized over the prefix mass;
unrestricted draws walk the full-vocab CDF in index order.
Extraction is bounded at 2048 candidates; if the nucleus cannot
close by the cap with no top_k bound, the kernel falls back to
the exact CDF walk (documented bound, count == vocab signals
it). Uniforms: SplitMix64 on `seed ^ ((row+1)·GOLDEN) ^
(draw·PHI_M1)`, one step, top 24 bits / 2^24 — bit-identical in
the kernel, the Rust reference, and the Python harness, so a run
reproduces from its receipt (seed + draw_index recorded).

Parity (`tools/check_sampling.py`, all passed): greedy exact vs
torch argmax on bench-batch and group rows; candidate sets and
tokens exact vs the semantics reference for 7 parameter combos
(incl. the CDF-fallback branch); 4096 seeded draws, max
|freq − p| = 0.0053 / 0.0048 (bound 0.025); determinism across
repeated calls and a fresh library load. The Rust suite also
proves kernel == reference exactly on fixed batches
(`v_sample_matches_reference_batch`).

Bench (resident 128×151,936, T=0.8/k=50/p=0.95, 7712 MiB free):
Mojo 4811 µs/launch vs PyTorch equivalent (softmax → top-k →
nucleus filter → multinomial) 8959 µs/launch — ~1.8×.
Scoring-only class comparison, as with the logprob bench.
</details>

<details>
<summary>Gates</summary>

- `cargo test -p phlow-trainer-mojo`: 22 unit + 6 backend
  integration + 10 FFI integration = 38 passed, 0 failed
  (also green `--no-default-features`: 19 + 10). GPU tests are
  serialized per test file — parallel device contexts in one
  process stall on primo (observed; mutex documented in the
  test files).
- `cargo test -p phlow-trainlab`: 57 passed, 0 failed.
- `cargo clippy -p phlow-trainer-mojo --all-targets -- -D
  warnings`: clean, both feature configurations.
- `cargo fmt -p phlow-trainer-mojo -- --check`: clean.
- Python harnesses: `check_sampling.py` ALL PASSED;
  `check_via_ctypes.py` ABI assertion updated 1 → 2.
</details>
