# Agent configuration for phlow

Agent skills for working on the phlow repo live in two places:

- `.claude/skills/` — read by Claude Code (which reads only its own
  skills directory).
- `.agents/skills/` — the cross-tool path, read by OpenCode, Codex,
  Cursor, GitHub Copilot, Gemini CLI, and others.

OpenCode also reads `.claude/skills/` directly, so the old
`.opencode/skills/` duplicates were removed as redundant; this
`.opencode/` directory is kept for OpenCode-specific configuration.

Skills: `tiger-style-rust` (all Rust code), `tiger-style-nix`
(flake.nix / dev shells), `tiger-style-mojo` (Mojo kernels and worker),
`git-wip-guard` (never destroy uncommitted work), `phlow-publish`
(crates.io release checklist: gates, version audit, topological publish
order, tagging), `rust-development` (Rust workflow: bacon, lldb-dap,
cargo-bsp, cargo-deny).
