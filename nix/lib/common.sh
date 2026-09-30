# --- phlow flake-app common library ---------------------------------------
# Prepended to every app script by flake.nix (NOT sourced at runtime).
# PHLOW_APP_NAME is baked in by flake.nix just above this library.
#
# Provides:
#   need_bin <cmd> [nix-pkg]  - fail fast, naming the missing dependency
#   require_repo_root          - refuse to run outside the phlow checkout
#   isolated_cargo_env         - fresh CARGO_TARGET_DIR in temp scratch dir,
#                                removed on EXIT whether we pass or fail
#   report_begin               - start reports/<app>-<UTC ts>.md
#   report_section <title>     - append a markdown section header
#   report_line <text...>      - append markdown lines
#   run_step <name> <log> <cmd...> - run a step, tee output, record PASS/FAIL
#   verify_cleanup             - remove scratch now, prove it is gone
#   verify_tree_clean          - prove the tree holds only the new report
# --------------------------------------------------------------------------

set -euo pipefail

PHLOW_SCRATCH=""
REPORT_FILE=""

# Fail fast with a clear message naming the missing dependency.
need_bin() {
  local cmd="$1" pkg="${2:-$1}"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "MISSING DEPENDENCY: '$cmd' is not on PATH (nix package: ${pkg})." >&2
    echo "This app is hermetic: every dependency must come from the flake inputs." >&2
    exit 3
  fi
}

# Refuse to run anywhere but the phlow repo root.
require_repo_root() {
  if [ ! -f ./Cargo.toml ] || [ ! -d ./crates ] || [ ! -f ./rust-toolchain.toml ]; then
    echo "ABORT: run this app from the phlow repository root (cwd: $PWD)." >&2
    exit 2
  fi
  PHLOW_REPO_ROOT="$PWD"
  PHLOW_HEAD="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
}

# Isolated cargo environment. Build scratch lives in a temp dir and is
# removed on EXIT (success or failure). The user's CARGO_HOME registry
# cache is reused read-mostly; Cargo.lock plus --locked keeps every build
# pinned, so reusing the cache does not hurt determinism.
#
# Scratch base: ${PHLOW_SCRATCH_BASE:-${TMPDIR:-/tmp}}. On machines where
# /tmp is a small tmpfs (16G on primo), export PHLOW_SCRATCH_BASE=/var/tmp
# (or another disk-backed dir) before running the heavy apps.
#
# Usage: isolated_cargo_env [min_free_kb] — aborts with a clear message if
# the scratch filesystem has less than min_free_kb free (default 8 GiB).
# A full-workspace build+test does not fit in less; failing fast beats a
# cryptic linker crash from ENOSPC halfway through.
isolated_cargo_env() {
  local min_kb="${1:-8388608}"
  local base="${PHLOW_SCRATCH_BASE:-${TMPDIR:-/tmp}}"
  [ -d "$base" ] || { echo "ABORT: scratch base $base does not exist." >&2; exit 2; }
  local free_kb
  free_kb="$(df -k --output=avail "$base" | tail -1 | tr -d ' ')"
  if [ -n "$free_kb" ] && [ "$free_kb" -lt "$min_kb" ]; then
    echo "ABORT: only $((free_kb / 1024)) MiB free in $base; need $((min_kb / 1024)) MiB." >&2
    echo "Set PHLOW_SCRATCH_BASE to a disk-backed directory (e.g. /var/tmp) and re-run." >&2
    exit 2
  fi
  PHLOW_SCRATCH="$(mktemp -d "${base}/phlow-${PHLOW_APP_NAME}-XXXXXX")"
  export CARGO_TARGET_DIR="${PHLOW_SCRATCH}/target"
  export CARGO_NET_RETRY=3
  # shellcheck disable=SC2064
  trap 'rc=$?; [ -n "${PHLOW_SCRATCH:-}" ] && [ -d "${PHLOW_SCRATCH:-}" ] && rm -rf "${PHLOW_SCRATCH:-}"; exit "$rc"' EXIT
}

report_begin() {
  local ts
  ts="$(date -u +%Y%m%d-%H%M%S)"
  mkdir -p reports
  REPORT_FILE="reports/${PHLOW_APP_NAME}-${ts}.md"
  {
    echo "# phlow flake app report: ${PHLOW_APP_NAME}"
    echo
    echo "- date (UTC): $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "- repo HEAD: ${PHLOW_HEAD:-unknown}"
    echo "- repo root: ${PHLOW_REPO_ROOT:-$PWD}"
    echo "- rustc: $(rustc --version 2>/dev/null || echo 'not on PATH')"
    echo "- cargo: $(cargo --version 2>/dev/null || echo 'not on PATH')"
    echo
  } >"$REPORT_FILE"
  echo "report: $REPORT_FILE"
}

report_section() {
  { echo "## $1"; echo; } >>"$REPORT_FILE"
}

report_line() {
  printf '%s\n' "$*" >>"$REPORT_FILE"
}

# run_step <name> <logfile> <command...> — runs the step, records PASS/FAIL
# with a log tail in the report. Returns the step's exit status.
# shellcheck disable=SC2329 # shared lib: not every app uses every helper
run_step() {
  local name="$1" log="$2"
  shift 2
  report_section "$name"
  echo "== $name =="
  if "$@" >"$log" 2>&1; then
    echo "PASS $name"
    report_line "result: **PASS**"
    return 0
  else
    # NB: capture $? inside the else branch. Reading it after the `fi`
    # always yields 0 (a failed if-condition with no else exits 0).
    local rc=$?
    echo "FAIL $name (exit $rc; full log: $log)"
    report_line "result: **FAIL** (exit $rc)"
    report_line ""
    report_line "<details><summary>log tail</summary>"
    report_line ""
    report_line '```'
    tail -30 "$log" >>"$REPORT_FILE"
    report_line '```'
    report_line "</details>"
    return "$rc"
  fi
}

# Remove the scratch dir NOW (idempotent with the EXIT trap) and prove it.
verify_cleanup() {
  report_section "cleanup"
  if [ -n "${PHLOW_SCRATCH:-}" ] && [ -d "${PHLOW_SCRATCH}" ]; then
    rm -rf "${PHLOW_SCRATCH}"
  fi
  if [ -n "${PHLOW_SCRATCH:-}" ] && [ -e "${PHLOW_SCRATCH}" ]; then
    echo "FAIL cleanup: scratch dir still exists: ${PHLOW_SCRATCH}"
    report_line "cleanup: **FAIL** — scratch dir still exists"
    PHLOW_SCRATCH=""
    return 1
  fi
  PHLOW_SCRATCH=""
  echo "PASS cleanup: scratch dir removed"
  report_line "cleanup: **PASS** — scratch dir removed; CARGO_HOME registry cache reused read-mostly (pinned via Cargo.lock + --locked)"
  return 0
}

# Prove the tree holds no modifications other than the new report.
verify_tree_clean() {
  report_section "tree state"
  local dirty
  dirty="$(git status --porcelain | grep -v '^?? reports/' || true)"
  if [ -n "$dirty" ]; then
    echo "FAIL tree state: unexpected modifications:"
    echo "$dirty"
    report_line "tree state: **FAIL** — unexpected modifications besides reports/:"
    report_line '```'
    printf '%s\n' "$dirty" >>"$REPORT_FILE"
    report_line '```'
    return 1
  fi
  echo "PASS tree state: clean except the new report"
  report_line "tree state: **PASS** — git status clean except the new report"
  return 0
}
