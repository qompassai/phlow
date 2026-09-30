# Flake apps — `nix run .#<app>`

Deterministic repo workflows. Toolchains come from pinned flake inputs
(see `flake.lock`); the cargo registry cache and (for `debug-smoke`)
the DAP server come from the ambient environment, and the docs below
call that out wherever it matters. Each app:

- fails fast naming any missing dependency,
- builds in an isolated `CARGO_TARGET_DIR` inside a temp scratch dir,
  removed on `EXIT` whether the run passes or fails,
- writes a markdown report to `reports/<app>-<UTC timestamp>.md`
  (`reports/` is gitignored),
- proves the tree is otherwise untouched (`git status --short` must
  show only the new report),
- runs every cargo invocation with `--locked` (reproducible against
  the checked-in `Cargo.lock`).

**First run costs one full workspace compile** (~30+ crates incl.
nightly features) into the isolated target dir; later runs reuse the
Nix store toolchain and your warm cargo registry cache. `version-audit`
and `debug-smoke` are the fast ones to try first.

**Scratch disk space.** The scratch dir defaults to
`${PHLOW_SCRATCH_BASE:-${TMPDIR:-/tmp}}`, and each app aborts fast if
that filesystem has less than ~8 GiB free (a full workspace build+test
does not fit in less — failing fast beats a cryptic linker crash from
`ENOSPC`). On machines where `/tmp` is a small tmpfs, export
`PHLOW_SCRATCH_BASE=/var/tmp` (or another disk-backed dir) before
running the heavy apps (`gates`, `publish-dryrun`, `release`).

## The apps

| App | Command | What it does |
|-----|---------|--------------|
| `gates` | `nix run .#gates` | The 4 publish gates from `scripts/publish/gates.sh`: `cargo build --locked`, `clippy` with **zero warnings**, `cargo fmt --check`, `cargo test --locked`. |
| `security` | `nix run .#security` | `cargo audit` (advisories), `cargo geiger --forbid-only` (unsafe code report). `cargo deny` is **skipped** — the repo has no `deny.toml` and the app won't invent a policy; add one and the app picks it up. |
| `publish-dryrun` | `nix run .#publish-dryrun` | `scripts/publish/version-audit.sh` + topological order + `cargo publish --dry-run --locked` for every crate. Never uploads. |
| `version-audit` | `nix run .#version-audit` | Fast read-only workspace version-consistency check. |
| `gauntlet` | `nix run .#gauntlet` | Builds the gauntlet binary and drives it. No args = `gauntlet list` smoke test. Pass args through for real runs: `nix run .#gauntlet -- run-all --clean`. Task runs need `--diver-lua` (never guessed — set `DIVER_LUA` or pass the flag); the Neovim binary defaults to the Nix-provided one (`GAUNTLET_NVIM_BIN` overrides). `--clean` removes the work dir only when every task passes. |
| `debug-smoke` | `nix run .#debug-smoke` | Proves the debugger chain end to end: builds the `phlow-json` test binary, drives `lldb-dap` over the real DAP protocol (`initialize` → `launch` stopped-at-entry → `setBreakpoints` → verified → `continue` → `stopped` at breakpoint), and **passes only if the breakpoint on `phlow_json::parse_limited` both verifies AND is hit**. Uses the ambient `lldb-dap` from PATH on purpose — nixpkgs' lldb 21.1.8 ships a broken `lldb-dap` (every `launch` fails with "invalid debugger", verified 2026-09-30), and the devShell does not pin lldb either. Honest `FAIL` when no DAP server is on PATH — never faked. |
| `release` | `nix run .#release -- 0.3.0 [--dry-run]` | Cuts a GitHub release end to end. See below. |

## Releasing (`nix run .#release`)

There is no default release: the app **refuses to run without an
explicit `<version>`** and validates it (`MAJOR.MINOR.PATCH`, optional
`v` prefix). Safety order:

1. The tag `v<version>` must not exist locally or as a GitHub release.
2. `crates/phlow-cli/Cargo.toml` must already be at `<version>`
   (release the version you bumped — run
   `scripts/publish/version-bump.sh <version>` first).
3. `gh` must be authenticated — ambient auth only; the app never
   accepts a token via args or env.
4. Builds `dist/phlow-<version>-x86_64-unknown-linux-gnu.tar.gz` +
   `.sha256` deterministically via `scripts/publish/package-linux.sh`.
5. Release notes self-populate, never hand-written:
   `CHANGELOG.md` section for the version if present, else `git-cliff`
   (conventional commits) between the previous tag and `HEAD`, else a
   `git log` summary.
6. `gh release create v<version> --title "phlow <version>" --notes-file …`
   with the tarball + sha256 as assets. `--dry-run` does everything
   except this step.
7. Report written; the built artifacts are removed after upload.

Always rehearse with `--dry-run` first.

## What stays a script (human-gated)

`scripts/publish/version-bump.sh` (mutates the tree),
`scripts/publish/package-linux.sh` (build helper used by `#release`),
`scripts/publish/tbr-crates-publish.sh` (real crates.io publish) and
`scripts/publish/tbr-github-release.md` (manual release checklist) are
deliberately **not** flake apps. `#release` automates the *mechanical*
half of `tbr-github-release.md` (tarball + notes + `gh release
create`); crates.io publishing, signing keys, and Play/F-Droid
submissions stay human-gated, always.

## Layout

- `nix/lib/common.sh` — shared prelude (dep checks, repo-root guard,
  isolated cargo env, report helpers, cleanup/tree-clean verification).
  Prepended to every app by `flake.nix`; not run directly.
- `nix/apps/<name>.sh` — one app body each. Must stay
  `shellcheck`-clean (`writeShellApplication` enforces this at build
  time).

## For agents

`nix run .#gates` is the canonical pre-commit validation. If you need
the raw scripts instead (e.g. their `/tmp` log behavior), they still
work directly — but the apps are the deterministic path.
