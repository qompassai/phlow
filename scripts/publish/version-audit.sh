#!/usr/bin/env bash
# Version audit for the phlow workspace. Reports every crate version and
# checks that each phlow-* path dependency carries a version requirement
# matching the dependency's actual version (crates.io requires this).
# Exit 0 only if fully consistent.
#
#   scripts/publish/version-audit.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

echo "== crate versions =="
for c in crates/*/Cargo.toml; do
  name=$(grep -m1 '^name' "$c" | cut -d'"' -f2)
  ver=$(grep -m1 '^version' "$c" | cut -d'"' -f2)
  printf '%-24s %s\n' "$name" "$ver"
done | sort -k2

echo
echo "== path-dep version requirements =="
bad=0
for c in crates/*/Cargo.toml; do
  name=$(grep -m1 '^name' "$c" | cut -d'"' -f2)
  # find phlow-* deps of this crate
  while read -r dep; do
    [ -z "$dep" ] && continue
    depdir="crates/$dep"
    depver=$(grep -m1 '^version' "$depdir/Cargo.toml" | cut -d'"' -f2)
    # version requirement on the dep edge inside $c
    req=$(grep -E "^$dep = " "$c" | grep -oE 'version = "[^"]+"' | cut -d'"' -f2 || true)
    if [ -z "$req" ]; then
      echo "MISSING: $name -> $dep (no version requirement; dep is $depver)"
      bad=$((bad + 1))
    elif [ "$req" != "$depver" ]; then
      echo "MISMATCH: $name -> $dep requires $req, dep is $depver"
      bad=$((bad + 1))
    fi
  done < <(grep -oE '^phlow-[a-z0-9-]+ =' "$c" | cut -d' ' -f1)
done

if [ "$bad" -eq 0 ]; then
  echo "OK: all path-dep version requirements match"
else
  echo "FAIL: $bad inconsistent path-dep edges (run version-bump.sh <VERSION> first)"
  exit 1
fi
