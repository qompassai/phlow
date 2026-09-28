# task-43: secrets in error messages

**Kind:** rust · **Status:** fail (seam absent — no structural secret types, no masked Display/Debug; the sole redaction is one string-scrub; banked product decision) · **Wave:** 41–45 · **Commits:** pending (wave 41-45)

## ELI5

Error messages are chatty: when something fails, the error often
repeats back what it was doing — including, sometimes, a password
or API key that was in the arguments. The safe design wraps secrets
in a special type (call it a "secret box"): the box prints as
`[redacted]` instead of the value, everywhere — in normal error
text *and* in debug dumps of structs. Then a failing operation
with a secret in any argument position renders zero occurrences of
the secret, by construction, not by hoping nobody typed the
password into a log line.

phlow has no secret boxes. No wrapper type, no masked
`Display`/`Debug`, no field-level debug skipping, no
memory-clearing wrapper exists in any crate. The one redaction
mechanism in the tree — `strip_source_echo` in phlow-config —
scrubs TOML source-echo lines out of *parse error* messages at a
single site; that is string-scrubbing, which the design explicitly
says does not count ("the redaction is structural, not
string-matching"). There is also nothing secret to leak: config
URLs reject credentials, and the operator registry holds only
*public* keys. The design's adversarial scenarios — a tool call
failing with a secret in its arguments, `Debug`-formatting a config
struct — have no target.

## What this task attempts

- **Goal:** find the structural secret types on phlow's error plane
  and run the design's battery: failing operations with secrets in
  every field position render zero secret occurrences; `Debug`
  formatting of config structs masks secrets on the actual output
  string.
- **Mechanism:** the `task_43.rs` driver probes the live working
  tree — a runtime vocabulary scan over every
  `crates/*/src/**/*.rs` for structural-secret tokens (assembled at
  runtime so the probe cannot self-match), plus documentation of
  the sole adjacent mechanism (`strip_source_echo`) and the
  no-secret-fields evidence (URL credential rejection, public-only
  operator keys).
- **Success criterion:** the redaction is structural
  (secret-typed wrappers), not string-matching; a battery of
  failing operations renders zero secret occurrences.
- **Non-goals:** adding secret types. Whether phlow should
  introduce structural secret types with masked `Display`/`Debug`
  is banked for Matt as a product decision, not auto-implemented.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly. There
are no secret boxes, and nothing secret to put in them:

- `no_structural_secret_types_in_sources` (V): the vocabulary scan
  finds zero structural-secret tokens across the workspace — no
  secret-typed wrapper, no masked marker type, no field-level
  debug skipping, no memory-clearing wrapper, no third-party
  secret-type crate.
- `sole_redaction_is_string_scrub` (V): `strip_source_echo`
  (`phlow-config/src/load.rs`) exists — TOML parse errors drop the
  source-echo and caret-annotation lines so config text is not
  echoed into diagnostics. Its call sites are config
  parse-error construction only. It is string-scrubbing at one
  site, not a secret-typed wrapper — the design's structural
  criterion is unmet, and the probe documents it as adjacent
  rather than claiming a pass.
- `failing_tool_call_has_no_masked_args` (A): no tool-arg or error
  type carries a secret-typed field, so the design's "tool call
  fails with a secret in its arguments" cannot be constructed
  against real types. A secret planted in a plain `String`
  argument would render verbatim — no masking layer exists to
  intercept it.
- `config_debug_derives_have_no_skips` (A): phlow-config's model
  structs derive `Debug` with zero field-level skips — and there
  are no secret-bearing fields to skip. Endpoint-URL validation
  rejects credentials ("must not carry credentials"), and the
  operator registry holds only public keys (`OperatorKey {
  ed25519_pk, mldsa_pk }`).

## The fix — what changed and why

No product fix was made — banked for Matt as a product decision
(whether phlow should introduce structural secret types with
masked `Display`/`Debug`; relevant if secret-bearing config fields
or credential-carrying tool args are ever added). The
gauntlet-side work was an honest probe:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_43.rs` (new) —
  bounded live-workspace probe: structural-secret vocabulary
  scan, the string-scrub documentation case, and the two
  adversarial no-target cases; fail-closed (a case fails if the
  vocabulary ever appears).
- **Why:** a redaction claim needs wrapper types. The probe proves
  the vocabulary is absent, the sole adjacent mechanism is a
  string scrub the design excludes, and there are no secret
  fields to mask — so the honest verdict is seam-absent, not a
  faked pass on "parse errors don't echo config text".
- **Source:** the live working tree (`crates/*/src/**/*.rs`),
  `crates/phlow-config/src/load.rs` (`strip_source_echo`,
  credential-rejecting URL validation),
  `crates/phlow-config/src/model.rs` (Debug derives, no
  field-level skips),
  `crates/phlow-experiment/src/registry.rs` (public-only
  `OperatorKey`).
- **Validation agents:** the 2 validation tests pin the
  `fail`-at-`seam` verdict and prove the scan ran over the real
  tree (zero wrapper tokens; the scrubber found and classified as
  non-structural).
- **Adversarial agents:** the 2 adversarial tests pin the
  no-masked-args / no-skips evidence and rule out a probe crash
  masquerading as the finding.

## Full technical depth

The probe's wrapper scan assembles its tokens at runtime from
halves (`Sec`+`ret<`, `Red`+`acted`, `debug`+`(skip)`,
`zer`+`oize`, `Zero`+`izing`, `sec`+`recy`), covering the design's
required machinery: a secret-typed wrapper, a masked marker type,
field-level debug skipping, memory-clearing wrappers, and a
third-party secret-type crate. The walk covers every `.rs` file
under a `src/` directory in `crates/` — including the gauntlet
crate itself — bounded by file size (1 MiB) and file count
(50,000); the probe prose avoids the literal tokens so it cannot
self-match. Result: zero hits.

The adjacent mechanism gets its own case rather than a
hand-wave. `strip_source_echo` keeps the first line of a TOML
parse error (message plus line/column) and drops the echo and
caret-annotation lines that toml-rs renders (`1 | <text>` plus
`^` markers), because storing the message verbatim would echo
operator config text into diagnostics. Its tests pin the
behavior: a planted `sk-fake-SECRET` in the echo lines is dropped
while the location ("line 1") is kept. This is real,
deliberate secret-adjacent hygiene — and it is exactly what the
design excludes: string-matching at one site, not a structural
type that masks by construction. If a secret ever arrives in a
different field position or a different error path, the scrubber
does not cover it; only a wrapper type would.

The "nothing to mask" half is sourced too. `phlow-config`'s URL
validation rejects credentials, query strings, and fragments in
endpoint URLs, and requires non-loopback hosts to opt in — so
config has no secret-bearing URL fields. The operator registry,
the one place that handles key material, maps operator names to
`OperatorKey { ed25519_pk, mldsa_pk }` — raw *public* keys; no
secret material is ever in-process. The design's battery ("a
battery of failing operations with secrets in every field
position renders zero secret occurrences") cannot run: there is
no field position that can carry a secret, and no wrapper that
would mask it if there were.

What structural redaction would need (banked for Matt, not
implemented here): a secret-typed wrapper with masked
`Display`/`Debug` (and ideally memory clearing on drop),
applied to any field that can carry a secret — introduced when
secret-bearing config fields or credential-carrying tool args
first appear. Until then, the error plane's posture is "no
secrets in-process", enforced by input validation rather than by
output masking.

## Sources

- Primary: the live working tree — `crates/*/src/**/*.rs`
  (vocabulary scan, zero structural-secret hits).
- Primary: `crates/phlow-config/src/load.rs`
  (`strip_source_echo`; "must not carry credentials" URL
  validation).
- Primary: `crates/phlow-config/src/model.rs` (Debug derives, no
  field-level skips).
- Primary: `crates/phlow-experiment/src/registry.rs`
  (public-only `OperatorKey`).
- Driver: `crates/phlow-gauntlet/src/tasks/task_43.rs` (bounded
  live-workspace probe).
- Tests: `crates/phlow-gauntlet/tests/task_43.rs` (2V/2A).
