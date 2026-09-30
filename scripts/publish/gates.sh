#!/usr/bin/env bash
# Phlow publish gates. Runs the four gates from the phlow-publish skill and
# reports PASS/FAIL with counts. Exit 0 only if all gates pass.
#
#   scripts/publish/gates.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

# Gauntlet integration tests need a Neovim binary; prefer an explicit
# GAUNTLET_NVIM_BIN, else fall back to the system nvim if present.
if [ -z "${GAUNTLET_NVIM_BIN:-}" ] && [ -x /usr/bin/nvim ]; then
  export GAUNTLET_NVIM_BIN=/usr/bin/nvim
fi

pass=0
fail=0
report() { # $1 = name, $2 = 0/1
  if [ "$2" -eq 0 ]; then echo "PASS $1"; pass=$((pass + 1));
  else echo "FAIL $1"; fail=$((fail + 1)); fi
}

echo "== 1. cargo build --workspace =="
if cargo build --workspace >/tmp/phlow-g-build.log 2>&1; then
  report "build" 0
else
  report "build" 1; tail -20 /tmp/phlow-g-build.log
fi

echo "== 2. cargo clippy --workspace --all-targets (zero warnings) =="
if cargo clippy --workspace --all-targets >/tmp/phlow-g-clippy.log 2>&1; then
  warns=$(grep -c '^warning' /tmp/phlow-g-clippy.log || true)
  if [ "$warns" -eq 0 ]; then report "clippy (0 warnings)" 0;
  else report "clippy ($warns warnings)" 1; fi
else
  report "clippy (build error)" 1; tail -20 /tmp/phlow-g-clippy.log
fi

echo "== 3. cargo fmt --all -- --check =="
if cargo fmt --all -- --check >/tmp/phlow-g-fmt.log 2>&1; then
  report "fmt" 0
else
  report "fmt" 1; head -20 /tmp/phlow-g-fmt.log
fi

echo "== 4. cargo test --workspace =="
if cargo test --workspace >/tmp/phlow-g-test.log 2>&1; then
  passed=$(grep -oE 'test result: ok\. [0-9]+ passed' /tmp/phlow-g-test.log \
    | grep -oE '[0-9]+' | paste -sd+ - | bc)
  failed=$(grep -oE 'test result: FAILED\. [0-9]+ passed; [0-9]+ failed' /tmp/phlow-g-test.log \
    | grep -oE '[0-9]+ failed' | grep -oE '[0-9]+' | paste -sd+ - | bc || true)
  echo "tests passed: ${passed:-0}, failed: ${failed:-0}"
  report "test" 0
else
  report "test" 1; grep -E 'test result: FAILED|failures:' /tmp/phlow-g-test.log | head -20
fi

echo "== gates: $pass passed, $fail failed =="
[ "$fail" -eq 0 ]
