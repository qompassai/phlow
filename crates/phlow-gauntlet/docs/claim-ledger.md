# Claim ledger (gate zero)

## Why

2026-09-28, wave 28: a wave summary claimed tasks 170-177 were wired into
`run_task` dispatch and metadata. Git history proves the arms never existed
in any commit; `run_task("task-170")` returned `UnknownTask`. The summary
was generation, not observation — and the integration pass caught it by
accident, not by design.

## Rule

A wave is complete when the *verifier* observes the artifact, never when the
producer's summary says so. Summaries are hints for where to look, not
evidence. The fix is never "tell them not to lie"; it is making the lie
structurally impossible to profit from.

## Procedure

1. Producer finishes the wave's code commit `C`, then writes
   `claims/<wave>.json`:
   `{"wave": "wave-32", "commit": "<C>", "claims": ["task-197", ...]}`,
   and commits the ledger as a child of `C`. (The ledger cannot name its
   own commit: the hash covers the ledger.)
2. Verifier checks out the ledger commit.
3. Verifier runs `gauntlet verify-claims claims/<wave>.json` FIRST, before
   build/clippy/tests. It asserts every claimed id names a real task, is
   wired in both dispatch (`run_task`) and metadata (`task_meta`), and
   that the ledger's commit is an ancestor of the checkout.
4. Only then do the expensive gates run.

A prose wave summary without a ledger is rejected: no ledger, no completion.

## Invariants (enforced in code)

- `tasks::is_wired(id)`: true only if `id` resolves in BOTH tables.
  Dispatch is probed without running anything.
- Unit test `every_listed_task_is_wired`: every `TASK_IDS` entry is wired.
  This failed for task-170..177 before the fix commit.
- `gauntlet list` prints `UNWIRED` and exits 1 while any listed id is
  unwired — the silent skip that hid the incident is gone.
- `verify-claims` fails closed: unknown ids, unwired claims, duplicates,
  empty ledgers, and ledgers whose commit is not an ancestor of the
  verifier's checkout are all rejections, never warnings. The commit
  binding fails rather than guessing when git is unavailable.
