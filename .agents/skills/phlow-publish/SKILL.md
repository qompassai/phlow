---
name: "phlow-publish"
description: "Crates.io release checklist for the phlow Rust workspace (~27 crates). Trigger when Matt asks to publish phlow crates: covers preconditions, version audit, topological publish order, and post-publish verification."
metadata:
  includeInPrompt: "true"
---

# Phlow Publish

Release the phlow workspace crates to crates.io, in dependency order,
without breaking downstream consumers.

## Preconditions

1. Clean tree: `git status --porcelain` empty. Nothing uncommitted.
2. Full gates green from the repo root:
   - `cargo build --workspace`
   - `cargo clippy --workspace --all-targets` — zero warnings
   - `cargo fmt --all -- --check` — clean
   - `cargo test --workspace` — all pass
3. `CHANGELOG.md` updated with the release notes for this version.
4. Matt has explicitly authorized this release (version number included).
   Each release authorization is one-time.

## Version audit first

The workspace has mixed versions (19 crates at `0.1.0`, 6 at `0.2.0`).
Before anything else:

1. List every crate version: `grep -h '^version' crates/*/Cargo.toml`.
2. Decide the release version with Matt. Do not invent one.
3. Bump consistently: every crate being released gets the new version,
   and every `phlow-*` path dependency requirement inside the workspace
   is updated to match. A crate published against a path-dep version
   that doesn't exist on crates.io will fail to publish.

## Publish order

crates.io has no workspace-aware publish. Publish leaf crates first,
in topological order of the `phlow-*` path dependencies:

1. Compute the order: `cargo tree --workspace` or inspect the
   `phlow-* = { path = ... }` edges in each `Cargo.toml`.
2. For each crate, in order:
   - `cargo publish --dry-run -p <crate>` — must pass clean.
   - `cargo publish -p <crate>` — the real publish.
   - Allow crates.io index propagation before publishing a crate that
     depends on the one just published (a few minutes; retry on
     "failed to select a version" errors rather than forcing).
3. Never use `--allow-dirty`. Never publish with uncommitted changes.

## After publish

1. Tag: `git tag v<X.Y.Z>` on the release commit, push the tag.
2. GitHub release on `qompassai/phlow` with the changelog entry.
3. Verify each published crate built on docs.rs
   (`https://docs.rs/phlow-<crate>/<version>`); re-check any that show
   build failures.

## What this skill does not cover

- crates.io tokens: they live in the user's cargo config
  (`~/.cargo/credentials.toml` or `cargo login`). Never ask for, read,
  or echo a token.
- Yanking a bad release, transferring crate ownership, or reserving new
  crate names — those need Matt's explicit direction each time.
