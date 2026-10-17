# A2A interoperability — first gated increment (2026-10-17)

Branch `pax/a2a-interop-20261017` from local main c43fcce, worktree
`~/workspace/repos/phlow-a2a-20261017`. NOT landed, NOT pushed.
Design contract: `~/workspace/phlow-a2a-program/SPEC.md` (sandbox).

## What landed on the branch

- `phlow-flowguard` (new leaf crate): OpenAPPA-principles flow
  security. Label = audience (reader set, statically expanded) ×
  trust (rank in a policy-declared chain). A trajectory's label is a
  pure fold of its append-only event log; join (⊔) = reader-set
  intersection + trust min is the only combination operation, so
  downgrade is unrepresentable (no set/widen API exists). TOML policy
  with full load-time validation (trust chain, group cycles, `within`
  assertions, delta/source/destination/sanitizer resolution, SHA-256
  fingerprint). Deterministic `decide()` — no model in the path.
  Remedies: sanitize (declared result_delta), authority approval
  (digest-bound, policy-fingerprint-bound, single-use, never lifts
  the label, never clears effect violations), isolation branches.
  Reserved mark `blocked` admits no remedy.
- `phlow-a2a` (new crate): A2A protocol v1.0 (ProtoJSON). Agent Card
  at `/.well-known/agent-card.json`; SendMessage,
  SendStreamingMessage (SSE), GetTask, ListTasks, CancelTask,
  SubscribeToTask, GetExtendedAgentCard; 0.2/0.3 aliases accepted as
  a recorded interop deviation. Hand-rolled std::net server
  (bounded everything); reqwest client. Server refuses non-loopback
  bind without per-agent bearer tokens. Gate wired per task:
  caller identity comes from the bearer token, never the payload;
  unknown callers join an unvetted delta (audience self, lowest
  trust). Push notifications not implemented (card says false).
- `phlow-runtime::multiagent`: concurrent sessions on the shared
  attention board; one service thread owns the dispatcher +
  authority callback + external sender, preserving Runtime's
  single-writer contract. The service thread RE-RUNS
  flowguard::decide over the session's cloned log before every
  dispatch — execution never trusts an earlier check. Attention
  switches are explicit FocusSwitched events; sub-agent spawn joins
  the parent's label with the requested delta; a child's end label
  joins back only through a recorded SubagentResult (the join that
  defeats two-agent laundering).

## Gate numbers (primo, this branch)

- phlow-flowguard: 28/28. phlow-a2a: 29/29. phlow-runtime: 111/111
  (102 pre-existing + 9 new multiagent).
- Workspace `cargo test --workspace --exclude randori
  --no-fail-fast`: 2343 passed / 273 failed = 272 phlow-gauntlet
  (pre-existing environmental: every failure panics on the missing
  `/home/phaedrus/workspace/tools/neovim-nightly/bin/nvim`,
  GAUNTLET_NVIM_BIN — the tree does not exist on primo) + 1
  phlow-cli (known: rose-serve live). Neither is touched by this
  diff.
- `cargo clippy --workspace --exclude randori --all-targets --
  -D warnings`: rc 0. `cargo fmt --check` (workspace): rc 0.

## Durable lessons

- Sanitizer arming is consumed by the next ToolResult, not the next
  ToolAllowed: the session records ToolAllowed before dispatch and
  the dispatch-point re-check must observe the same armed state.
- Approvals are consumed by the first later ToolResult carrying the
  same action digest; child trajectories inherit labels at spawn
  but copy NO approval events (confused-deputy guard).
- Do not put worktrees under /tmp on primo: it is a 16G tmpfs; a
  baseline worktree's target dir filled it and lld died with a bus
  error mid-link. Worktrees belong under ~/workspace/repos/.
- The publish-prep "390 passed" workspace figure came from a
  narrower invocation than `--no-fail-fast` over all binaries
  (2343 passed here); always state the invocation with the count.
