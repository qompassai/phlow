# Code: Crate Tour

Think of the phlow codebase as the workshop from [Architecture](architecture.md),
but now we walk the floor and read the nameplates on the machines. The
Cargo workspace has **27 members**: 25 crates under `crates/`, plus
`kernels/mojo` and `workers/phlow-mojo-worker`. Every description
below comes from that crate's `Cargo.toml` `description` and its
`lib.rs` module docs — nothing invented.

(The [Architecture](architecture.md) chapter describes the older "14
port crates" state; this tour covers the workspace as it stands on
`main` now.)

## The pipeline: where work happens

- **phlow-runtime** — *"The safe agent runtime: bounded planner, coder,
  host verification, and reviewer."* The single authority shared by the
  CLI, the TUI, and the MCP server. Mirrors `flow/runtime.py`.
- **phlow-agent** — *"Bounded agent layer for the phlow runtime:
  conversation context, orchestrator, and SQLite FTS5 memory store."*
  Mirrors `flow/agent/`.
- **phlow-llm** — *"Ollama backend for the phlow agent runtime."*
  Ports `flow/llm/backend.py` and `flow/llm/prompts.py`: the request
  payloads and response shapes the model sees.
- **phlow-tools** — *"Tool surfaces for the phlow safe runtime."* A
  static, non-executable registry — the model can see the tools, but the
  tools themselves are fixed declarations, not something a prompt can
  extend.

## Config, workspace, and checks: what the operator controls

- **phlow-config** — *"Validated operator configuration for the phlow
  agent runtime."* Owns the TOML schema documented in [Configuration](configuration.md):
  fail-closed parsing, unknown-key rejection, validated ranges.
- **phlow-workspace** — *"Confined file operations for the phlow agent
  runtime."* A `Workspace` pins one root directory and mediates every
  file access the agent makes — the "channel bed" the fluid cannot
  leave.
- **phlow-checks** — *"Named, operator-approved checks for the phlow
  agent runtime."* A `CheckRunner` executes the exact argv from the
  config, no shell involved.

## Surfaces: how you and your editor reach the runtime

- **phlow-cli** — *"The `phlow` command-line interface: CLI, TUI, and
  MCP surfaces over one safe agent runtime."* The one binary: `run`,
  `serve`, `check`, `status`, `tui`. Owns the exit-code contract
  and the one-ASCII-escaped-JSON-line report format. See [The
  CLI](cli.md).
- **phlow-tui** — *"Terminal chat interface for the phlow agent
  runtime."* The line-based interactive loop with ratatui-rendered
  panels; all panel rendering is pure `(widget, area) -> buffer`, so
  tests use a test backend, never a real terminal. See [TUI
  Controls](tui-controls.md).
- **phlow-mcp** — *"Newline-delimited JSON-RPC (MCP) server for the
  phlow agent runtime."* A byte-faithful port of `flow/mcp.py` —
  this is what `phlow serve` runs. See [The MCP stdio
  contract](mcp.md).
- **phlow-editor** — *"Private Neovim socket bridge for the phlow agent
  runtime."* How phlow calls back into the editor's native tools. See
  [The private Neovim socket](editor.md).

## Safety: the walls of the workshop

- **phlow-approval** — *"Fail-closed approval and policy subsystem for
  phlow: versioned policy parsing, scope decisions, human-bound approval
  queue, permission deltas, and an append-only event ledger. In-memory
  only; no persistence."* Untrusted policies enter as JSON and are
  narrowed before they are believed.
- **phlow-seccomp** — *"Linux seccomp egress filter for phlow check
  children: socket(AF_INET/AF_INET6) and io_uring_setup fail with
  EPERM."* The crate's single `unsafe` block is the `pre_exec`
  registration — that is the entire unsafe surface.
- **phlow-json** — *"Checked JSON narrowing at untrusted boundaries for
  the phlow agent runtime."* JSON arrives from places we do not control
  (MCP frames, tool output); this crate decides what shape it is allowed
  to have before the rest of the code touches it.

