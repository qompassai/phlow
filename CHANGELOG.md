# Changelog

All notable changes to phlow are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); dates are UTC.

## [Unreleased]

### Added

- Staged supervised-self-improvement experiment scaffolding: new workspace
  member `crates/phlow-experiment` (`phlow-experiment` 0.1.0, edition 2024,
  `#![forbid(unsafe_code)]`, no new external dependencies — only `toml 0.8`,
  `serde`, `serde_json`, all already in `Cargo.lock`). Types, manifests,
  fixtures, skeletons, and docs only: control-plane types (`NodeState` as one
  exhaustive enum, eight `WorkerRole`s with no production write for any role,
  `CapabilitySet` with strict-subset delegation, `SchedulerNode` with all
  plan-required fields, `SchedulerLimits`, and a pure-logic `Scheduler`
  enforcing bounded admission, generation-checked at-most-once publication,
  and terminal cancellation); promotion pipeline (`Lifecycle` transitions per
  the plan's diagram, opaque `HumanApproval` constructible only from a
  shape-validated operator record with no model-output constructor path,
  `ImprovementProposal`, protected-surface policy, and a fail-closed
  `PromotionGate`); evaluator skeleton (`EvalStage` order, fail-closed
  `BudgetTracker`, `EvidenceBundle::is_complete()`); `EvaluationRecord`
  matching the plan's JSON schema with `verified`/`eligible`/`human_approved`
  starting false and movable only on complete evidence or a real approval
  token; `toml::Value` + explicit-control-flow validators (no derive) for the
  four shipped manifests (`manifests/suites.toml`, `languages.toml` with
  tiers A–E, `budgets.toml` with the initial experimental limits,
  `promotion.toml` with the 100%-safety / 10%-latency / 20%-cost / rollback-
  rehearsal thresholds) and the task-manifest contract. Data: `evals/`
  (public/holdout/safety/regressions split contracts; holdout and safety are
  not candidate-writable, safety is immutable) with one example task manifest
  each for public (`rust-cli-parse-001`) and safety (`prompt-injection-001`);
  `fixtures/` (apps with one trivial inert fixture, attacks with one clearly
  labeled INERT injection-attempt fixture, mcp, failures). Docs:
  `docs/experiments/PHLOW_EXPERIMENTAL_SELF_IMPROVEMENT.md` (plan backlog
  item 1). Hard constraints enforced by types: no autonomous
  self-modification, no self-approval/self-promotion, promotion gates stay
  human, read-only default, fail-closed on missing evidence/timeout/budget
  exhaustion. 48 tests, exactly 24 validation / 24 adversarial. No behavior
  was enabled: no runtime concurrency, no scheduler execution, no
  self-editing, `PromptEvolver::evolve()` untouched and still disabled, no
  changes to existing crates or CI. Not yet validated with cargo in this
  environment; gates run on primo.

- Experimental Mojo worker support, re-expressed as Rust-side contracts in
  Tiger Style Rust (two new workspace members, `#![forbid(unsafe_code)]`, no
  new external dependencies). Mojo cannot compile in this workspace, so both
  crates are contracts + deterministic test doubles:
  `kernels/mojo` (`phlow-mojo-kernels`: `KernelName` validation, kernel
  descriptors + registry, `LaunchConfig` geometry validated against
  `DeviceLimits` with checked arithmetic, `LaunchBudget` planning,
  `KernelExecutor` boundary with `launch` + `synchronize` mirroring Mojo's
  `enqueue_function` + `DeviceContext.synchronize()`) and
  `workers/phlow-mojo-worker` (`phlow-mojo-worker`: lifecycle state machine,
  bounded task intake with reject-on-full, task ids, generation tokens,
  in-flight tracking, bounded result queue, cancellation policy, `MojoWorker`
  trait (`init`, `poll`, `cancel`, `shutdown`) as the boundary a real Mojo
  implementation plugs into). Each crate's `simulated` feature (default on)
  provides an in-process test double that performs no GPU work and compiles
  no Mojo — never a substitute for a real device executor or Mojo worker.
  Mojo APIs were checked against Modular's Mojo GitHub and
  docs.modular.com/mojo (concepts only, no Mojo code in-tree). 42 tests,
  exactly 21 validation / 21 adversarial. Per-crate design records in
  `kernels/mojo/docs/decisions.md` and
  `workers/phlow-mojo-worker/docs/decisions.md`. `docs/ARCHITECTURE.md`
  updated (22 crates, Layer 0 placement, crate table, Mojo landing notes).

