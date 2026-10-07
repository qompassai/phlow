# 2026-10-06 — Model specialists landed (trials: 3 of 8 models can serve)

Phlow now has a `[specialists]` config registry: a role bound in
`[models]` may declare `[specialists.<role>] source_dir = <HF dir>`;
the runtime probes the backend's served list once per run and falls
back to `ollama.model` when a declared specialist is not served,
recording `model_configured` + `model_fallback` on the role's report
entry. Roles without a declaration behave exactly as before. Full
design, mapping and results: `docs/specialists.md`.

Trial verdicts (bounded solve.py task, real loop):
- Tool capability in Ollama gates everything: only qwen3-coder-30b,
  gemma-4-12b and nemotron-3.5-lightning-30b accept tool calls;
  granite, phi-4, gpt-oss and devstral are refused (HTTP 400).
- Feasible binding: planner nemotron-lightning, coder qwen3-coder,
  reviewer gemma, default gemma. Coder output passed the host check
  in every finished attempt.
- No binding completes a verified run yet: gemma fences its reviewer
  JSON (strict parser, faithful to the Python reference), nemotron's
  reviewer tool-loops without verdicting, qwen3-coder will not stay
  in a text-only planner seat, and qwen2.5-coder PAUSE-cascades.
- nemotron-3-super-120B not provisioned: Q4 ≈ 70 GB > 62 GB RAM.
  Revisit with the A6000 eGPU attached.
- Live fallback proven (T3): declared 120B reviewer fell back to
  gemma with the substitution recorded in the report.

Environment note: phlow-cli's `run_without_ollama_fails_closed` test
fails while an `ollama serve` daemon is up (it asserts no server on
11434). Run the suite with the daemon stopped for a true total.
