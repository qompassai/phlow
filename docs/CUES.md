# Cues — servers hand work to each other (PH-K2)

Status: primitives + reference behaviour landed 2026-11-10 in
`phlow-checks::cue`. Runtime/MCP wiring (a phlow surface that polls
its cue directory between runs) is **not** landed — see "Remainder".

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

## Remainder (noted, not built)

- Wiring `consume_run_this` into a live phlow surface: the natural
  home is the MCP server's idle path or a `flow_cues` operator
  command, with the handler parsing `{checks: [names]}` and running
  `CheckRunner::run_all`. That is a surface change (Tiger Style MCP
  rules: schema + annotations + contract gate) and belongs in its
  own package, not folded into the primitives.
- Cross-server chains (fathom → lumen → torogao) adopt these
  primitives per-server; each server's K2 copies the
  `consume_run_this` call shape with its own handler.