- NVlabs GPU-compute concepts, re-expressed in Tiger Style Rust as four new
  CPU-only supporting crates with `#![forbid(unsafe_code)]` and no CUDA
  dependency (upstream concepts only — no verbatim ports; kda is NOASSERTION,
  so only its research → implement → verify/profile → iterate methodology was
  adapted, with no kda docs/prose/prompts/skill text copied):
  `phlow-compute` (cutile-rs concepts: tile shapes, exact partitions, disjoint
  origins, exclusive-output/shared-input launch ownership, deterministic CPU
  `TileExecutor`), `phlow-compute-cuda` (cuda-oxide concepts: `ComputeArch`
  sm_80/90/100, structural PTX validation, kernel descriptors, launch-config
  validation, specialization `KernelPolicy`, bounded `CudaBackend` trait with a
  deterministic CPU `SimulatedBackend`; original PTX/descriptor fixtures under
  `kernels/`), `phlow-gpu-worker` (one-job-at-a-time Idle/Busy/Stopped
  lifecycle dispatcher with job ids and generation-based cancellation),
  `phlow-council` (domain-free candidate workflow: `TaskContract`, candidate
  lineage, per-candidate `Evidence`, keep/revise/reject `CouncilReview`; keep
  promotes only verified candidates). Upstream: cuda-oxide
  (github.com/nvlabs/cuda-oxide, Apache-2.0), cutile-rs
  (github.com/nvlabs/cutile-rs, Apache-2.0), kda (github.com/nvlabs/kda,
  NOASSERTION). 108 tests, exactly 54 validation / 54 adversarial
  (`tests/validation.rs` + `tests/adversarial.rs` per crate). Per-crate design
  records in `crates/phlow-*/docs/decisions.md`. Validated on Primo:
  `cargo +nightly-2026-09-25 test --workspace` green,
  `clippy --workspace --all-targets -- -D warnings` clean,
  `rustfmt --check` clean.

