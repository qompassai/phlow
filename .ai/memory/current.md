# Current Work — phlow

Python agent runtime being ported to Rust. CLI/TUI/MCP share one safe runtime.

## Active (2026-10-04)

- **Rust port**: `main` is the port branch. tiger-style-rust. Phase 5 in
  progress. Nightly + rust-analyzer + clippy installed.
- **Standing push auth** (2026-09-28): Push to qompassai/phlow `main` ONLY
  when the exact tree pushed is build-green + tests-passing on primo.
  GitHub Git Data API (no HTTPS git auth on VM). Byte-identical,
  force=false. Verify origin/main == local after each push. Confirm via
  ls-remote (beware stale remote-tracking refs in shallow clone).
- **Pre-existing bugs**: none open. The three recorded 2026-10-04 are
  closed: package-linux.sh basename collision (fixed `37ebe94`,
  verified on main 2026-10-12); path-dep version requirements
  (completed `8cb5aab` — the last two, both on phlow-trainlab);
  task_244/245 subprocess-timing flakes (observations hardened to
  poll-until-state, `5e40406`).
- **Staged** (each needs one-time auth): (2) diver flip + push,
  (3) land rework + push, (4) delete flow/ + Python packaging (destructive,
  git-wip-guard). (1) smoke done.
- **Serve smoke**: MCP handshake byte-identical to Python flow serve
  (protocolVersion 2025-11-25). Tool-schema/error-semantics parity is
  follow-up.

## Standing rules

- ALL cargo on primo. Sandbox `~/workspace/repos/phlow` is stale/damaged —
  primo is source of truth.
- Probe the real Python for contract parity, not code-reading.
- Half adversarial / half validation testing. MITRE ATLAS gambit for
  feasible techniques.
