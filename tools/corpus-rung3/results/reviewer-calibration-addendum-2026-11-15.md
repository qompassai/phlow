# PH-5 — Reviewer calibration addendum (2026-11-15): final-verdict semantics

The rung-3 calibration (`reviewer-calibration.md`) scored the outcome rows' `reviewer_verdict` field as the pre-rung-5 runtime recorded it: the **first** reviewer entry of a multi-cycle run. Rung 5 corrected `RunOutcome::from_report` to record the **final** verdict — the last reviewer entry, the verdict the run actually ended under — because a run approved in cycle 1 and rejected in cycle 2 was recorded `approved` while ending `failed`. This addendum re-derives the table over the same corpus, the same labels, and the same ground truth, recomputing each run's verdict from its run report with the production extraction. **The rung-3 document is unchanged; where the two disagree, this addendum governs.** Demotion remains a proposal the operator applies by hand.

Corpus runs scored: 24. Verdicts recomputed from reports: 24; runs with no report file (recorded verdict kept, disclosed): 0; report files skipped as unreadable or id-less: 0.

## Verdict changes

| Run | Reviewer model | Truth | Recorded (first entry) | Final (last entry) |
|---|---|---|---|---|
| run-2041177-1794737084894912173-0 | hf-granite-4.2-8b-tools-64k | success | not_approved | approved |
| run-2113719-1794738421425422794-0 | hf-devstral-24b-tools-64k | success | approved | not_approved |
| run-2150706-1794739919326098298-0 | hf-gpt-oss-20b-tools-64k | success | not_approved | approved |
| run-2243017-1794744944765682143-0 | hf-devstral-24b-tools-64k | success | approved | not_approved |

## Reliability: recorded vs final

| Model | Scored | Agreement (recorded) | Agreement (final) | False rejections (recorded → final) | Verdict (recorded) | Verdict (final) |
|---|---|---|---|---|---|---|
| hf-devstral-24b-tools-64k | 8 | 0.88 | 0.62 | 1 → 3 | calibrated | demote to advisory (proposed) |
| hf-gpt-oss-20b-tools-64k | 8 | 0.75 | 0.88 | 2 → 1 | demote to advisory (proposed) | calibrated |
| hf-granite-4.2-8b-tools-64k | 8 | 0.88 | 1.00 | 1 → 0 | calibrated | calibrated |

## Status changes

- **hf-devstral-24b-tools-64k**: calibrated → demote to advisory (proposed) (agreement 0.88 → 0.62).
- **hf-gpt-oss-20b-tools-64k**: demote to advisory (proposed) → calibrated (agreement 0.75 → 0.88).

