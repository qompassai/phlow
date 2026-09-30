#!/usr/bin/env bash
# HUMAN-GATED: publish the phlow crates to crates.io for real.
# Matt runs this himself. Requires his crates.io token in the cargo config
# (~/.cargo/credentials.toml via `cargo login`) — this script never reads,
# echoes, or stores any token.
#
#   VERSION=0.3.0 scripts/publish/tbr-crates-publish.sh
#
# Preconditions (enforced, not assumed):
#   1. VERSION is set and every crate is bumped to it (version-bump.sh).
#   2. Clean tree.
#   3. All publish gates green (scripts/publish/gates.sh).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

VERSION="${VERSION:-}"
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "ABORT: set VERSION (e.g. VERSION=0.3.0 $0)" >&2
  exit 2
fi
if [ -n "$(git status --porcelain)" ]; then
  echo "ABORT: tree is not clean" >&2
  exit 1
fi
for c in crates/*/Cargo.toml; do
  v=$(grep -m1 '^version' "$c" | cut -d'"' -f2)
  if [ "$v" != "$VERSION" ]; then
    echo "ABORT: $c is $v, want $VERSION (run version-bump.sh $VERSION)" >&2
    exit 1
  fi
done

echo "== gates =="
scripts/publish/gates.sh || { echo "ABORT: gates failed" >&2; exit 1; }

echo "== publishing $VERSION to crates.io in dependency order =="
while read -r crate; do
  [ -z "$crate" ] && continue
  echo "--- dry-run $crate ---"
  cargo publish --dry-run -p "$crate" || { echo "ABORT: dry-run failed for $crate" >&2; exit 1; }
  echo "--- publish $crate ---"
  # crates.io index propagation: retry a few times on version-selection lag
  for attempt in 1 2 3 4 5 6; do
    if cargo publish -p "$crate" 2>/tmp/phlow-pub-err.log; then
      break
    elif grep -q "failed to select a version" /tmp/phlow-pub-err.log && [ "$attempt" -lt 6 ]; then
      echo "index not propagated yet; waiting 60s (attempt $attempt/6)"
      sleep 60
    else
      echo "ABORT: publish failed for $crate"; cat /tmp/phlow-pub-err.log >&2
      exit 1
    fi
  done
  echo "PUBLISHED $crate $VERSION"
done < <(scripts/publish/topo-order.sh)

echo "== all crates published at $VERSION =="
echo "Next: tag the release commit and cut the GitHub release (tbr-github-release.md)."
