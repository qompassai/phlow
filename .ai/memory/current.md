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
- **Pre-existing bugs** (separate follow-up, don't weaken):
  package-linux.sh basename collision; 52 path-deps lack version reqs
  (blocks cargo publish); flaky task_244/245 subprocess-timing tests.
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
