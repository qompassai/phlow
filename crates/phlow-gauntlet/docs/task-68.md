# task-68: metric cardinality bound

**Kind:** rust (adversarial) · **Status:** fail (open) · **Wave:** 66–70 · **Commits:** pending (wave 66-70)

## ELI5

A metrics registry tracks things like "tool calls per run" — but if the *labels* on a metric can include a run ID or, worse, raw text from untrusted tool output, an attacker can make the registry grow forever (a million labels = a million time series = memory exhaustion). The design asks for a hard cap on label combinations: a per-run label must be rejected or hashed with an explicit `CardinalityExceeded` signal. Phlow has no such registry at all: no metrics dependency in any crate, no labeled metric-series store, and the only counting mechanism (`report_mut` in phlow-runtime) uses fixed literal keys like `"runs"` and `"errors"` — there are no label sets to bound. Both adversarial weapons misfire: the per-run-id label has no label-registration API to attack, and untrusted tool output has no label path to travel. The driver's verdict is the honest FAIL at the absent seam: the cardinality bound the design demands is guarding a registry that doesn't exist.

## What this task attempts

- **Goal:** verify a hard bound on metric cardinality under adversarial pressure (per-run labels, untrusted-output label values).
- **Mechanism:** `src/tasks/task_68.rs` is an audit-only driver (no seam to drive). It scans every phlow crate's `src` tree (live working tree, exact-token case-insensitive, phlow-gauntlet excluded) for registry/label/series vocabulary; checks all crate manifests for metrics dependencies; classifies every `report_mut::{counter,add,set,push}` call site as static or dynamic key.
- **Success criterion:** a labeled registry with a hard series cap and explicit rejection, or a sourced honest FAIL.
- **Non-goals:** adding a metrics library on gauntlet authority.

## What happened

Honest FAIL at `where = "seam"`, first attempt:

- **V1:** the registry-vocabulary scan found 0 hits across all phlow crates, and 0 crates depend on a metrics library. No labeled registry exists anywhere.
- **V2:** every `report_mut` call site is static-literal-keyed (0 dynamic keys) — a JSON report object, not a labeled series store. Classified adjacent, not the seam.
- **A1:** the per-run-id label weapon has no target: the label-API vocabulary scan finds no registration surface, so there is nothing to reject or hash, and no `CardinalityExceeded` signal to emit.
- **A2:** untrusted tool output cannot become a label — there is no label path, only static report keys. The attack the design fears is structurally impossible in a registry that doesn't exist.

## Full technical depth

`phlow-runtime::runtime::report_mut` is a private JSON report helper: `counter(name)`, `add(name, delta)`, `set(name, value)`, `push(name, item)` all take literal `&str` keys written at the call site. The call-site scan verified every key is a static literal — no `format!`-built keys, no run-id interpolation, no untrusted-output-derived keys. A vocabulary scan for registry/label/series/cardinality/histogram/gauge/counter-family tokens across all crates returned 0 hits; `grep` over all `Cargo.toml` files for prometheus/opentelemetry/metrics/statsd returned 0. The design's "at most N series" cap and "explicit rejection signal" describe an API surface phlow has never built.

Banked for Matt (product decision, not a bug): whether phlow should gain a labeled metrics/telemetry registry at all; if it does, the hard series cap, the explicit rejection signal (`CardinalityExceeded`), and the label-value sanitization policy for untrusted tool output — the design's three asks.

## Sources

- `crates/phlow-runtime/src/runtime.rs` — `report_mut` (private JSON report helper, literal keys)
- `crates/*/Cargo.toml` — no metrics dependencies (workspace-wide)
- `~/workspace/gauntlet-design-tasks-21-70.md` — task-68 design (Wave 12)
