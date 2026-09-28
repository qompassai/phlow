# Changelog

All notable changes to phlow are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); dates are UTC.

## [Unreleased]

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
