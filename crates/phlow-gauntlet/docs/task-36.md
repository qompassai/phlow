# task-36: tool-output exfiltration

**Kind:** nvim-lua · **Status:** fail (seam absent — no secret-pattern redaction on tool-output persistence; diver-owned finding, flagged not fixed) · **Wave:** 36–40 · **Commits:** pending (wave 36-40)

## ELI5

Imagine a spy agency's phone log. Every call's contents get written
into the logbook — verbatim. The agency has a rule: "if a file is
named 'password.txt', black it out." But the rule only looks at file
*n*ames, not file *contents*. So an agent writes down a live API key
on a page titled "shopping list" — and the logbook keeps the key in
cleartext, photocopied into the archives and the monitoring reports.

That's the harness: tool outputs (which can contain secrets — a leaked
API key, a token pasted by a tool) are persisted verbatim to the
transcript (`ai.harness.events` sink) and to the logs
(`ai.harness.telemetry.log`). The redactor only scrubs secrets hiding
under secret-sounding *key names* — a bare `sk-live-...` token under
a benign field name like `output` flows through in cleartext. There
is no scanner that reads the *contents* of string values and redacts
secret-shaped text, no streaming redactor that would reassemble a
secret split across chunk boundaries, and `event.redacted` is a flag
the caller sets — not a scan result.

## What this task attempts

- **Goal:** drive the real harness sink/transcript with tool outputs
  that secretly contain credentials — default: clean output
  round-trips; then a tool output containing a credential (as part of
  normal output, not labeled a secret); then the tainted output is
  appended to the real transcript/sink and persisted; then the logs —
  assert the credential is redacted from the transcript and the logs.
- **Mechanism:** the `task_36.lua` driver in headless Neovim against
  the REAL diver Lua tree — real `sink:append` (events) and real
  `telemetry.log`, with a probe `sk-live-...` secret. No mocks of the
  sink or the redactor. The driver never echoes the raw secret into
  its evidence (boolean findings only), so the verdict/report JSON
  stays secret-free by construction.
- **Success criterion:** the credential is redacted from the
  transcript and the logs.
- **Non-goals:** fixing diver. Diver-owned findings stay flagged,
  never fixed on gauntlet authority.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly. The
credential reaches every persisted copy verbatim:

- `clean_output_roundtrips` (V): clean tool output round-trips
  verbatim through the sink. The sink is a faithful persistence layer
  — which is exactly the problem for secret content.
- `probe_exercises_the_real_sink_path` (V): the tainted output was
  appended to the real sink and the real `telemetry.log` before
  concluding — the finding rests on the real path, not a mock.
- `secret_reaches_sink_verbatim` (A): the secret is present in the
  sink (transcript) JSON — `sink:append` stores the payload table
  verbatim, and `event.redacted` is caller-controlled, not
  scan-derived. Nobody flagged the secret, so nothing redacted it.
- `secret_reaches_telemetry_log` (A): the secret is present in the
  `telemetry.log` entries JSON — `telemetry.redact` redacts based on
  secret-bearing *key names*, not secret *patterns* in string values.
  Under a benign field name, the bare token survives.
- `secret_survives_chunk_splitting` (A): the secret split across two
  `stream_delta` chunks reassembles to the full secret in the event
  stream — no streaming redactor joins chunk boundaries.

One adjacent mechanism exists and is cited in the evidence, not
relied on: `ai/dataaccess/secrets.redact` scrubs secrets in URI
userinfo and assignment forms (`password=`, `"token":`), but it
handles neither bare `sk-live-...` tokens nor the harness sink path —
the harness sink and `telemetry.log` never call it.

## The fix — what changed and why

No product fix was made — diver-owned, flagged not fixed. The
gauntlet-side work was an honest probe:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_36.lua` (new) —
  drives the real sink and `telemetry.log` with a probe secret,
  including the chunk-split streaming scenario; fail-closed
  (`where = "recon"` if the seam ever changes shape).
- **Changed:** `crates/phlow-gauntlet/src/tasks/task_36.rs` (new) —
  thin `nvim-lua` shim, mirroring `task_30.rs`.
- **Why:** a redaction claim needs a redactor. The probe proves the
  persisted copies contain the secret verbatim — so the honest verdict
  is seam-absent, not a faked pass on the adjacent `secrets.redact`.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/events.lua`
  (`sink:append`, caller-controlled `redacted` flag),
  `~/workspace/repos/diver/lua/ai/harness/telemetry.lua`
  (`log`, `redact` by key name only),
  `~/workspace/repos/diver/lua/ai/dataaccess/secrets.lua`
  (adjacent redactor, not wired into the sink).
- **Validation agents:** the 2 validation tests
  (`probe_reports_seam_absence`, `probe_exercises_the_real_sink_path`)
  pin the `fail`-at-`seam` verdict and prove the real sink path was
  exercised before concluding.
- **Adversarial agents:** the 2 adversarial tests
  (`verdict_is_a_finding_not_a_probe_crash`,
  `secret_reaches_persisted_copies_verbatim`) rule out a crashing
  probe masquerading as the finding and pin the verbatim-presence /
  chunk-reassembly evidence (and that the driver itself never echoes
  the raw secret).

## Full technical depth

`events.lua`'s sink is an append-only JSON store: `sink:append(event)`
serializes the event table as given. The event schema carries a
`redacted` field, but it is set by the caller — there is no
scan-derived path; nothing in the harness inspects string values for
secret shapes. `telemetry.lua`'s `redact` walks the entry table and
masks values under keys that *look* secret-bearing (`token`,
`password`, `secret`, …) — a key-name denylist, not a
value-pattern scanner. A bare token under `output` is not on the
list, so it passes through in cleartext into the log entries JSON.

Streaming makes it worse structurally: the event stream emits
`stream_delta` chunks, and there is no join step that would let a
redactor see the reassembled text — the probe splits the probe secret
across two chunks and reassembles the full secret from the stream
events, demonstrating the gap without inventing anything.

The adjacent `ai/dataaccess/secrets.redact` is a real redactor but
for a different shape (URI userinfo, `password=`/`"token":`
assignment forms) and, critically, the harness sink and
`telemetry.log` never call it — so even its coverage does not reach
the transcript path.

What redaction would need (banked for Matt, not implemented here): a
value-pattern scanner on the sink/transcript path (secret-shaped
strings like `sk-live-...` redacted regardless of key name), a
streaming redactor that joins chunk boundaries before persisting, and
the adjacent `secrets.redact` either extended or explicitly documented
as out-of-scope for the harness path. Until then, any secret a tool
prints under a benign field name lands in the transcript and the logs
in cleartext — and the driver never echoes it, so the finding itself
does not become a second leak.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/events.lua`
  (sink, caller-controlled `redacted` flag).
- Primary: `~/workspace/repos/diver/lua/ai/harness/telemetry.lua`
  (`log`, key-name-only `redact`).
- Primary: `~/workspace/repos/diver/lua/ai/dataaccess/secrets.lua`
  (adjacent redactor; not wired into the harness sink).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_36.lua` (real sink,
  real telemetry.log, headless Neovim).
- Shim: `crates/phlow-gauntlet/src/tasks/task_36.rs`.
- Tests: `crates/phlow-gauntlet/tests/task_36.rs` (2V/2A).
