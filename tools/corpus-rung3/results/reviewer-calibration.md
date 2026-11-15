# PH-5 — Reviewer calibration (proposal only; the operator applies)

Each reviewer's recorded verdict (`approved` / `not_approved`) is scored against the corpus manifest's independently verified ground truth. The runtime records no numeric confidence, and none is invented: a binary verdict read as a certain prediction has a Brier score equal to its error rate, which is what the table reports. Agreement with the host gate (`tool_checked`) is shown as the checks-only comparison; the gap between the two columns is where escapes live — runs the checks and the reviewer both passed that were actually wrong.

Floor: agreement >= 0.80 and false-approval rate <= 0.25, over at least 5 scored verdicts. Below that count the verdict is *insufficient evidence* — neither trust nor demotion. Runs whose label names no reviewer model: 0 (excluded; the outcome row does not record roles).

## Reliability per reviewer model

| Model | Scored | Approved | Agreement | Error (=Brier) | False-appr. rate | False-rej. rate | Host-gate agreement | Verdict |
|---|---|---|---|---|---|---|---|---|
| hf-devstral-24b-tools-64k | 8 | 6 | 0.88 | 0.12 | 0.00 | 0.14 | 0.62 | calibrated — no action |
| hf-gpt-oss-20b-tools-64k | 8 | 6 | 0.75 | 0.25 | n/a (no such runs) | 0.25 | 0.88 | **demote to advisory (proposed)** |
| hf-granite-4.2-8b-tools-64k | 8 | 7 | 0.88 | 0.12 | n/a (no such runs) | 0.12 | 0.62 | calibrated — no action |

### hf-devstral-24b-tools-64k

Scored verdicts: 8 (6 approved, 2 not approved); unscored verdicts: 0. True approvals 6, false approvals 0, false rejections 1, true rejections 1.

### hf-gpt-oss-20b-tools-64k

Scored verdicts: 8 (6 approved, 2 not approved); unscored verdicts: 0. True approvals 6, false approvals 0, false rejections 2, true rejections 0.

**Demotion proposed (shadow-before-trust).** This reviewer's verdict should stop gating cycle approval and become advisory: the verdict is still recorded on every run (so calibration keeps measuring it), but cycle approval rests on the coder's status and the host verification gate alone until the model's agreement recovers above the floor on a fresh corpus. The operator applies this by hand in the workspace config — the interim form available today is to seat a calibrated model as `reviewer` in `[models]` and keep this model only on non-gating work; the durable form is a `reviewer_mode = "advisory"` setting honoured by the runtime's cycle-approval rule, which is proposed here and not implemented by this report.

### hf-granite-4.2-8b-tools-64k

Scored verdicts: 8 (7 approved, 1 not approved); unscored verdicts: 0. True approvals 7, false approvals 0, false rejections 1, true rejections 0.

