# phlow-publish-dryrun: version audit + topological order + cargo publish
# --dry-run for every crate. Wraps the canonical scripts/publish/
# helpers in the hermetic flake environment (pinned toolchain, isolated
# target dir, markdown report). Never uploads anything.
#
#   nix run .#publish-dryrun
#
# Report: reports/phlow-publish-dryrun-<UTC timestamp>.md (gitignored).

need_bin cargo cargo
need_bin git git
need_bin python3 python3
require_repo_root
isolated_cargo_env
report_begin

pass=0
fail=0
failed_crates=()

report_section "version audit"
echo "== version audit =="
if ./scripts/publish/version-audit.sh >"${PHLOW_SCRATCH}/audit.log" 2>&1; then
  echo "PASS version audit"
  report_line "result: **PASS**"
  pass=$((pass + 1))
else
  echo "FAIL version audit — run version-bump.sh <VERSION> first"
  report_line "result: **FAIL** — workspace versions inconsistent; run version-bump.sh <VERSION> first."
  report_line '```'
  cat "${PHLOW_SCRATCH}/audit.log" >>"$REPORT_FILE"
  report_line '```'
  fail=$((fail + 1))
fi

report_section "publish order"
order="$(./scripts/publish/topo-order.sh)"
report_line '```'
printf '%s\n' "$order" >>"$REPORT_FILE"
report_line '```'

report_section "cargo publish --dry-run per crate"
while IFS= read -r crate; do
  [ -z "$crate" ] && continue
  log="${PHLOW_SCRATCH}/dryrun-${crate}.log"
  echo "-- dry-run $crate"
  if cargo publish --dry-run --locked -p "$crate" >"$log" 2>&1; then
    echo "  DRY-RUN OK $crate"
    report_line "- ${crate}: **OK**"
    pass=$((pass + 1))
  else
    echo "  DRY-RUN FAIL $crate"
    report_line "- ${crate}: **FAIL**"
    failed_crates+=("$crate")
    fail=$((fail + 1))
  fi
done <<EOF
$order
EOF

if [ "${#failed_crates[@]}" -gt 0 ]; then
  report_section "failed crates"
  for c in "${failed_crates[@]}"; do
    report_line "### ${c}"
    report_line '```'
    tail -15 "${PHLOW_SCRATCH}/dryrun-${c}.log" >>"$REPORT_FILE"
    report_line '```'
  done
fi

verify_cleanup || fail=$((fail + 1))
verify_tree_clean || fail=$((fail + 1))

report_section "summary"
report_line "checks passed: ${pass}, failed: ${fail}"
echo "== publish-dry-run: ${pass} passed, ${fail} failed =="
[ "$fail" -eq 0 ]
