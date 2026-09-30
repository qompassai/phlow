---
name: "tiger-style-mojo"
description: "Write or review Mojo code in Matt's Tiger Style: safety-first Mojo standard with explicit contracts, assertions, bounded work, and disciplined ownership. Use when writing, regenerating, refactoring, or reviewing Mojo — especially numerical kernels, GPU/accelerator code, and Python-interop orchestration. See references/TIGER_STYLE_MOJO.md for the full guide."
---

# Tiger Style Mojo

## Purpose

Apply Matt's Tiger Style standard to Mojo code. Priority order, always:
**Safety > Performance > Developer Experience.** The full guide lives in
[references/TIGER_STYLE_MOJO.md](references/TIGER_STYLE_MOJO.md); this file
is the operational core. When the two disagree, the full guide wins.

## Workflow

1. Pin a reviewed compiler version and its matching standard library (plus
   MAX and accelerator libraries, pinned consistently, when used). Keep the
   package/environment manifest and lockfile in the project. Never use a
   moving nightly channel as a reproducibility claim.
2. Record build/release evidence: compiler version, source and dependency
   revisions, CPU target, OS, native libraries, optimization and assertion
   settings, GPU model/driver/device when applicable, Python environment
   when Python interop is enabled.
3. Run `mojo --version`, `mojo build`, and `mojo format` from the same
   environment Neovim uses. A global executable and a project executable
   can be different language releases — resolve that before weakening
   diagnostics or changing code.
4. Lay the module out top-to-bottom: purpose, public contract (types,
   entry points, ownership), then private machinery.
5. Validate external input with ordinary control flow returning a typed
   error; assert facts a correct implementation already established.
6. Review against the checklist below before calling the code done.

## Operating Rules

- **Contracts first.** Every substantial operation answers: accepted and
  rejected inputs, trust boundary, max work/memory/output/time, owner of
  every buffer/allocation/device/task, failure and cancellation behavior,
  cleanup obligations. A successful compilation proves none of this; nor
  does an impressive benchmark.
- **Ownership discipline.** Mojo's ownership and borrowing are tools for
  the contract, not decoration. Borrow for inspection; transfer ownership
  when retaining or handing off is the contract. Name the owner when work
  escapes a scope (task, callback, device buffer, Python object).
- **Bound everything.** No unbounded loops, retries, queues, or growth.
  Name limits with units (`input_bytes_max`, `batch_size_max`,
  `retry_count`, `deadline`). Long-lived services may run indefinitely,
  but each batch is bounded and cancellation has a documented effect.
- **Numerics are a safety property.** State precision, rounding, overflow,
  and NaN/inf policy where they affect correctness. Keep tight numerical
  kernels and interactive orchestration behind explicit boundaries —
  they have different cost profiles.
- **Python interop is a trust boundary.** Declare the Python
  implementation and package environment, keep the interop surface
  narrow and typed, and never let a Python import silently authorize
  device access, package installation, or network use.
- **Exceptions need a reason, a local owner, and a check.** Describe what
  assumption would invalidate the exception. No global suppressions that
  make later violations invisible.
- **Toolchain upgrades are reviewed changes.** Re-run formatting,
  compilation, tests, and representative benchmarks; keep compiler
  updates separate from unrelated system maintenance so failures can be
  attributed and rolled back. Build and test as an ordinary user; review
  package channels, installers, AUR build files, and foreign dependencies
  before executing them. Arch is rolling — never infer vendor support
  from local installability.

## Output Contract

Mojo you produce for Matt must: compile on the pinned toolchain with
diagnostics at full strength, carry doc comments on public items stating
contract and bounds, return typed errors on expected failures, assert
internal invariants, bound all work/queues/retries/memory, keep kernels
and orchestration behind explicit boundaries, record build evidence, and
pass the review checklist.

## Review Checklist

- [ ] External input validated before allocation, indexing, and mutation.
- [ ] Every buffer, device allocation, task, and Python object has one owner and a cleanup path.
- [ ] Work, batches, queues, memory, retries, and time have explicit budgets.
- [ ] Numeric precision, overflow, and NaN/inf policy stated where they matter.
- [ ] Errors preserve failure and cause; assertions enforce established invariants.
- [ ] Python interop surface is narrow, typed, and its environment recorded.
- [ ] Cancellation cannot publish stale results or duplicate effects silently.
- [ ] Compiler/stdlib/MAX/accelerator versions pinned; build evidence recorded.
- [ ] No global diagnostic suppressions; exceptions are local, owned, checked.
- [ ] Logs, argv, and build environments expose no secrets.
- [ ] Performance claims include measurements and their environment.
