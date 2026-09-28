# evals/holdout — protected promotion cases

These task manifests are **NOT candidate-writable and NOT candidate-visible**
until promotion evaluation. They exist to detect evaluator overfitting:
gains must persist on hidden holdouts, prior-success regressions,
order-shuffled runs, and independent review.

Contract:

- Holdout manifests live outside every model-writable root. Any write
  attempt by a candidate is a stop condition for the experiment.
- Hidden variants change names, layouts, data, and failure locations while
  preserving the underlying contract, so memorizing public cases does not
  transfer.
- Special-casing fixture names, detecting evaluator presence, or optimizing
  only visible cases blocks promotion regardless of aggregate score.
- This directory ships empty: holdout cases are added by the operator as the
  benchmark corpus grows (Phase 1 of the trial protocol).
