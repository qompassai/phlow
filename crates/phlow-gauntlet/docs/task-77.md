# task-77: quantization behavior change

**Kind:** rust (adversarial) · **Status:** fail (open) · **Wave:** 76–80 · **Commits:** pending (wave 76-80)

## ELI5

Quantization is "making the model smaller so it runs faster, at the cost of precision" — FP16 is the careful full-precision version, INT8 is the compressed version, INT4 is the heavily compressed version. The design wants a validation pipeline that catches when the compression breaks the model's tool calls: FP16 must pass everything, INT8 must break exactly where float64-vs-float32 coercion changes an argument ("4 becomes 4.0"), INT4 must break harder. Phlow's real validation pipeline — shape parsing, tool-call normalization, argument JSON parsing, MCP schema validation — does reject malformed tool calls with named violations. But it validates SHAPES, not precisions: there is no model-weight quantization selection in phlow at all. Models are served externally by Ollama; `phlow_inference::kv_policy` is KV-cache policy (which layers get fp4/int8 precision in the cache), not model-loading precision control. The design's precision ladder — FP16 → INT8 → INT4 with degradation curves and INT8/INT4 gates — has no implementation.

## What this task attempts

- **Goal:** verify the quantized-inference pipeline — FP16 4/4 valid, INT8 3/4 (breaking exactly on the float64/float32 coercion), INT4 1/4, with a precision ladder and degradation gates — or document the gap.
- **Mechanism:** `src/tasks/task_77.rs` drives the REAL validation seam with simulated fixtures at each rate: fp16_pipeline_verifies (4/4 through `parse_chat_message` + `normalize_tool_calls` + argument JSON + `validate_arguments`), malformed_rejected_with_names (4 malformed calls, 4 named violations), int8_pipeline_breaks (3/4; the coercion case), int4_pipeline_collapses (1/4). The real `phlow_inference::kv_policy` is exercised to place it correctly as KV-cache policy.
- **Success criterion:** a precision ladder with measured degradation gates, or the gap documented as the finding (and banked for Matt as a product decision).
- **Non-goals:** inventing model-loading precision control on gauntlet authority (it is a product decision, not a bug fix).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the pipeline is real but precision-blind:

- **V1:** FP16 fixture — 4/4 schema-valid tool calls pass the full real pipeline (shape → normalization → argument JSON → MCP schema validation).
- **V2:** malformed tool calls are rejected with NAMED violations — missing function block, unknown tool, malformed argument JSON, schema violation — the loud-failure property the quantized fixtures rely on.
- **A1:** INT8 fixture — 3/4 valid, breaking exactly on the float64-vs-float32 argument coercion (the "4 becomes 4.0" analog). `kv_policy::recommend`/`validate` are shown to be KV-cache policy (fp4_recommend, INT8+KV8 ladders), not model-loading precision.
- **A2:** INT4 fixture — 1/4 valid; the pipeline collapses further (missing function blocks, wrong tool names, unparseable args).

## Full technical depth

The seam is three real pieces: `phlow_llm::parse_chat_message` (Ollama message shape), `phlow_runtime::normalize_tool_calls` (id normalization, empty-id filtering), and `phlow_mcp::validate_arguments` (JSON-schema validation against the tool schema) — with the argument-JSON parsing equivalent to the runtime's private path exercised directly. The fixture rates (FP16 4/4, INT8 3/4, INT4 1/4) are simulated, but every validation step is the real code path; the finding is that the pipeline answers "is this call well-formed?" and never "at what precision was this model loaded?". `phlow_inference::kv_policy` (the tempting near-miss) selects per-component KV-cache precision — it cannot select or report the served model's weight precision, which lives inside Ollama.

Banked for Matt (product decision, NOT auto-implemented on gauntlet authority): whether phlow should measure/verify Ollama-served precision (and record it in run records). That is a new product feature, not a bug fix; the gauntlet documents the gap and stops.

## Sources

- `~/workspace/repos/phlow/crates/phlow-llm/src/payload.rs` — `parse_chat_message`
- `~/workspace/repos/phlow-runtime/src/tool_calls.rs` — `normalize_tool_calls`
- `~/workspace/repos/phlow/crates/phlow-mcp/src/schema.rs` — `validate_arguments`
- `~/workspace/repos/phlow/crates/phlow-inference/src/kv_policy.rs` — KV-cache precision policy (not model precision)
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-77 design (Wave 76–80)
