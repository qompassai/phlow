# Model-supplied code children now get the phlow-seccomp egress filter

- Discovery (security bundle 2026-11-20, LiteLLM CVE-2026-40217 class audit):
  phlow-trainlab's reward executor (model completions; reached via
  `phlow-trainlab eval/run --execute-rewards` and `phlow-autoresearch run-live`)
  and phlow-self-improve `validate_check` (`phlow-escapes propose --validate`,
  runs a recorded argv before any operator verdict) spawned children WITHOUT
  the egress filter that phlow-checks installs on approved checks.
- Fix: branch pax/security-bundle-20261120, commit 6d29ada. Both spawn sites
  call `phlow_seccomp::apply_egress_filter`, fail-closed.
- Residual (not fixed, design-level): children still run as the operator uid
  and can read/write ~ (e.g. ~/.ssh, ~/.bashrc). Closing that needs Landlock /
  a mount namespace — phlow is not an OS sandbox.
- Not reachable from `phlow serve` (MCP): phlow-mcp/cli/agent/tui do not link
  trainlab or autoresearch; runtime links self-improve but never calls
  validate_check (cargo tree -i, 2026-11-22).
- Gate notes: workspace test needs `randori` pixi env (`pixi install`) or
  `--exclude randori`; phlow-gauntlet task_08 needs
  GAUNTLET_PHLOW_BIN=<tree>/target/debug/phlow (fallback path is
  ~/workspace/repos/phlow, absent on primo); phlow-checks fleet_checks
  toolchain_accepts_* fail on Arch rustc (version string suffix
  "(Arch Linux rust ...)").
- Validate: cargo test -p phlow-trainlab -p phlow-self-improve -- ip_sockets --nocapture
