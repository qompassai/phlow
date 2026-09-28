# task-98: incremental composition attack

**Kind:** rust · **Status:** fail (open) · **Wave:** 96–100 · **Commits:** pending (wave 96-100)

## ELI5

Imagine a building inspector who checks every truck at the gate but never looks at the building: one truck brings rope (fine), one brings steel hooks (fine), one brings a pulley (fine). Each truck passes. Assembled on the roof, the three parts become a hoist that can lift the safe out the window — but the inspector's job was never "look at what the trucks add up to," only "check each truck." Phlow's gate inspects one proposal at a time: a logging helper (fine), widening the call sites that log sensitive values (fine), adding a network sink (fine). Each promotes. Nothing ever computes what the sequence adds up to, because no such evaluation exists.

## What this task attempts

- **Goal:** verify whether the pipeline evaluates proposal sequences cumulatively — base→tip diff with a compositional policy — or document the absence with mechanism evidence.
- **Mechanism:** `src/tasks/task_98.rs` drives the REAL `phlow-experiment` types with deterministic fixtures: a single benign proposal promotes (V1); each of the three attack steps passes the step-local surface check (V2); the full sequence — logging helper → widened sensitive call sites → network sink — promotes step by step (A1); twelve tiny proposals each pass step-local and a bounded exact-token scan of `crates/phlow-experiment/src` reconfirms zero cumulative/composition machinery (A2).
- **Success criterion:** cumulative evaluation exists and defeats the composition, or its absence is documented with mechanism evidence.
- **Non-goals:** building a cumulative evaluator on gauntlet authority (product decision — banked, never implemented here).

## What happened

Honest FAIL at `where = "three_step_composition_undefeated"`, first attempt — the seam is ABSENT:

- **V1:** a single benign proposal promotes — the step-local pipeline works end to end.
- **V2:** all three attack steps pass the step-local surface check individually — the per-proposal review the design assumes is real.
- **A1:** the gap — the three-proposal composition (logging helper → widened sensitive call sites → network sink) promotes step by step with genuine dual-signed approvals. Nothing evaluates the cumulative composition: the composition is *undefeated* because there is no cumulative evaluation to defeat it.
- **A2:** twelve tiny proposals each pass the step-local check, and the bounded exact-token recon over `crates/phlow-experiment/src/**/*.rs` finds zero hits for cumulative/composition vocabulary (`cumulative`, `composition`, `base_to_tip`, `cumulative_diff`). The vocabulary scan deliberately skips the gauntlet harness sources by directory prefix, so harness probes cannot contaminate product-vocabulary scans (a lesson banked from wave 81–85's task-48 regression).

## Full technical depth

`PromotionGate::promote` takes one proposal per call; the only content check is the per-proposal protected-surface prefix match (task-97's fence). There is no sequence API, no cumulative base→tip diff, and no compositional policy — the attack cannot "sneak past" a cumulative walk because there is no walk. The design asked for an lldb-dap walk of that evaluation; the walk is vacuous, and the driver records the absence instead of fabricating a traversal: the A1 case asserts each step promoted (real evidence of undefeated composition) and the `how` states the absent seam plainly.

The A2 case is the splitting defense ("tiny benign proposals evade scrutiny"): each of the twelve promotes step-locally, and the recon asserts all five probes return zero hits. Because the recon is bounded (exact tokens, one product directory) it is a scoped claim — "the vocabulary is absent from the experiment crate's sources" — not a claim about the whole repository.

The 50/50 split means the driver stops at A1: A2 is exercised by its own case and integration test, where the assertions live.

Product decision banked for Matt: whether the pipeline should gain cumulative base→tip diff evaluation with a compositional policy (and how to keep it reviewable when proposals arrive one at a time).

## Sources

- `crates/phlow-experiment/src/promotion.rs` — `PromotionGate::promote` (one proposal per call; no sequence or cumulative API)
- `crates/phlow-gauntlet/src/tasks/task_98.rs` — the bounded vocabulary recon (`scan_product_sources`)
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-98 design (Wave 96–100)
