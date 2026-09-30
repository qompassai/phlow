# phlow-version-audit: quick workspace version-consistency check.
# Wraps scripts/publish/version-audit.sh in the hermetic flake
# environment. Read-only: touches nothing in the tree.
#
#   nix run .#version-audit
#
# Report: reports/phlow-version-audit-<UTC timestamp>.md (gitignored).

need_bin git git
require_repo_root
# No cargo needed: the audit is pure manifest parsing. Still isolate,
# so the app shape matches the others and temp logs are cleaned.
isolated_cargo_env
report_begin

report_section "version audit"
echo "== version audit =="
if ./scripts/publish/version-audit.sh >"${PHLOW_SCRATCH}/audit.log" 2>&1; then
  echo "PASS version audit"
  report_line "result: **PASS**"
  cat "${PHLOW_SCRATCH}/audit.log" >>"$REPORT_FILE"
  rc=0
else
  rc=1
  echo "FAIL version audit"
  report_line "result: **FAIL**"
  report_line '```'
  cat "${PHLOW_SCRATCH}/audit.log" >>"$REPORT_FILE"
  report_line '```'
fi

verify_cleanup || rc=1
verify_tree_clean || rc=1

report_section "summary"
report_line "exit: ${rc}"
exit "$rc"
