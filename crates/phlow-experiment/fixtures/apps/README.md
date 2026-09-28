# fixtures/apps — tiny source projects by language and task

Deterministic, offline app fixtures for the benchmark corpus. Each fixture
is a minimal but realistic starting point (greenfield, seeded defect, or
repair target), pinned to the language tier in `manifests/languages.toml`.

Contract:

- Fixtures are **inert**: no network access, no credential material, no
  destructive operations. A fixture that needs secrets or the network does
  not belong here.
- Fixtures are **deterministic**: no wall-clock dependence, no randomness
  without a recorded seed.
- Shipped fixture: `rust-cli-hello/` — a trivial inert Rust CLI that prints
  a greeting. It exists so the manifest parser, fixture loader, and the
  example task manifest have something real to point at.
