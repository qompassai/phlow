# Benchmarks

Machine: primo — 13th Gen Intel i7-13620H, 16 threads, 62 GiB RAM, cargo 1.100.0 (2026-11-05).
Method: hand-rolled `std::time` harness in `benches/hot_paths.rs` (harness = false), named-constant workloads, warmup before timed loops, results printed as ns/op and ops/s.
Note: primo was under multi-lane load tonight (load average ~40+ at times), so these numbers are baselines, not peaks; expect substantial run-to-run variance on cold start in particular.

Placement: example and bench live in `crates/phlow-mcp`, phlow's MCP crate. Choice (also stated in the example's doc comment): phlow's MCP face via `phlow serve` is its cleanest public surface, so the example drives that instead of the internal runtime API. The representative read-only tool is `flow_status` (local health, configured models/checks, editor capabilities).

Workloads:
- `cold_start_discover_and_tools_list`: spawn `phlow serve`, send `server/discover` + `tools/list` (modern era 2026-07-28), 10 reps.
- `tool_call_flow_status`: one warm session, 20 warmup + 200 timed `tools/call flow_status {}`.

Results (2026-11-27):
- cold_start_discover_and_tools_list: 20,072,029 ns/op (~20.1 ms) over 10 reps
- tool_call_flow_status: 561,531 ns/op (~0.56 ms), 1,780.8 ops/s over 200 calls

Example: `cargo run -p phlow-mcp --example stdio_session` — discover (server name `flow`) → tools/list (7 tools) → one `flow_status` call, transcript summary printed.
