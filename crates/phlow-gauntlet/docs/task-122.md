# task-122: capability-based risk escalation

**Kind:** nvim-lua · **Status:** fail (open, diver gap: launch classifies nothing) · **Wave:** 121–125 · **Commits:** pending (wave 121-125)

## ELI5

Before starting a job, the harness is supposed to ask the worker "what
can you do — can you reach the network?" and write down the answer as a
risk level: network-capable means risky, local-only means less risky. If
the worker stammers or lies about its shape, the answer is "risky"
anyway — better safe. Today the harness never asks the question at all:
it hands the job to the worker without writing anything down. "Correct"
means: every launch asks (safely, so a broken answer can't crash it),
writes down network-vs-local, and checks the rulebook.

## What this task attempts

- **Goal:** prove launch classifies risk from the adapter's `probe()`
  capabilities and consults `policy.decide` — or record precisely that it
  does neither.
- **Mechanism:** diver's `ai.harness` — `supervisor.lua` `launch` (line
  182), `adapter.lua` `M.probe` (the strict capability contract),
  `types.CAPABILITY_KEYS` — via the driver
  `crates/phlow-gauntlet/lua/gauntlet/task_122.lua`.
- **Success criterion:** `remote = true` → request risk `'network'`;
  `remote = false` → `'process'`; `probe()` raising → `'network'`
  (fail-closed via pcall); malformed caps (`remote = nil`, non-table,
  missing probe fn, truthy non-boolean) → `'network'`; `decide` consulted
  at launch time.
- **Non-goals:** vetting whether a probe tells the truth. The lying-probe
  trust boundary (an adapter attests `remote = false` while doing network
  I/O) is banked from task-118: classification trusts attestation by
  design; vetting is adapter-vetting work, not a launch-path fix.

## What happened

Fail, open diver gap (Fix-3 detail absent) — on the first attempt. Launch
builds no policy request at all:

- `remote-true` / `remote-false`: stub adapters with scripted strict-
  boolean probe tables launch fine; the probe-call spy reads 0 and the
  decide-call spy reads 0 — launch resolves the adapter by name and calls
  `chosen.start` directly (supervisor.lua line 182 has no probe call, no
  risk request, no `decide` call).
- `probe-raises`: a `probe()` that errors is never even invoked by launch
  — there is no pcall and no fail-closed fallback; the broken probe is
  invisible *and* unclassified.
- `malformed-probe`: the strict contract exists and works —
  `adapter.lua` `M.probe` rejects `remote = 'yes'`, non-table caps, and
  `remote = nil` ("must be a boolean" / "must return a table"), and
  `register_adapter` rejects a probeless adapter — but launch never
  invokes `M.probe` and classifies nothing either way.

All four scenarios report `fail` with `where = "no-launch-policy-request"`.

## The fix — what changed and why

No fix — this is a documented diver finding (Phase-2 Fix 3 detail), and
diver findings are never fixed under gauntlet authority. The gauntlet-side
work was getting the evidence right:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_122.lua` (new) —
  four scenarios, each launching a real run against a scripted-probe stub
  adapter with a probe-call spy and a `policy.decide` spy, plus direct
  characterization of the strict probe contract.
- **Why:** the defect is the missing classification, not a broken probe
  contract — the driver separates the contract that works (`M.probe`'s
  strict booleans) from the launch path that never calls it.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  line 182 (`launch` — no probe/decide/risk references),
  `~/workspace/repos/diver/lua/ai/harness/adapter.lua` `M.probe` (strict
  contract, unused by launch), `~/workspace/repos/diver/lua/ai/harness/types.lua`
  `CAPABILITY_KEYS`.
- **Validation agents:** the 2 validation tests
  (`remote_true_builds_no_network_request`,
  `remote_false_builds_no_process_request`) assert both spies read zero
  and pin the `'network'`/`'process'` acceptance.
- **Adversarial agents:** the 2 adversarial tests
  (`probe_raises_no_failclosed_fallback`, `malformed_probe_no_classification`)
  assert the missing pcall fallback and the unclassified malformed shapes.

## Full technical depth

`launch(sup, run, adapter_name)` resolves the named adapter via
`registry.get_adapter` and calls `chosen.start(run, sup.sink)` directly.
`adapter.probe` is called only by `adapter.negotiate`, which runs solely
when *no* adapter name is given — and even then its output feeds adapter
*selection*, never a policy request. The monkey-patched `policy.decide`
spy confirms zero consultations across all launches. `M.probe` (pcall'd,
strict boolean per `CAPABILITY_KEYS`) is the right classification input
mechanism and already exists — it is simply never invoked on the launch
path.

Phase-2 acceptance (banked, diver-owned): launch pcall()s `probe()`,
classifies `remote = true` → risk `'network'` else `'process'`, maps every
malformed shape (raising probe, `remote = nil`, non-table caps, missing
probe fn, truthy non-boolean) → `'network'` fail-closed, consults
`decide` at launch time, and a deny lands the run in `failed` with the
policy reason recorded.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/supervisor.lua` line 182
  (`launch` — goes straight to `chosen.start`, no probe/decide/risk).
- Primary: `~/workspace/repos/diver/lua/ai/harness/adapter.lua` `M.probe`
  (strict capability contract, pcall'd, unused by launch).
- Primary: `~/workspace/repos/diver/lua/ai/harness/types.lua`
  `CAPABILITY_KEYS` (streaming, cancellation, resume, permissions,
  artifacts, remote, tools).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_122.lua` (four
  scenarios, all `where = "no-launch-policy-request"`).
- Tests: `crates/phlow-gauntlet/tests/task_122.rs` (2V/2A).
- Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no
  Phase-2 branch exists).
