# Patterns — phlow

## Toolchain

- Rust nightly, all features, tokio async, ratatui TUI.
- MDBook docs under the phlow name.
- Orchestration: `~/workspace/bin/primo-dispatch` → pax-dispatch (tmux).
  Claude = Opus 5.5 + worker-settings.json; Codex = gpt-6-astra worker.
  Bounded 3600s. Every brief carries a handover contract.

## Testing

- Half adversarial / half validation. Don't weaken pre-existing flaky tests;
  fix or queue separately with Matt's decision.
- Gauntlet: 130 difficult agent-orchestration tests (in progress).

## Neovim integration

- rose.nvim ↔ phlow: MCP stdio client in `lua/rose/native/mcp.lua`;
  private socket back for editor tools. Keep wire formats stable across
  the seam (MCP framing, `Rose.Config` keys, socket protocol).

## Skill

`~/workspace/skills/flow-agent/SKILL.md` — covers the Python agent runtime
workflows; Rust port follows tiger-style-rust.
