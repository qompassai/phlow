# PH-2 — Budget proposal (proposal only; the operator applies)

Generated from PH-1 `run_outcomes` rows joined with the corpus manifest. Nothing in this file is applied by any tool: budgets change only when the operator edits the config by hand (PH-9: the loop may never widen its own budgets).

## Method

- Per task class, fit on the fit split's **successful** runs only: nearest-rank 95th percentile of recorded usage, times the margin 5/4. With the small samples here the nearest rank is typically the observed maximum — the sample size is stated beside every number.
- `max_iterations` is fitted from recorded **model calls** (the outcome row carries no separate iteration counter); `max_cycles` from cycles; `max_tool_calls` from tool calls; the timeout from wall time, rounded up to whole seconds.
- Held-out replay: a run is *cut* when any recorded usage exceeds the proposed cap. Wasted-turn reduction is measured on held-out **failed** runs, in tool calls: usage above the proposed cap is what the cap would have saved.
- A class with fewer than 3 successful fit-split runs gets **no proposal** — a distribution that thin cannot carry a budget.
- Outcome rows without a corpus label: 0 (excluded, counted, never silently fitted).

## Proposed budgets

### Task class: analysis

Labelled runs: 12 (fit 6, held-out 6)

Fitted from 6 successful fit-split runs.

| Budget | Proposed |
|---|---|
| max_iterations | 27 |
| max_cycles | 2 |
| max_tool_calls | 23 |
| run timeout | 310000 ms (310 s) |

Held-out replay for this class: 6 successes, 0 cut; 0 failures, wasted tool calls saved 0 of 0 (0.0%).

To apply (operator's hand, in the workspace config's `[agent]` table):

```toml
max_iterations = 27
max_cycles = 2
max_tool_calls = 23
```

The fitted run wall budget is 310 s; the config's per-request timeout is a different knob and is not changed by this proposal.

### Task class: code_fix

Labelled runs: 12 (fit 6, held-out 6)

Fitted from 5 successful fit-split runs.

| Budget | Proposed |
|---|---|
| max_iterations | 39 |
| max_cycles | 3 |
| max_tool_calls | 33 |
| run timeout | 1616000 ms (1616 s) |

Held-out replay for this class: 6 successes, 0 cut; 0 failures, wasted tool calls saved 0 of 0 (0.0%).

To apply (operator's hand, in the workspace config's `[agent]` table):

```toml
max_iterations = 39
max_cycles = 3
max_tool_calls = 33
```

The fitted run wall budget is 1616 s; the config's per-request timeout is a different knob and is not changed by this proposal.

## Held-out replay — total

- Held-out runs: 12 (12 successful, 0 failed)
- Successful runs cut by the final budgets: **0** (gate: 0)
- Wasted tool calls saved on failed runs: 0 of 0 (0.0%)
- Cycles saved: 0; wall time saved: 0 ms
