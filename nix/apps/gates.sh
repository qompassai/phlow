# phlow-gates: the four publish gates from scripts/publish/gates.sh,
# run hermetically: pinned Nix toolchain, isolated cargo target dir
# (removed afterwards), --locked for determinism, markdown report.
#
#   nix run .#gates
#
# NOTE: the first run compiles the whole workspace from scratch in the
# isolated target dir; expect it to take a while. The report lands in
# reports/phlow-gates-<UTC timestamp>.md (gitignored).

need_bin cargo cargo
need_bin rustc rustc
need_bin git git
require_repo_root
isolated_cargo_env
report_begin

pass=0
fail=0
step_log() { printf '%s/%s-%s.log' "${PHLOW_SCRATCH}" "${PHLOW_APP_NAME}" "$1"; }

if run_step "cargo build --workspace --locked" "$(step_log build)" \
    cargo build --workspace --locked; then
  pass=$((pass + 1))
else
  fail=$((fail + 1))
fi

if run_step "cargo clippy --workspace --all-targets --locked" "$(step_log clippy)" \
    cargo clippy --workspace --all-targets --locked; then
  warns="$(grep -c '^warning' "$(step_log clippy)" || true)"
  report_line "clippy warnings: ${warns}"
  if [ "$warns" -eq 0 ]; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "FAIL clippy: ${warns} warnings (gate requires zero)"
    report_line "result: **FAIL** — ${warns} warnings, gate requires zero"
  fi
else
  fail=$((fail + 1))
fi

if run_step "cargo fmt --all -- --check" "$(step_log fmt)" \
    cargo fmt --all -- --check; then
  pass=$((pass + 1))
else
  fail=$((fail + 1))
fi

if run_step "cargo test --workspace --locked" "$(step_log test)" \
    cargo test --workspace --locked; then
  passed="$(grep -oE 'test result: ok\. [0-9]+ passed' "$(step_log test)" \
    | grep -oE '[0-9]+' | paste -sd+ - | bc || echo 0)"
  report_line "tests passed: ${passed:-0}"
  pass=$((pass + 1))
else
  fail=$((fail + 1))
fi

verify_cleanup || fail=$((fail + 1))
verify_tree_clean || fail=$((fail + 1))

report_section "summary"
report_line "gates passed: ${pass}, failed: ${fail}"
echo "== gates: ${pass} passed, ${fail} failed =="
[ "$fail" -eq 0 ]