## Sessions: many workshops at once

- **phlow-tuios** — *"Session multiplexing for phlow (terminal
  multiplexer)."* Concepts adapted from tuios: the agent state machine
  (`none/working/needs_input/idle/done/errored/unknown`), a bounded
  per-session message ring (`mailbox`), named hooks fired with
  environment-variable contexts, a line-delimited JSON control protocol
  over a Unix socket (socket directory held at mode `0700`), the
  session daemon, and declarative session `tape`s. Deliberate
  differences from tuios are documented in its `lib.rs`: JSON-only on
  the socket, hooks as argv arrays (never shell strings), tapes that
  script agent sessions rather than terminal keystrokes.

## Compute: the GPU wing

- **phlow-compute** — *"Tile-based compute abstractions for phlow:
  validated tile shapes, disjoint partitions, and launch-ownership
  discipline."* Concepts adapted from NVlabs cutile-rs.
- **phlow-compute-cuda** — *"CUDA kernel-target model for phlow:
  validated PTX modules, kernel descriptors, launch configs,
  compile-time specialization policies, and a bounded CudaBackend trait
  with a deterministic CPU simulator."* Concepts adapted from NVlabs
  cuda-oxide.
- **phlow-gpu-worker** — *"Bounded GPU work dispatcher: lifecycle state
  machine, generation-based cancellation (cuda-oxide host-runtime
  concepts, CPU-only)."*
- **kernels/mojo** and **workers/phlow-mojo-worker** — workspace members
  for the Mojo kernel experiment: kernels written in Mojo and the worker
  that runs them.

## Decision-making and verification: judging the work

- **phlow-council** — *"Candidate workflow for phlow: task contracts,
  candidate lineage, evidence, and council review with keep/revise/reject
  iteration."* Workflow methodology adapted from NVlabs kda.
- **phlow-gauntlet** — *"130-task agent-orchestration proving ground
  for phlow (130 implemented): task drivers, failure recorder, and
  report CLI."* **EXPERIMENTAL** — drives only test doubles and
  headless Neovim; enables no production behavior.
- **phlow-inference** — *"Inference-serving primitives for the phlow
  agent runtime, adapted from DeepSeek-V4.1-Flash (arXiv:2609.19969)."*
  Predates the port; a supporting crate, not part of the original
  fourteen.
- **phlow-system1** — *"Fast System 1 decision layer for phlow: a
  bounded client for the Jev system-one wire protocol (as served by
  laya-serve), a scripted mock backend, and a confidence-gated approval
  risk fast path that can only escalate to the existing human path."*
  The fast path can *escalate*, never *approve* — fail-closed by
  construction.

## Generation and self-improvement: the experimental wing

- **phlow-codegen** — *"Code generation surfaces for phlow: language
  profiles, fail-closed code validator, and bounded app generator."*
- **phlow-self-improve** — *"Self-improvement surfaces for phlow:
  feedback store and read-only skill store."*
- **phlow-experiment** — *"Staged supervised-self-improvement
  experiment scaffolding for phlow: control-plane types, lifecycle and
  promotion gates, evaluator skeletons, manifest validators, and
  evaluation records."* **EXPERIMENTAL — no behavior enabled.**

## How they fit together

One sentence per layer: `phlow-cli` parses your intent; `phlow-tui`,
`phlow-mcp`, and `phlow-editor` are the three doors in;
`phlow-config` validates the permission slip; `phlow-runtime` runs
the bounded loop over `phlow-agent` + `phlow-llm`; `phlow-tools`
and `phlow-workspace` are what the agent may touch; `phlow-checks`
verifies the result; `phlow-approval`, `phlow-seccomp`, and
`phlow-json` are the walls; `phlow-tuios` multiplies the workshops;
the compute, decision, and experiment crates extend what the workshop
can do — each behind its own explicit bounds.
