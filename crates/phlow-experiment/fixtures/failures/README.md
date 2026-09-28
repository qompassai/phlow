# fixtures/failures — compiler, test, lint, timeout, and crash scenarios

Failure fixtures for the evaluator and orchestration suites: the harness
uses them to prove that failure is reported as failure — never as success,
never as a skip, and never as "verified".

Contract:

- Each fixture pins the failing input, the exact check argv, the expected
  failure signature (exit code, signal, timeout, or output pattern), and
  the required cleanup (no orphaned processes, sockets, or temp files).
- A fixture that hangs must hang only under the harness's deadline, which
  kills the whole process group.
- This directory ships empty: failure scenarios are added alongside the
  checks they exercise, each reviewed before landing.
