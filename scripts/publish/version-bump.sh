#!/usr/bin/env bash
# Unify the workspace on one release version: sets every crates/*/Cargo.toml
# `version` and every phlow-* path-dep `version` requirement to VERSION.
# The release version is Matt's decision — pass it explicitly.
#
#   scripts/publish/version-bump.sh 0.3.0
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

VERSION="${1:-}"
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: $0 <VERSION>   (e.g. $0 0.3.0)" >&2
  exit 2
fi

python3 - "$VERSION" <<'EOF'
import re, sys, glob

version = sys.argv[1]

def fix_dep(m):
    dep, inner = m.group(1), m.group(2)
    if re.search(r'version\s*=', inner):
        inner = re.sub(r'version\s*=\s*"[^"]*"', f'version = "{version}"', inner)
    else:
        inner = inner.rstrip()
        if inner and not inner.endswith(','):
            inner += ','
        inner += f' version = "{version}"'
    return f"{dep} = {{{inner} }}"

for path in sorted(glob.glob("crates/*/Cargo.toml")):
    with open(path) as f:
        text = f.read()
    # [package] version is the first version line in the file
    text, n = re.subn(r'(?m)^version = "[^"]+"', f'version = "{version}"',
                      text, count=1)
    assert n == 1, f"no version line in {path}"
    # every phlow-* path-dep edge gets a matching version requirement
    text = re.sub(r'(?m)^(phlow-[a-z0-9-]+)\s*=\s*\{(.*)\}$', fix_dep, text)
    with open(path, "w") as f:
        f.write(text)
    print(f"bumped {path}")
EOF

echo "== verifying =="
scripts/publish/version-audit.sh
