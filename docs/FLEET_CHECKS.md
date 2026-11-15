# Fleet checks as named checks (PH-12) and the torogao seat (PH-10)

Fable plan D-1266, rung 2 (2026-11-14). PH-10 gives phlow a seat on
one estate workspace — trusted, with named checks, a reachable model.
PH-12 turns the fleet's three standing gates into approved named
checks on that seat: theodolite zero, the pinned-toolchain gate, and
the hash-twice freeze. Each check is a small Python program in
`tools/fleet-checks/`, admitted to the seat by exact argv in the
operator's hand-written config — the only way a check ever becomes
approved in phlow.

## The seat (PH-10)

- Workspace: a dedicated clean worktree of NTRanch/torogao,
  `/home/phaedrus/.local/share/pax/torogao-ph10-wt-20261114` on
  branch `pax/ph10-seat-20261114` at cde6a69 — the commit the plan's
  probe read. The live checkout under `~/.GH/NTRanch/torogao` is
  factory state (dirty tree, its own producer artifacts) and is
  never written by the seat; the worktree shares its repository but
  no working-tree state.
- Operator config: `~/.config/phlow/torogao.toml` (operator's home,
  passed with `-c`). It names the workspace, the rose endpoint, the
  trio role models, and the three checks below. Trust is granted by
  the operator's `--trusted` flag per invocation; phlow rejects a
  `trusted` key in TOML by design, and this config does not try.
- Producer note: the workspace's `.bacon-locations` producer
  (`bacon --headless -j bacon-ls`, the estate prefs job) needs the
  CUDA toolkit on its PATH (`/opt/cuda/bin`, `CUDA_PATH=/opt/cuda`)
  or the cudarc build script panics and no locations are exported.
  The theodolite check fails closed until the producer has run.

## The three checks (PH-12)

All three print one verdict line per fact and share exit codes:
0 pass, 1 the gate's condition is false, 2 unproven (a precondition
or the measuring instrument failed — never a pass).

### `theodolite_zero` — kind `diagnostics`

```
python3 tools/fleet-checks/theodolite_zero.py \
  --workspace <ws> Cargo.toml crates/torogao-osc/src/lib.rs
```

For each target, the bridge's diagnostics owner for the file's
language (rs → bacon-ls, toml → taplo, lua → lua-language-server,
py → jedi-language-server, ron → ron-ls) must report zero
error-severity diagnostics. A zero is only believed from a proven
session: for each non-Rust route in the run the script first plants
a syntax-error control inside the workspace and requires the bridge
to report it (the estate's positive-control rule). The Rust route
has no in-band control — bacon-ls reads the producer's locations —
so the script instead enforces the producer-first precondition:
`.bacon-locations` must exist and be at least as fresh as every
Rust target, or the check exits 2 rather than reading a stale zero.

### `toolchain_pin` — kind `check`

```
python3 tools/fleet-checks/toolchain_pin.py --workspace <ws>
```

The workspace's recorded pin (`rust-toolchain.toml` channel, or the
legacy file) must resolve to the pinned toolchain itself: rustup's
own `which` locates the pinned rustc, and the workspace-resolved
`rustc --version` must be byte-identical to it. All invocations use
the cargo-home binaries — PATH's rustc on primo is a system shim
that ignores toolchain selection (it answered
`rustc +nightly-2026-10-30 --version` with the system version and
exit 0, measured 2026-11-14), so a PATH-first check could pass on
the wrong toolchain. Automatic installs are disabled: a pin that is
not installed fails, never downloads.

### `tree_freeze` — kind `check`

```
python3 tools/fleet-checks/tree_freeze.py \
  --workspace <ws> --freeze-file ~/.config/phlow/freezes/torogao-ph10.tree.json
```

A deterministic hash of the source tree (sorted paths; `.git`,
`target`, `__pycache__`, the theodolite control dir and
`.bacon-locations` excluded; symlinks hashed as their target text,
never followed) is computed twice in one run; the two computations
must agree, and the agreed hash must equal the freeze record. The
record lives outside the workspace, on the operator's side, so the
measured tree cannot rewrite its own baseline. A first run with no
record records the freeze and passes — that recording is the
freeze-taking act.

## Gate evidence (recorded 2026-11-14, rung 2)

PH-12's gate: each check returns the same verdict as the hand-run
gate on three recorded cases.

- theodolite_zero, hand-run bridge diagnostics vs the named check:
  (1) seat root Cargo.toml (taplo): hand 0 errors — check PASS.
  (2) crates/torogao-osc/src/lib.rs (bacon-ls): hand 0 errors —
  check PASS. (3) crates/torogao-gen/src/sd/attention.rs (bacon-ls):
  hand 1 error (E0433, `candle_flash_attn` unresolved — a real
  pre-existing defect at this commit) — check FAIL, exit 1, same
  diagnostic. The gen file is the Rust route's positive control: the
  route that reports its error reports the osc file's zero.
- toolchain_pin, hand-read `rustc --version` in the workspace
  (1.101.0-nightly, 92b5b8c87 2026-10-29) vs the named check:
  (1) the seat (pin nightly-2026-10-30) — PASS, identical version
  string. (2) a fixture pinning nightly-1999-01-01 — FAIL, exit 1.
  (3) resolution overridden away from the pin (RUSTUP_TOOLCHAIN)
  — FAIL, exit 1 (suite case `toolchain_rejects_resolution_mismatch`).
- tree_freeze on the seat itself: (1) recorded 2026-11-15T08:16Z —
  638 files, sha256 d7b4a21e8b6811cbf99fb34406d3dc896f78c08ff58adad3a5cf384d103177eb. (2) unchanged tree — PASS (also inside the seat's
  first live run, where all three checks ran green). (3) README.md
  mutated by one byte — FAIL naming the recorded hash; restored
  via git — PASS, which is byte-level proof of the restore.

Seat status (PH-10's gate), from `phlow -c torogao.toml --trusted
status`: `"trusted": true`, checks = the three named checks above
(all required), workspace = the seat worktree, models = the trio.
The same command without `--trusted` reports `"trusted": false`.
Model reachability (status never contacts the model, by design):
a minimal live run on the seat (all roles pinned to the smallest
trio model, hf-granite-4.2-8b-tools-64k, via the operator's
`--model` override) completed ok — 6 model calls, reviewer
approved, `changed_files: []` on a read-only task, all three named
checks green inside the run's own verification, and a PH-1 outcome
row written.

The scripts' behaviour contracts are pinned by
`crates/phlow-checks/tests/fleet_checks.rs` (validation and
adversarial cases per check; environment-gated cases skip off
primo).
