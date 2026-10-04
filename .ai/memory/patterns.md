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

## Repomap: read first, verify fresh

`.repomap.txt` (repo root) is the always-fresh codebase map. **Read it
before exploring the tree** — it lists the highest-signal definitions
first (ranked by cross-file references), then a complete per-file table
of contents. Cheaper than walking the tree yourself.

**Verify it's current before trusting it.** The map is a derived artifact;
it goes stale when sources change outside a `nix develop` session.

```bash
# Is the map newer than every .rs file? (empty output = fresh)
find . -name '*.rs' -newer .repomap.txt 2>/dev/null | head -5
# If stale or missing, regenerate (no flake changes needed):
nix run github:qompassai/nix?dir=repomap -- . --budget 15000 --out .repomap.txt
```

The devShell `shellHook` auto-regenerates the map on every `nix develop`
entry when an `.rs` file is newer than it. If you edited sources without
entering the shell, regenerate manually with the command above.

`.repomap.txt` is gitignored — never commit it. If it's missing, the
one-shot command recreates it.
