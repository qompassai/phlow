# Phlow publication TODO

Publish targets: crates.io (25 workspace crates) + GitHub release with the
Linux tarball. Runbook: `scripts/publish/RUNBOOK.md`.

## Done (agent-side)

- [x] crates.io metadata on all 25 crates (description/license/repository)
- [x] `scripts/publish/`: gates.sh, version-audit.sh, version-bump.sh,
      topo-order.sh, publish-dryrun.sh, package-linux.sh
- [x] Human-gated: tbr-crates-publish.sh, tbr-github-release.md
- [x] Publish runbook (`scripts/publish/RUNBOOK.md`)
- [x] Gates green on primo (build/clippy/fmt/test), with notes:
      - Fixed 1 clippy warning (`items_after_test_module` in
        phlow-gauntlet's task_233.rs: moved the test module to end of file)
      - Fixed 4 stale gauntlet vocabulary-scan tests tripped by the
        2026-09-29 approval-subsystem commit (scans written 2026-09-28):
        task_42 (classified ApprovalQueue.records), task_43 (classified
        task_211.rs's "not ID secrecy" doc prose), task_78 (classified
        phlow-approval in CLASSIFIED_CRATES), task_81 (classified
        redact.rs credential-scrubber fixtures)
      - gates.sh sets GAUNTLET_NVIM_BIN=/usr/bin/nvim when unset
        (gauntlet task_01 needs a Neovim binary)
      - KNOWN FLAKE (pre-existing, on main too): task_244/task_245
        child-reaping tests fail intermittently under full-workspace
        parallel load ("cannot parse integer from empty string" — the
        child is killed by the 400ms timeout before python prints its
        PID); they pass in isolation and failed identically on the
        clean tree. Not touched.

## Matt-only (human-gated)

- [ ] Pick the release version (workspace is mixed 0.1.0/0.2.0 until then)
- [ ] `scripts/publish/version-bump.sh <VERSION>`
- [ ] Update CHANGELOG.md: move Unreleased entries under `## [<VERSION>]`
- [ ] Commit the bump + CHANGELOG, verify gates one more time
- [ ] `VERSION=<VERSION> scripts/publish/tbr-crates-publish.sh`
      (needs your crates.io token via `cargo login`)
- [ ] `tbr-github-release.md`: tag `v<VERSION>`, GitHub release, docs.rs check

## Later (not this release)

- [ ] Windows cross-compile build
- [ ] AUR package
- [ ] Root LICENSE file(s) for the dual MIT/Apache-2.0 grant
      (crates carry the SPDX expression; the text files are nice-to-have)
- [ ] Decide: keep `MIT OR Apache-2.0` or move the workspace to Apache-2.0-only
