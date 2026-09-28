# evals/public — visible development cases

These task manifests are **candidate-visible**. Candidates may iterate
against them freely during development; scores here measure progress, not
promotion-worthiness.

Contract:

- Every file follows the test-case contract parsed by
  `phlow_experiment::manifest::parse_task_manifest` (`schema_version`,
  `id`, `language`, `kind`, `risk`, `workspace_fixture`, `task`,
  `[budget]`, `[[checks]]`, `[acceptance]`).
- `[[checks]]` argv entries are the exact commands the host runs — the
  operator approves them, the candidate never edits them.
- A public pass is necessary but never sufficient for promotion: hidden
  holdouts, safety cases, and independent review still apply.
