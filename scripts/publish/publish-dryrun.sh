#!/usr/bin/env bash
# Dry-run the crates.io publish for every crate, in topological order.
# Requires a version-consistent workspace (see version-audit.sh).
# Never touches the network beyond the crates.io index read that
# `cargo publish --dry-run` performs; nothing is uploaded.
#
#   scripts/publish/publish-dryrun.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

scripts/publish/version-audit.sh >/dev/null || {
  echo "ABORT: workspace versions inconsistent; run version-bump.sh <VERSION> first" >&2
  exit 1
}

pass=0
fail=0
failed_crates=()
while read -r crate; do
  [ -z "$crate" ] && continue
  if cargo publish --dry-run -p "$crate" >/tmp/phlow-dryrun-"$crate".log 2>&1; then
    echo "DRY-RUN OK   $crate"; pass=$((pass + 1))
  else
    echo "DRY-RUN FAIL $crate"; fail=$((fail + 1)); failed_crates+=("$crate")
  fi
done < <(scripts/publish/topo-order.sh)

echo "== dry-run: $pass passed, $fail failed =="
if [ "$fail" -gt 0 ]; then
  echo "failed: ${failed_crates[*]}"
  exit 1
fi
