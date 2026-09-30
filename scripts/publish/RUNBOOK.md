# Phlow publish runbook

How a phlow release goes from "tree on main" to "crates on crates.io and a
GitHub release". Repeatable steps are scripts under `scripts/publish/`;
steps prefixed `tbr-` are **human-gated** — Matt runs them himself.

## What "publish" means for phlow

Phlow is a CLI/TUI/MCP agent runtime, not an Android app, so the publish
targets are:

1. **crates.io** — the 25 workspace crates, published in dependency order.
2. **GitHub release** on `qompassai/phlow` — tag plus the Linux tarball.
3. **docs.rs** — automatic per published crate; verified after release.

Out of scope for now (honest split): F-Droid and Google Play do not apply
to a desktop/server CLI tool. A Windows build (cross-compile) and an AUR
package are future work, not part of this release.

## The pipeline

```
gates.sh  →  version-bump.sh <V>  →  publish-dryrun.sh  →  CHANGELOG
   (repeatable)      (Matt picks V)      (repeatable)
      →  tbr-crates-publish.sh  →  tbr-github-release.md  →  docs.rs check
            (Matt, needs his            (Matt: tag +
             crates.io token)            GitHub release)
```

## Step by step

### 1. Gates (repeatable)

```sh
scripts/publish/gates.sh
```

Runs `cargo build --workspace`, `cargo clippy --workspace --all-targets`
(zero warnings), `cargo fmt --all -- --check`, and `cargo test --workspace`,
reporting PASS/FAIL with counts. All four must pass.

### 2. Version (Matt's decision)

The workspace ships mixed versions until release time. Matt picks the
release version; the bump is mechanical:

```sh
scripts/publish/version-bump.sh 0.3.0
```

This sets every crate's `version` and every `phlow-*` path-dep `version`
requirement to the new version (crates.io rejects path deps without a
version requirement), then runs `version-audit.sh` to verify.

### 3. Dry-run (repeatable)

```sh
scripts/publish/publish-dryrun.sh
```

Runs `cargo publish --dry-run -p <crate>` for all 25 crates in topological
order (leaves first; see `topo-order.sh`). Must be 25/25 clean.

### 4. CHANGELOG

Move the `## [Unreleased]` entries under a new `## [VERSION] - YYYY-MM-DD`
heading (Keep a Changelog format). Commit the version bump + CHANGELOG
together; this commit is what gets tagged.

### 5. Publish to crates.io (human-gated, Matt)

```sh
VERSION=0.3.0 scripts/publish/tbr-crates-publish.sh
```

Needs Matt's crates.io token in his cargo config (`cargo login`). The
script re-verifies the tree, versions, and gates, then publishes each crate
in dependency order with index-propagation retries. Never `--allow-dirty`.

### 6. Tag + GitHub release (human-gated, Matt)

Follow `tbr-github-release.md`: tag `vVERSION`, push the tag, build the
Linux tarball with `package-linux.sh`, create the GitHub release with the
CHANGELOG notes, verify docs.rs.

## Rollback

- A bad crates.io release is **yanked**, not deleted: `cargo yank -p <crate> --vers <V>`.
  Yanking needs Matt's explicit direction each time.
- A bad tag is deleted and re-pushed only before the GitHub release is cut.
