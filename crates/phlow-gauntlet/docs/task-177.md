# task-177: pairing ceremony integration and license audit

**Kind:** rust · **Status:** pass · **Wave:** 28 · **Commits:** pending (wave 170-177)

## ELI5

This is the end-to-end walkthrough: the whole pairing ceremony
played start to finish like a real user would do it — the daemon
makes a code, the phone shows it back a minute later, the daemon
registers the phone, and the phone's later calls authenticate.
Every step is written down in a transcript, and then the transcript
is replayed against a *different* daemon to prove the recording is
worthless to a thief: every step fails, because each code is sealed
to the daemon that made it. The second case is a paperwork check:
every Rust file adapted from Ghostex must start with the exact
three-line attribution header, and the driver verifies all 17 of
them byte for byte (the shared module, the 8 drivers, the 8
integration tests).

## What this task attempts

- **Goal:** the legitimate multi-step ceremony registers exactly
  the devices it should, a replayed transcript fails everywhere on
  a foreign daemon, and every adapted file carries the exact
  attribution header.
- **Mechanism:** `crates/phlow-gauntlet/src/pairing.rs`
  (`Daemon::issue`/`verify`/`device_info`, instance-key binding);
  driver `src/tasks/task_177.rs`, tests `tests/task_177.rs`.
- **Success criterion:** two cases pass — V1: issue at T+0,
  present at T+60 → `PairingOk` with `device_info` showing
  label `pixel-9` and the right timestamp, the paired device's
  later calls authenticate while unknown ids are refused, a
  recorded transcript of the ceremony, and every transcript step
  replayed against a fresh daemon refused as `Authenticity`;
  A2: all 17 adapted `.rs` files begin with the
  exact three-line header.
- **Non-goals:** transport (loopback only), sidecar supervision
  (task 176).

## What happened

Both cases pass on primo. The ceremony case records each step
(issue/present/device_info) into a transcript, asserts the live
ceremony's outcomes, then replays the transcript against a fresh
daemon: every replayed step is refused as `Authenticity` — the
transcript is bound to the daemon that minted it. The license case
checks the exact three-line header on all 17 adapted files (the
shared module, the 8 drivers, the 8 integration tests). Gates:
`cargo build` clean, 2/2 integration tests, 64/64 lib tests,
`cargo fmt --check` clean, `cargo clippy --all-targets -D warnings`
clean.

## Where it went wrong

One gate-loop iteration, in the driver.

- **Stage:** first `cargo build` on primo.
- **Symptom:** `error[E0618]: expected function, found
  'pairing::Daemon'` at `src/tasks/task_177.rs:162`.
- **Evidence:** the case bound `let daemon = daemon()?;`
  (shadowing the `daemon()` helper) and later called `daemon()`
  again for the fresh replay target.
- **Root cause:** the local variable shadowed the helper function;
  the second call resolved to the variable.

A pre-gate review also questioned the license case's relative file
paths (`src/...`, `tests/...`): cargo runs integration-test
binaries with the working directory set to the package root, so the
paths resolve — and the passing test is the proof. No change was
needed; the driver's doc comment records the assumption.

## The fix — what changed and why

- **Changed:** `src/tasks/task_177.rs` — the helper renamed
  `daemon()` → `new_daemon()`; both call sites updated.
- **Commit:** pending (wave 170-177)
- **Why:** the shadowing was the whole bug; renaming the helper
  (called twice) is smaller than renaming the variable (used a
  dozen times). The other seven drivers call their helper once and
  never hit this.
- **Source:** `rustc` E0618.
- **Validation agents:** primo `cargo build` clean after the fix.
- **Adversarial agents:** n/a for this iteration.

- **Changed:** `src/tasks/task_177.rs` — the 119-line ceremony
  case split at meaningful contracts (`run_ceremony`,
  `replay_transcript_foreign`) to satisfy the ≤70-physical-line
  rule; the replay helper returns `(refused, fresh_devices)` so
  the report stays honest on the failure path.
- **Commit:** pending (wave 170-177)
- **Why:** the worktree AGENTS.md targets changed functions at
  ≤70 physical lines.
- **Source:** worktree `AGENTS.md` ("Tiger Style and
  performance").
- **Validation agents:** primo full gate sweep after the split.
- **Adversarial agents:** n/a for this iteration.

## Full technical depth

The ceremony case scripts a `ManualClock` from T+0: issue a code
for `pixel-9`, present it at T+60 with the correct secret →
`PairingOk`; `device_info` returns `("pixel-9", T+60)`; the
device's later `device_call`s authenticate and unknown ids are
refused. The present step is recorded onto a transcript (op name,
code, secret, timestamp). Then a *fresh* daemon — a different
32-byte instance key drawn from `/dev/urandom` — replays the
transcript: the `verify` fails at the MAC gate as `Authenticity`,
because the HMAC binds the code to the minting daemon's key. The
transcript is therefore useless to anyone who steals it — it
cannot be replayed elsewhere, and on the home daemon the code is
already consumed.

The license case enumerates the 17 adapted files — `src/pairing.rs`,
the 8 drivers, the 8 integration tests — and
asserts each begins with the exact three lines:

```
// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607...
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
```

The ceremony/transcript design and the header audit are phlow's
own; the pairing-code mechanics they exercise adapt Ghostex
`server/src/remote_access/{pairing_code,pair_device}.rs`.

## Sources

- Ghostex `server/src/remote_access/pairing_code.rs` and
  `pair_device.rs` @ `c91146607205ac49303d1bcfe2fd6f9a86741500` —
  pairing flow concept (primary).
- `crates/phlow-gauntlet/src/pairing.rs` — `issue` (~line 328),
  `verify` (~line 435), `device_info` (~line 574).
