#!/usr/bin/env bash
# Topological publish order for the phlow crates, derived from the
# phlow-* path-dependency edges (leaves first). Used by publish-dryrun.sh
# and tbr-crates-publish.sh. Prints one crate name per line.
#
#   scripts/publish/topo-order.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

python3 - <<'EOF'
import re, glob
from collections import defaultdict

crates = {}
for path in sorted(glob.glob("crates/*/Cargo.toml")):
    text = open(path).read()
    name = re.search(r'(?m)^name = "([^"]+)"', text).group(1)
    deps = set(re.findall(r'(?m)^(phlow-[a-z0-9-]+)\s*=\s*\{[^}]*path\s*=', text))
    crates[name] = deps

# Kahn's algorithm, deterministic (sorted)
order, ready = [], sorted(n for n, d in crates.items() if not d)
remaining = {n: set(d) for n, d in crates.items()}
while ready:
    n = ready.pop(0)
    order.append(n)
    for m, d in remaining.items():
        if n in d:
            d.discard(n)
            if not d and m not in order and m not in ready:
                ready.append(m)
    ready.sort()
if len(order) != len(crates):
    missing = sorted(set(crates) - set(order))
    raise SystemExit(f"CYCLE or missing dep among: {missing}")
print("\n".join(order))
EOF
