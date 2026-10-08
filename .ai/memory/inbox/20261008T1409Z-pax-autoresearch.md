# phlow-autoresearch first slice (2026-10-08)

New crate crates/phlow-autoresearch on branch pax/autoresearch-20261008 (off
trainlab tip 42d7d9d): a bounded autoresearch-style loop orchestrator wired
to phlow surfaces. Scripted proposer/evaluator only in v1 — no real training
runs; the live evaluator adapter lands with the PyTorch trainer track.

Invariants enforced in code and tests: bounded everything (iteration cap,
per-experiment and total wall-clock budgets, consecutive-failure budget);
hash-chained append-only ledger (tamper fails closed on open); change-sets
cannot escape the worktree or touch trainlab/experiment surfaces, the gate
state, or the ledger (canonical-ancestry checks; two gate violations halt);
the confirmation gate is never opened — an integration test runs the loop
beside a real ConfirmationGate and asserts byte-identical gate state.
Promotion stays phlow-experiment human-signed only.

Gates: cargo test -p phlow-autoresearch 28 passed / 0 failed (27 unit +
1 integration); clippy --all-targets -D warnings clean; fmt clean.
Design: ~/workspace/phlow-edit/autoresearch/integration-design.md.
