# Cues — servers hand work to each other (PH-K2)

Status: primitives + reference behaviour landed 2026-11-10 in
`phlow-checks::cue`. **Live wiring landed 2026-11-15 (PH-K2, rung
6):** the runtime drains its own cue directory at the head of
every `Runtime::run` — see "Live wiring".

## Shape (INDEX §2.2, D-1269)

A cue is a small JSON file in the **consumer server's own** cue
directory (`~/.local/share/<server>/cue/`; phlow's is
`~/.local/share/phlow/cue/`, resolved by `phlow_cue_dir()`). A
producer writes into the consumer's directory and returns. Fields:
`{id, kind, from, to, created_unix_ms, expires_unix_ms,
payload_hash, origin_lane, content_label}`. The payload is stored
content-addressed under `payloads/<sha256>` and referenced by hash,
never embedded.

Directory layout: `pending/`, `claimed/`, `expired/`, `payloads/`.

## Rules as implemented

- **Expiry.** `expires = created + ttl`. Expired unconsumed cues are
  listed (`list_expired`) and moved to `expired/` by
  `reap_expired_cues` — never retried forever, never waited on.
- **Claim.** Atomic rename `pending/ → claimed/`; a second claim
  fails with `AlreadyClaimed` naming the id. The payload is
  hash-verified at claim time; a tampered payload is refused.
- **Answer.** A consumer answers with a `run-done` cue, not a reply
  message. `consume_run_this` is the reference behaviour: claim the
  oldest unexpired `run-this` addressed to this server, hand the
  payload to the server's own run logic, write `run-done` with
  `{run_this_id, verdict, evidence}` addressed to the sender,
  carrying the original cue's lane labels.
- **Lanes.** Labels are born with the cue; empty `content_label`
  defaults to `nsfw`, empty `origin_lane` to `unknown` (fail-closed,
  as everywhere in this programme). An SFW-scoped consumer
  (`LaneScope::SfwOnly`) cannot list or claim a non-`sfw` cue.
- **Malformed.** A cue file that does not parse, violates a bound,
  or whose id does not match its file name fails the read/listing
  with the file's name — refused by name, never skipped silently.
- **No waiter loops.** Nothing in the module sleeps or polls; a
  consumer reads its directory when it chooses (the run contract's
  rule, INDEX §2.1).

## Live wiring (PH-K2, landed 2026-11-15)

The runtime is the fleet's reference consumer. `Runtime::run`
drains phlow's own cue directory first (`phlow-runtime::cues`,
called before the run's own work): expired cues are reaped and
listed on the report as `cues_expired`; each live `run-this`
addressed to `phlow` is consumed and answered. The handler parses
`{checks: [names]}` and runs each named check through the same
pinned `CheckRunner` the runtime verifies with — **operator-
approved names only**: a cue naming a check outside the config is
refused whole (verdict `refused`, the name in the evidence,
nothing runs). Verdicts are `ok` / `failed` / `refused`, with
per-check evidence bounded to 2,000 characters. Consumed cues
land on the report as `cues` (cue id, sender, verdict, answer
id); a drain failure — including a malformed cue file, refused
by name — lands as `cues_error` and never fails the run. Bounds:
at most 8 cues per run, at most 32 checks per cue, answers live
for one hour. The directory is the embedder's `set_cue_dir`, or
`phlow_cue_dir()` by default; the lane scope is the embedder's
`set_cue_scope`, defaulting to unrestricted (answers carry the
original cue's labels, so lane visibility is preserved end to
end). There is still no polling and no waiter loop: the drain
rides the run cadence, exactly as the run contract prescribes.

## Remainder (noted, not built)

- Cross-server chains (fathom → lumen → torogao) adopt these
  primitives per-server; each server's K2 copies the
  `consume_run_this` call shape with its own handler.
