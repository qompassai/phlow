# MATE joint-attention coordination landed (crate + config flag + runtime gate + MCP tools)

Date: 2026-10-12 (UTC 08:00). Agent: pax (MATE implementation run,
commissioned by Owner 2026-10-12 00:28 PDT).
Source paper: "Grounded Joint-Attention Other-Play for Zero-Shot
Coordination", arXiv:2610.06025. Full design + evidence:
`~/workspace/mate-implementation/{mechanism.md,design.md,REPORT.md}`.

## What changed

- New crate `crates/phlow-attention`: saliency snapshot over workspace
  targets (canonical serialisation, SHA-256 frame hash), focus
  derivation (snapshot ranking verbatim, optional filter), alignment
  (same-frame only, Jaccard + top-shared), the coordination rule
  (parallel / serialize under the lexicographically smaller agent id /
  reject), and a bounded focus board (identity-bound publish,
  monotone-seq anti-replay, 64-agent cap). Added to workspace members.
- `crates/phlow-config`: `[attention] enabled` (default false),
  `AttentionConfig`, unknown keys rejected, parsed like `[agent]`.
- `crates/phlow-runtime`: new `attention` module — `AttentionGate`
  (publish own focus, decide against board partners, deterministic
  deferral) and `snapshot_from_workspace` (v1 signal: task/plan
  naming). `Runtime::run_inner` consults the gate after planning when
  the config enables it and a gate is installed; losing a contended
  top target ends the run with status `deferred` and the holder named.
  New report key `attention` records hash, focus, per-partner
  decisions.
- `crates/phlow-mcp`: four tools — `flow_attention_publish`,
  `flow_attention_focus`, `flow_attention_alignment`,
  `flow_attention_decide` — served from a board in the server process.
  Publish is bound to the transport identity (legacy `clientInfo.name`
  at initialize; modern `_meta` clientInfo): spoofed publishes are
  tool errors. `McpServer::set_attention_board` shares the board with
  the runtime gate.
- `crates/phlow-cli`: installs the gate (agent id `phlow`) at runtime
  construction; `serve` hands the gate's board to the MCP server.

## Deliberate divergence (needs Owner's awareness)

The MCP tool catalog is no longer byte-identical to Python
`flow serve`: it has 7 tools, not 3. The golden fixture
(`crates/phlow-mcp/tests/fixtures/mcp_golden.jsonl`) pair 5
(tools/list) was regenerated from the Rust server's actual output and
the divergence is documented in `crates/phlow-mcp/tests/golden.rs`.
Every other golden pair remains byte-identical to Python.

## Validation (primo, worktree phlow-mate-wt, base 9d764ca)

- `cargo test --workspace`: 2653 passed / 0 failed.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo fmt --all`: applied.
- Per crate: phlow-attention 36, phlow-config 56, phlow-mcp 51+3
  golden, phlow-runtime 102, phlow-cli 54.
- Viability harness (separate crate, `~/workspace/mate-implementation/
  harness`): card-game cross-play — convention pairs 0.145, no-signal
  0.199 (chance), grounded-attention pairs 1.000 over 5,000 XP
  episodes/condition; workspace sim conflict rate 0.558 → 0.098 with
  the protocol.
- Note: `crates/phlow-trainer-mojo`'s build script requires its pixi
  env (`.pixi/`, gitignored) inside the crate dir; fresh worktrees
  need it linked or `pixi install` run there before workspace builds.

## Not in this change

Periscope's vendored attention tools and the diver Lua client are
separate deliverables in `~/workspace/mate-implementation/` (periscope
tree at `~/workspace/periscope`, diver sketch sandbox-only, unmirrored
by design). Lumen: assessed PARTIAL with a prototype; no lumen code
changed.