- NVlabs/SoL-Pi agent-harness features, re-expressed in Tiger Style Rust as
  opt-in, disabled-by-default modules (upstream: MIT; "every mechanism is
  opt-in and disabled by default", "a missing configuration leaves every
  mechanism disabled"). `phlow-tools/src/solpi/`: **Action Fusion**
  (`fuse()` runs an edit/write and its follow-up validation in one call;
  validation is a caller-supplied closure, output capped at 8 KiB, failures
  never re-run or roll back the action) and **ObservationPack** (`PackStore`
  archives observations ≥ 10 KiB behind opaque handles with 4 KiB verbatim
  pages; originals always retrievable; 4 MiB per-object / 256-obs / 16 MiB
  store caps, typed errors, never silent eviction).
  `phlow-agent/src/solpi/`: **Evidence-Preserving Reducer** (compacts to a
  receipt only when every retained quotation matches the source byte-for-byte;
  any mismatch yields `Unchanged` and the borrowed source is untouched; the
  upstream remote reducer-model call is deliberately omitted — all
  verification is local) and **Online Context Compact** (explicit cost model:
  compacts only under window-pressure ≥ 0.75, ≥ 4096 reclaimable bytes, ≥ 2
  completed unpinned candidates; returns an observable `CompactionPlan`, never
  compacts itself). All four are inert unless explicitly enabled
  (`enable()`/`opt_in()`); disabled use fails closed with typed errors.
  Per-crate design records in `crates/phlow-{tools,agent}/docs/solpi-decisions.md`
  (upstream citations, bounds, MITRE ATLAS v6 mapping, out-of-scope rationale).
  46 tests (23 validation / 23 adversarial). Validated on Primo:
  `cargo +nightly-2026-09-25 test --workspace` green,
  `clippy --workspace --all-targets -- -D warnings` clean,
  `rustfmt --check` clean.

### Changed

- Operator config paths follow the Phlow product rename: the default
  auto-load path is now `$XDG_CONFIG_HOME/phlow/config.toml` (previously
  `$XDG_CONFIG_HOME/flow/config.toml`), and the preferred workspace-local
  project-config name is `.phlow.toml` (previously `.flow.toml`). The legacy
  locations keep working as a migration fallback — the `flow/` config is used
  when the `phlow/` file is absent (with a deprecation warning naming both
  paths), and `.flow.toml` remains warned-on and write-protected. When both
  exist, the `phlow/` location wins. Decision recorded in
  `docs/decisions.md`.

## [2026-09-28] — tuios session/inbox integration

### Added
- New workspace crate `phlow-tuios`: agent-session multiplexing adapted from
  the tuios architecture (MIT, Go/Bubble Tea terminal multiplexer).
  Concepts re-expressed in Tiger Style Rust; no Go code ported.
  - `state`: agent-state machine (none/working/needs_input/idle/done/
    errored/unknown; `needs_input` carries a reason).
  - `mailbox`: bounded per-session ring of direct messages, session
    notices, and ask records; sender rate limits; reserved `human`
    address; no self-addressing; ask-cycle refusal.
  - `hooks`: named events split daemon-side / client-side, fired with a
    bounded environment; hooks spawn as explicit argv (no implicit shell).
  - `protocol` + `server`: line-delimited JSON over a Unix socket, one
    request line to one response line, opaque request-id echo, stable
    string error codes, 16 MiB request cap, socket dir at mode 0700.
  - `daemon`: `SessionDaemon` owning up to 256 sessions; sessions own
    windows (64 max), agent panes, per-session mailbox and ask graph.
  - `tape`: declarative tapes that drive sessions (adapted; not keystroke
    replay).
  - Deliberate differences from tuios are recorded in
    `crates/phlow-tuios/docs/decisions.md` (JSON-only socket, explicit
    argv hooks, tapes drive sessions, no unauthenticated human spoofing,
    `ask-agent` is delivery+recording, `ping` is an adapted health verb).
- `docs/ARCHITECTURE.md`: reflects the landed integration (16 crates,
  Layer-0 placement, component-responsibilities row, TUI-client note).

### Decisions
- New language-neutral crate rather than extending `phlow-tui` or
  `phlow-mcp`: agent state, mailbox, and the session daemon are broader
  than one frontend, and the Unix-socket JSON-line protocol is not MCP.
  The TUI (and CLI, and future non-Rust clients) will be clients of this
  crate; wiring the TUI to it is a later change.
- Crate-wide `#![forbid(unsafe_code)]`; typed errors on all external
  input; named bounds on sessions, windows, mailbox capacity, hook
  concurrency, and request bytes.

### Validation
- `cargo +nightly-2026-09-25 test -p phlow-tuios` on primo: **122/122
  pass** (61 validation / 61 adversarial), 0 failed.
- `cargo +nightly-2026-09-25 clippy -p phlow-tuios --all-targets -- -D warnings`: clean.
- `cargo +nightly-2026-09-25 fmt -p phlow-tuios --check`: clean.
- Spot review: no `expect`/`unwrap` outside `#[cfg(test)]`; public items
  carry contract doc comments.
- Note: the lane's original "probed against the real tuios daemon"
  claim (2026-09-28, when `/tmp/tuios` existed) could not be re-verified —
  the upstream material is gone. The crate's own 122 tests are the
  standing evidence.
