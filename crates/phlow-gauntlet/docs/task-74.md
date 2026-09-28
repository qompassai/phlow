# task-74: result aggregation under partial failure

**Kind:** rust (adversarial) · **Status:** fail (open) · **Wave:** 71–75 · **Commits:** pending (wave 71-75)

## ELI5

Partial-failure aggregation is "what the verdict looks like when some subagents never come back." The design wants the verdict to account for every gap explicitly: it must name exactly which contributions are missing and why, distinguish "3 of 5" from "5 of 5", treat a late result as late (recorded, never merged into the published verdict), and return a typed `insufficient_contributions` when everyone fails. Phlow's real aggregator, `phlow_council::CouncilReview`, is a votes-only tally: it aggregates the votes it is HANDED and knows nothing about the votes that never arrived. A 3-vote "3-of-5" review is `PartialEq`-identical to the same 3 votes framed as "3-of-3" — the two missing contributions appear nowhere, so downstream consumers cannot tell "3 of 5" from "5 of 5". Zero votes fails with `CouncilError::TooManyItems { field: "reviewers", max: 8 }` — no `insufficient_contributions` variant exists, and the name is misleading for the empty case ("reviewers holds more than 8 items" for zero reviewers). And a late vote arriving after aggregation silently moves a published Keep to Revise — `decision()` is a pure function of the votes slice with no published-verdict boundary, no generation token, no late marker.

## What this task attempts

- **Goal:** verify the aggregation accounts for every gap — missing contributions enumerated with causes, "3 of 5" distinguishable from "5 of 5", late results recorded-not-merged, typed `insufficient_contributions` on total failure — or document the gap.
- **Mechanism:** `src/tasks/task_74.rs` drives the REAL `phlow_council::CouncilReview` directly: all_present_aggregates (5/5 votes → Keep), partial_returns_indistinguishable (3 of 5 vs the same 3 framed complete), all_fail_no_typed_verdict (0 votes), late_arrival_silently_merges (Keep → rebuilt with the late vote → Revise).
- **Success criterion:** an aggregation envelope carrying expected reviewers, named quorum thresholds, and a late-result policy — or the gap documented as the finding (and banked for Matt as a product decision).
- **Non-goals:** inventing the aggregation envelope on gauntlet authority (it is a product decision, not a bug fix).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the gap IS the finding:

- **V1:** the complete case aggregates per contract: 5/5 votes (keep=3, revise=1, reject=1) → Keep on strict plurality. The tally mechanism works; it is the baseline the partial cases are measured against.
- **V2:** 2 of 5 contributors time out; the 3-vote partial review == the 3-vote complete review (`PartialEq`) — the missing contributions (r4, r5, timed out) appear nowhere. No quorum constant exists: `REVIEWERS_MAX = 8` is a capacity bound on the votes slice, not a quorum of expected contributors (and it is not even re-exported — `mod review` is private).
- **A1:** all subagents fail — zero votes → `Err(CouncilError::TooManyItems { field: "reviewers", max: 8 })`. The empty case fails closed (no empty success), but with a misnamed error and no typed `insufficient_contributions` for downstream consumers to match on.
- **A2:** a late vote (r4 = Reject) arrives after aggregation: rebuilding the review with it moves the published verdict Keep → Revise with no late marker, no generation token, no audit trail. "Recorded as late, never merged" is unrepresentable — `decision()` is a pure function of the votes slice.

## Full technical depth

The seam is `crates/phlow-council/src/review.rs`: `CouncilReview { candidate, votes }`, `CouncilReview::new(candidate, votes)` validating only the votes slice it is handed (empty → TooManyItems; >8 → TooManyItems), `decision()` tallying available votes. There is no expected-contributor count, no missing-contribution cause records, no quorum threshold (named or otherwise), no late-result state, no published-verdict immutability. The all-fail error is worth noting precisely: it fails closed, which is the safe direction — the finding is the *name*, not the refusal. A downstream consumer matching on `insufficient_contributions` to trigger re-delegation cannot exist because the variant does not exist. The late-arrival case is the sharpest: any caller that rebuilds the review with newly arrived votes silently mutates history; the struct gives callers no way to do better.

Banked for Matt (product decision, NOT auto-implemented on gauntlet authority): whether phlow-council should gain an aggregation envelope — an expected-reviewer set, named quorum thresholds, a typed `insufficient_contributions` verdict, and a late-result policy (record-as-late, never merge into a published verdict). That is a new product feature, not a bug fix; the gauntlet documents the gap and stops.

## Sources

- `~/workspace/repos/phlow/crates/phlow-council/src/review.rs` — `CouncilReview`, `decision()`, `REVIEWERS_MAX`
- `~/workspace/repos/phlow/crates/phlow-council/src/error.rs` — `CouncilError` (no `insufficient_contributions` variant)
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-74 design (Wave 71–75)
