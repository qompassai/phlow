# task-08: MCP stdio tool bridging

**Kind:** nvim-lua · **Status:** pass · **Wave:** 2 (task brief) ·
**Commits:** none (rules forbid committing)

## ELI5

MCP (Model Context Protocol) is how an AI agent talks to outside tools: the
agent's side (the *client*) spawns a *server* program and they exchange
JSON-RPC messages over the server's standard input/output, one JSON object
per line. Diver (Matt's Neovim config) has a real MCP client
(`lua/ai/mcp/client.lua`) and a harness adapter
(`lua/ai/harness/adapters/mcp.lua`) that wraps it so agent "runs" can use
MCP servers as tool providers. This task proves the whole chain works: a
harness run starts a server session through the *real* adapter, then the
*real* `tools/list` → `tools/call` path lists the server's tools and calls
one, and the result flows back into the run as evidence. Along the way the
task found two real bugs in diver's MCP code (read-only here — reported,
not fixed): the client encodes the empty `capabilities` table as a JSON
*array*, which strict servers reject; and the client's `env` option for
spawned servers is silently dropped. Both are pinned as evidence below.

## What this task attempts

- **Goal:** bridge diver's real MCP harness adapter to an MCP stdio server
  and prove the `tools/list` → `tools/call` round trip end to end, plus
  three adversarial cases (bad args, dying server, oversized frame).
- **Mechanism:** `crates/phlow-gauntlet/src/tasks/task_08.rs`
  (`run`/`run_scenario` → `phlow_gauntlet::run_nvim_lua_driver_with_env`
  with `GAUNTLET_SCENARIO`, `XDG_DATA_HOME`, `GAUNTLET_PHLOW_BIN`)
  spawns headless nvim on `lua/gauntlet/task_08.lua`, which drives
  `harness.run({adapter='mcp', extensions={mcp={server=name}}})` against
  diver's `lua/ai/harness/adapters/mcp.lua` → `lua/ai/mcp/{client,tools,
  registry}.lua`, with tool-call confirmation pre-approved through the real
  `ai.security` allowlist (persisted under `XDG_DATA_HOME`, inside the work
  dir). Prints one JSON verdict; always exits 0.
- **Success criterion:** `tools/list` returns the server's tools and
  `tools/call` with valid args returns the expected result through the real
  adapter/client/tools path, evidenced in the verdict.
- **Non-goals:** the `ai.rose.mcp` opportunistic transport (absent here);
  MCP resources/prompts/subscriptions; the security vet's hostile-metadata
  path (descriptions are benign); fixing the two diver bugs found (diver is
  read-only for this task).

## What happened

Iteration 1 (driver bug: asserted `Invalid params` while the client sent
`"arguments":[]` — see below), iteration 2 (driver used the registry
entry's `env` to steer the mock; the env never arrived — root-caused to a
diver client bug), iteration 3: **all four scenarios pass**, driver-direct
and via `cargo test -p phlow-gauntlet --test task_08` (4/4):

- `default`: the run against the **real** `phlow serve` binary fails the
  `initialize` handshake as predicted — `model.completed` carries
  `outcome=failed, error="initialize failed: Initialize requires
  capabilities and clientInfo"` (diver sends `"capabilities":[]`; phlow-mcp
  requires an object). The round trip then runs against the mock:
  `tools/list` → 3 tools (`gauntlet_add,gauntlet_blob,gauntlet_echo`);
  `tools/call gauntlet_add{a=40,b=2}` → `"42"`; the result is appended to
  the run as `diagnostic.observed`.
- `bad-args`: missing required arg → `Invalid params: missing required
  argument: 'text'`; wrong type → `argument 'a' must be an integer`;
  unknown tool → `Unknown tool: 'no_such_tool'`. All typed JSON-RPC -32602
  errors; the run stays `completed`, the driver stays alive.
- `server-dies`: mock `os._exit(1)` mid-call → `server exited with code 1`
  after **26 ms** — the client's process-exit path resolves the pending
  request; no hang.
