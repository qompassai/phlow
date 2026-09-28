# Decisions

Dated, append-only records of product-level decisions. Newest last.

## 2026-09-28 — flow → Phlow product rename

The product is now **Phlow**. The Rust port (binary, user-visible
strings, docs, packaging, metadata) was renamed `flow` → `phlow`. The
`flow` compatibility binary alias was removed: the only binary is
`phlow`, and `--version` prints `Phlow 0.2.0`.

These names deliberately **stay** as `flow`:

- **MCP tool names `flow_run`, `flow_check`, `flow_status`.** They are a
  wire contract: renaming them breaks every connected client (rose.nvim
  and any other MCP caller). The names are protocol, not branding.
- **The legacy Python `flow/` package.** It is the historical
  implementation. Its module name, entry point (`flow`), config paths,
  and docs are frozen as history; the Rust port does not touch them.
- **`phlow-editor` `contract.rs` doc comments naming
  `Flow._editor_context`.** They name the Python class, which is
  unchanged — renaming the reference would lie about what it mirrors.

The two `system_prompt.md` copies
(`crates/phlow-runtime/skills/system_prompt.md` and
`flow/skills/system_prompt.md`) were both updated to "You are Phlow"
and must remain byte-identical (pinned by
`vendored_prompt_is_byte_identical_to_flow_skills`).

## 2026-09-28 — Config paths follow the Phlow rename (with legacy fallback)

The operator config locations were the last product-facing `flow` names the
rename did not cover. The default auto-load path is now
`$XDG_CONFIG_HOME/phlow/config.toml`, and the preferred workspace-local
project-config name is `.phlow.toml` (warned-on when not explicitly
selected, and always write-protected).

To avoid stranding existing operators, the legacy locations keep working as
a migration fallback: `$XDG_CONFIG_HOME/flow/config.toml` is used when the
`phlow/` file does not exist, and `.flow.toml` is still warned-on and
protected. Selecting the legacy XDG path emits a deprecation warning naming
both locations. When both locations exist, the `phlow/` one wins. This keeps
the rename promise ("everywhere product-facing") without a flag day.
