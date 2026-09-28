# evals/regressions — every accepted real failure

When a real failure is confirmed during the experiment, a regression case
lands here so it can never silently reappear.

Contract:

- Add the case **after** removing secrets and environment-specific details.
- Each case pins the exact check argv that caught the failure, the
  baseline revision where it was found, and the revision where the fix
  landed.
- `allow_new_failures_on_prior_successes = false`: a newly failing
  previously-solved critical task blocks promotion, and this directory is
  the list of tasks that must keep passing.
- This directory ships empty: it grows only from confirmed, remediated
  failures — never from speculation.
