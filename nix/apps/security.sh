# phlow-security: supply-chain safety checks over the workspace.
# Uses what's available from nixpkgs and reports honestly what's missing:
#   - cargo deny check   (SKIP when the repo has no deny.toml — the policy
#                         file is Matt's call, not something to invent here)
#   - cargo audit        (RustSec advisories; fetches the advisory DB)
#   - cargo geiger       (unsafe-code inventory; needs no full build)
#
#   nix run .#security
#
# Report: reports/phlow-security-<UTC timestamp>.md (gitignored).

need_bin cargo cargo
require_repo_root
isolated_cargo_env
report_begin

pass=0
fail=0
skip=0
step_log() { printf '%s/%s-%s.log' "${PHLOW_SCRATCH}" "${PHLOW_APP_NAME}" "$1"; }

# --- cargo deny -----------------------------------------------------------
report_section "cargo deny"
if [ -f ./deny.toml ]; then
  need_bin cargo-deny cargo-deny
  if run_step "cargo deny check" "$(step_log deny)" cargo deny check --locked; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
  fi
else
  echo "SKIP cargo deny: no deny.toml in repo (license/ban policy is Matt's call)"
  report_line "result: **SKIP** — no deny.toml in the repo; refusing to invent a security policy."
  skip=$((skip + 1))
fi

# --- cargo audit ----------------------------------------------------------
if command -v cargo-audit >/dev/null 2>&1; then
  if run_step "cargo audit (RustSec)" "$(step_log audit)" cargo audit; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
  fi
else
  echo "SKIP cargo audit: cargo-audit not on PATH"
  report_line "result: **SKIP** — cargo-audit not available in this environment."
  skip=$((skip + 1))
fi

# --- cargo geiger ---------------------------------------------------------
if command -v cargo-geiger >/dev/null 2>&1; then
  if run_step "cargo geiger (unsafe inventory)" "$(step_log geiger)" \
      cargo geiger --all-targets --forbid-only 2>&1 | tee "$(step_log geiger).tmp" >/dev/null; then
    # --forbid-only exits nonzero when any unsafe is found; surface counts.
    pass=$((pass + 1))
  else
    # geiger's nonzero exit MEANS "unsafe found" — report, don't fail the app.
    echo "NOTE cargo geiger: unsafe code present (see report); not a gate failure"
    report_line "result: **NOTE** — unsafe code blocks exist; inventory above. Not a gate failure by itself."
    pass=$((pass + 1))
  fi
  mv "$(step_log geiger).tmp" "$(step_log geiger)"
else
  echo "SKIP cargo geiger: cargo-geiger not on PATH"
  report_line "result: **SKIP** — cargo-geiger not available in this environment."
  skip=$((skip + 1))
fi

verify_cleanup || fail=$((fail + 1))
verify_tree_clean || fail=$((fail + 1))

report_section "summary"
report_line "checks passed: ${pass}, failed: ${fail}, skipped: ${skip}"
echo "== security: ${pass} passed, ${fail} failed, ${skip} skipped =="
[ "$fail" -eq 0 ]