- `oversize`: two rounds of a ~10 MiB single-line response → frames dropped
  past the client's 8 MiB `LINE_BYTES_MAX` cap → `request timed out after
  4000ms` (typed); Lua memory growth 2061 KB across both rounds (cap
  32768 KB); a follow-up `tools/list` still works (session survived, buffer
  reset).

Gates: `cargo fmt --check` clean, `cargo clippy --all-targets -D warnings`
zero warnings, `luac -p` clean on the driver.

## Where it went wrong

Two driver-side failures (fixed) and two diver-side bugs (reported, not
fixed — diver is read-only for this task):

1. **Stage:** driver, `bad-args`/`oversize` scenarios. **Symptom:** the mock
   answered `Invalid params: arguments must be an object` for calls made
   with `{}` args. **Root cause:** `vim.json.encode({})` → `[]`; the driver
   passed bare `{}` as tool arguments and diver's `tools/call` sent
   `"arguments":[]`. **Fix:** the driver passes `vim.empty_dict()` for
   empty argument objects (encodes as `{}`). This is the same empty-table
   encoding gotcha as bug A below, on the `tools/call` path.
2. **Stage:** driver, `server-dies`/`oversize` scenarios. **Symptom:** the
   mock never saw `GAUNTLET_MCP_MOCK` (always ran its default behavior),
   so the fatal call returned a normal result instead of dying.
   **Evidence:** mock startup log showed `SCENARIO='bad-args'` (the
   default) despite the registry entry carrying
   `env={GAUNTLET_MCP_MOCK='server-dies'}` (confirmed via
   `registry.get`). **Root cause (bug B below):** diver's
   `spawn_native` builds `env` as a *list* of `"K=V"` strings, but
   `vim.system`'s `env` option takes a *dict*; the list form is silently
   ignored. Bisected empirically: `env={'K=V'}` → child sees ABSENT,
   `env={K='V'}` → child sees the value. **Fix:** the driver steers the
   mock through `vim.env.GAUNTLET_MCP_MOCK` (inherited by spawned
   children — verified), and passes no registry env.

## The fix — what changed and why

- **Changed:** `lua/gauntlet/task_08.lua` (new driver), `src/tasks/task_08.rs`
  (new module: `run`/`run_scenario`, `GAUNTLET_PHLOW_BIN` resolution with
  fail-closed `TaskOutcome::Fail` when the binary is missing,
  `XDG_DATA_HOME` pointed at `<work>/task-08/xdg`),
  `tests/task_08.rs` (4 tests, 2V+2A), `docs/task-08.md` (this file).
- **Why:** the task brief mandates exactly these four files; the driver
  does the real work, the Rust module is a thin env-passing wrapper, the
  tests assert on verdict evidence strings.
- **Source:** diver `lua/ai/mcp/client.lua` (NDJSON framing, `LINE_BYTES_MAX
  = 8*1024*1024`, handshake shape), `lua/ai/mcp/tools.lua` (list/call +
  confirmation gate), `lua/ai/harness/adapters/mcp.lua` (adapter contract),
  `lua/ai/security/init.lua` (`confirm_tool_call` allowlist +
  headless default-deny); phlow `crates/phlow-mcp/src/{protocol,server}.rs`
  (NDJSON JSON-RPC 2.0, `initialize` version negotiation, strict
  `capabilities`/`clientInfo` validation); MCP spec (capabilities is an
  object); Neovim `vim.system` (empirically: `env` is a dict).
- **Validation agents:** the driver was run directly through headless nvim
  for all four scenarios before the Rust suite; `cargo test
  -p phlow-gauntlet --test task_08` 4/4 green; fmt/clippy/luac gates clean.
- **Adversarial agents:** the three adversarial scenarios *are* the
  red-team: invalid/unknown tools, mid-call process death, and a 10 MiB
  frame 25% past the client's documented 8 MiB cap. The mock itself was
  validated against hand-written JSON-RPC frames before the driver used it.
- **Citations:** see Sources.

## Full technical depth

**Wire comparison (real server vs diver client), checked first per the
brief.** Both speak newline-delimited JSON-RPC 2.0 over stdio with no
Content-Length framing (diver `client.lua` header; phlow-mcp `server.rs`
`serve`). Both implement `initialize` → `notifications/initialized` →
`tools/list` → `tools/call`. Version negotiation is lenient server-side:
diver's `protocolVersion: "2024-11-05"` negotiates to phlow-mcp's
`"2025-11-25"` (verified with hand-written frames: initialize/list/call
`flow_status` all answered correctly). The mismatch is in `initialize`
*params*: diver builds `capabilities = {}` and `vim.json.encode` renders
the empty Lua table as `[]`; phlow-mcp's `initialize()` requires
`capabilities` to be a JSON object (MCP spec: `ClientCapabilities`) and
answers `-32602 "Initialize requires capabilities and clientInfo"`. The
default scenario pins this: the harness run against `phlow serve` lands in
`failed` with exactly that error in the `model.completed` payload. **Bug A
(diver, reported):** `run_handshake` should send `vim.empty_dict()` for
capabilities. The same gotcha affects `tools/call` with empty args
(`"arguments":[]`) — the driver works around it with `vim.empty_dict()`.

**Bug B (diver, reported):** `McpServerEntry.env` never reaches the server.
`spawn_native` builds `env_list` as `{"K=V", ...}` but `vim.system` takes
`env` as a dict; the list form is silently dropped (bisected: list →
child sees ABSENT; dict → child sees the value). Every registry entry with
`env` is affected, not just this task's. The driver steers the mock via
`vim.env` instead (verified to propagate to `vim.system` children, which
merge the parent environment).

**Round trip (mock).** `harness.run({adapter='mcp',
extensions={mcp={server='gauntlet-mcp-mock'}}})` → adapter `M.start`
requires `ai.mcp.client`, spawns `/usr/bin/python3 <mock>` via
`vim.system` argv form (no shell), runs the `initialize` handshake
(15 s cap), and reports `model.completed outcome=completed` on the sink;
the supervisor finishes the run (`completed`) and closes the adapter
handle (`client.stop`). `ai.mcp.tools.list` lazily re-starts the session,
sends `tools/list`, vets descriptions through `ai.security.mcp_vet`
(benign here — verified no imperative-phrase / cross-tool-trigger hits),
and returns 3 tools. `ai.mcp.tools.call` consults `ai.security` first:
headless default-deny is bypassed by the driver's pre-approved allowlist
entries (the real persistent-allowlist mechanism, redirected into the work
dir via `XDG_DATA_HOME`). `gauntlet_add{a=40,b=2}` → `"42"`; the driver
appends `diagnostic.observed {kind='gauntlet_mcp_roundtrip', ...}` to the
run's sink.

**Adversarial paths.** `bad-args`: the mock validates strictly (unknown
args, missing required, wrong types → `-32602`); the client maps
`error.message` to the callback's `err` string; the run is untouched.
`server-dies`: `os._exit(1)` with the request pending → `vim.system`'s
on-exit → `teardown(session, 'server exited with code 1')` resolves every
pending request with that reason and kills the handle (26 ms observed, no
hang). `oversize`: a 10 MiB single line exceeds `LINE_BYTES_MAX`
(8 MiB) → `handle_line` drops it → the pending request is never resolved
→ the 4 s request timer fires with `request timed out after 4000ms`; the
session buffer is reset, a later `tools/list` succeeds, and Lua memory
grew 2061 KB across two rounds (cap 32768 KB) — bounded, not accumulating.

**Budgets:** every wait in the driver is deadline-bounded (`COMPLETE_WAIT_MS
= 30000`, `CALL_WAIT_MS = 20000`, oversize 4000 ms request timeout);
`run_nvim_lua_driver_with_env` enforces the Rust-side timeout by killing the
child. Nothing is written outside `GAUNTLET_WORK_DIR` (registry redirected
via `_test_set_data_dir`, allowlist via `XDG_DATA_HOME`, mock script in the
work dir).

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/mcp/client.lua` (framing,
  `LINE_BYTES_MAX`, handshake, pending-request timeouts, teardown);
  `~/workspace/repos/diver/lua/ai/mcp/tools.lua` (list/call, confirmation
  gate, vet hook); `~/workspace/repos/diver/lua/ai/harness/adapters/mcp.lua`
  (adapter contract); `~/workspace/repos/diver/lua/ai/mcp/registry.lua`
  (entry shape, `_test_set_data_dir`); `~/workspace/repos/diver/lua/ai/
  security/init.lua` (`confirm_tool_call`, allowlist, headless
  default-deny); `~/workspace/repos/phlow/crates/phlow-mcp/src/server.rs`
  (`initialize` strict validation, NDJSON serve loop);
  `~/workspace/repos/phlow/crates/phlow-mcp/src/protocol.rs` (tool specs,
  version negotiation); `~/workspace/repos/phlow/crates/phlow-cli/src/lib.rs`
  (`Commands::Serve` → stdio serve).
- Protocol: MCP specification — `initialize` params `capabilities` is a
  `ClientCapabilities` object; JSON-RPC 2.0 `-32602` invalid params.
- Secondary: Neovim `vim.system()` `env`-as-dict behavior was established
  empirically (dict propagates, list is ignored) after the docs lookup hung;
  treat as verified-by-experiment, not by doc citation.
