# Overnight leftover reconciliation — 2026-10-12 (Pax)

Reconciled the 2026-10-11 open-items list against live `origin/main`
(`a5332b2` at start of pass). Evidence and dispositions:

- **0195b53** ("gauntlet: add --clean flag and deterministic nix dev
  shell"): SUPERSEDED. No branch contains it; its content is on main —
  the gauntlet binary documents/implements `--clean`, and
  `crates/phlow-gauntlet/flake.nix` + `flake.lock` exist at `a5332b2`.
  A twin commit (351dd33, same subject) carried the content in the
  other lineage. No action needed; do not cherry-pick it.
- **CS1 landing**: ALREADY LANDED on main. The five rename files
  (phlow-agent best_of_n/system1, phlow-inference lib/projection,
  docs/ARCHITECTURE) carry the CS1 wording; remaining `CLM` tokens in
  them are only `/tmp/CLM/...` provenance paths, kept deliberately
  per the naming note in system1.rs. The uncommitted five-file diff
  in the sandbox checkout is a stale duplicate (two files
  byte-identical to main; three are older variants without the
  2026-10-08 layout pins) — committing it would regress main. Left
  untouched; the sandbox checkout's disposition is Matt's ruling.
- **Supervisor run (tasks 101–120)**: MOOT. Wave 1 (Seat A) found a
  slot collision before writing anything and stopped clean, per
  `~/supervisor-wave1-progress.md`: tasks 101–105 (and through 120)
  already exist and are committed on main (318 task modules, 195
  docs reports at `a5332b2`). The run's premise — that 101–130 had
  not run — was wrong. Nothing to resume.
- **Calibration crate**: NOT BUILT — activation check fails. Of the
  design's data sources, none exist as data on primo: no
  DecisionTrace corpus files (capture code landed in `9d764ca`), no
  approval-history dataset, no feedback-store data (`~/.config/phlow`
  holds only config.toml). Per the design's own stop rule, the crate
  waits on those inputs; a gauntlet trace run plus Matt's approval
  history are the unblocking artifacts.
- **Baseline note**: full suite at `a5332b2` needs
  `GAUNTLET_NVIM_BIN=/usr/bin/nvim` on primo (the compiled-in default
  path `~/workspace/tools/neovim-nightly` is the sandbox layout and
  does not exist on primo). Without it task_01 fails on a missing
  binary. With it, the only baseline failures were task_78's
  unclassified attention seams — fixed in `99b443f`.
