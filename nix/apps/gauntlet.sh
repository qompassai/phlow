# phlow-gauntlet: build the gauntlet binary hermetically and drive it.
# Default (no args) is a smoke run: `gauntlet list` proves the binary
# works. Pass gauntlet args through for real runs, e.g.:
#
#   nix run .#gauntlet -- run-all --clean
#
# `run`/`run-all` need a Neovim binary (GAUNTLET_NVIM_BIN, defaulting to
# the Nix-provided neovim) and --diver-lua (never guessed: gauntlet
# refuses to run without it, so pass it explicitly or set DIVER_LUA).
# --clean removes the work dir only when every task passes.
#
# Report: reports/phlow-gauntlet-<UTC timestamp>.md (gitignored).

need_bin cargo cargo
need_bin git git
require_repo_root
isolated_cargo_env
report_begin

report_section "build gauntlet"
echo "== cargo build -p phlow-gauntlet --locked =="
if ! cargo build --locked -p phlow-gauntlet --bin gauntlet \
    >"${PHLOW_SCRATCH}/build.log" 2>&1; then
  echo "FAIL build gauntlet"
  report_line "result: **FAIL** — could not build the gauntlet binary."
  report_line '```'
  tail -30 "${PHLOW_SCRATCH}/build.log" >>"$REPORT_FILE"
  report_line '```'
  verify_cleanup >/dev/null 2>&1 || true
  exit 1
fi
echo "PASS build gauntlet"
report_line "result: **PASS**"
BIN="${CARGO_TARGET_DIR}/debug/gauntlet"

# Neovim for task runs: explicit env wins, else the Nix neovim.
# DIVER_LUA is never defaulted — gauntlet errors clearly without it.
export GAUNTLET_NVIM_BIN="${GAUNTLET_NVIM_BIN:-$(command -v nvim)}"
echo "nvim: ${GAUNTLET_NVIM_BIN}"
report_line "nvim: \`${GAUNTLET_NVIM_BIN}\`"

if [ "$#" -eq 0 ]; then
  set -- list
fi

report_section "gauntlet run"
echo "== gauntlet $* =="
if "$BIN" "$@" >"${PHLOW_SCRATCH}/gauntlet.log" 2>&1; then
  echo "PASS gauntlet $*"
  report_line "result: **PASS**"
  rc=0
else
  rc=$?
  echo "FAIL gauntlet $* (exit $rc)"
  report_line "result: **FAIL** (exit $rc)"
fi
report_line '```'
tail -40 "${PHLOW_SCRATCH}/gauntlet.log" >>"$REPORT_FILE"
report_line '```'

verify_cleanup || rc=1
verify_tree_clean || rc=1

report_section "summary"
report_line "exit: ${rc}"
exit "$rc"
