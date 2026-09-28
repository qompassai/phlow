# HumanApproval cryptography: Ed25519 + ML-DSA-65 hybrid

## The ELI5 version

A promotion approval is a note from a human operator that says "yes, ship
this". Before this change, the gate checked only that the note was
*shaped* like an approval — the right fields, in the right order. Anyone
could write a note in that shape. That is like accepting a wax seal
because it is round.

Now the note carries **two wax seals**, made with two different
signet rings:

1. A classical seal: **Ed25519** (the ring everyone has used for years).
2. A post-quantum seal: **ML-DSA-65** (a new ring designed to survive
   future quantum computers).

The gate checks **both** seals against the operator's registered public
keys before the approval counts. Forging one seal is not enough; an
attacker must break both, and they are built on unrelated math, so one
breakthrough does not crack the other.

## The mechanism

### Nested dual signing

The operator signs the canonical approval bytes twice, nested:

```
ed_sig = Ed25519.Sign(ed_sk, canonical)
pq_sig = ML-DSA-65.Sign(pq_sk, canonical || ed_sig)
```

The post-quantum signature covers the classical signature, not just the
message. This *binds* the two halves together: an attacker cannot take a
valid Ed25519 signature from one approval and pair it with an ML-DSA-65
signature from another. Verification is AND semantics — both halves must
verify, or the record is rejected with `BadSignature`.

### Why ML-DSA-65

- ML-DSA is the NIST-standardized post-quantum signature scheme
  (**FIPS 204**, August 2024), selected from the CRYSTALS-Dilithium
  submission.
- **ML-DSA-65** is parameter set 2, targeting **NIST security level 3**
  (roughly the strength of AES-192 against classical attack, and the
  level NIST recommends for general use).
- The Rust crate used here, `ml-dsa 0.1.1`, is pure Rust (`no_std`,
  zero `unsafe`), and its signatures were cross-verified against
  OpenSSL 3.5.8's FIPS 204 implementation in both directions during
  evaluation — they interoperate.

### Why this is not X.509 composite

X.509 composite signatures (draft-ietf-lamps-pq-composite-sigs) solve a
different problem: embedding multiple algorithms inside one ASN.1
certificate structure for PKI. This gate has no certificates, no PKI, and
no ASN.1 — it verifies two independent raw signatures against two
independently pinned public keys in a local TOML registry. The design
borrows the composite draft's *hedging rationale* (an attacker must break
both algorithms) but not its encoding. See also
draft-ietf-pquip-hybrid-signature-spectrums for the spectrum of hybrid
approaches and where nested dual-signing sits on it.

### Key and signature sizes

| Item | Bytes | Hex chars in the record |
|---|---|---|
| Ed25519 public key | 32 | 64 |
| ML-DSA-65 public key | 1952 | 3904 |
| Ed25519 signature | 64 | 128 |
| ML-DSA-65 signature | 3309 | 6618 |
| SHA-256 enrollment fingerprint | 32 | 64 |

### Crate pins (exact)

- `ml-dsa = "=0.1.1"` — pure Rust ML-DSA, FIPS 204.
- `ed25519-dalek = "=3.0.0"` — Ed25519, `forbid(unsafe_code)`,
  `verify_strict` (rejects malleable/small-order signatures).
- `sha2 = "=0.10.9"` — enrollment fingerprints only, never signatures.

> Crate audit note: at the time of writing, the `ml-dsa` crate carries
> no independent third-party audit that the author could verify; the
> classical half (Ed25519) remains as the hedge. The PQC-only sunset
> criterion below requires an audited or certified implementation.

## The v2 record

Exactly eight keys, in this order; anything else fails to parse:

```
v: 2
operator: <name>
approval_id: <unique id>
candidate: <hex digest>
scope: <scope string>
expires_ms: <unix millis>
signature_ed25519: <128 hex chars>
signature_mldsa65: <6618 hex chars>
```

The canonical bytes that are signed are the six non-signature fields in
fixed order, LF-separated, with no trailing newline. v1 (shape-only)
records do not parse at all — there is no legacy verify path, because v1
"signatures" were never cryptographic.

## The operator registry

`$XDG_CONFIG_HOME/phlow/operators.toml` (fallback
`~/.config/phlow/operators.toml`; the `PHLOW_OPERATORS_FILE` environment
variable overrides it in tests):

```toml
[operators."gauntlet-test-operator"]
ed25519_pubkey = "<64 hex>"
mldsa65_pubkey = "<3904 hex>"
fingerprint   = "<64 hex>"   # SHA-256(ed_pubkey || pq_pubkey)
enrolled_ms   = 1750000000000
revoked       = false
```

- Key lengths are enforced exactly; the fingerprint is recomputed and
  compared at load, so a registry that disagrees with itself fails
  closed.
- On Unix the registry file must not be group- or world-readable
  (owner-only permissions); a world-readable registry fails to load.
  Rationale: the registry is not secret (public keys are public), but a
  writable-by-others registry is an enrollment attack, and the permission
  check is the cheap tripwire.
- Unknown operators → `UnknownOperator`; `revoked = true` →
  `RevokedOperator`.

## Replay protection

Consumed approval IDs are persisted as JSONL at
`$XDG_DATA_HOME/phlow/consumed-approvals.jsonl` (fallback
`~/.local/share/phlow/consumed-approvals.jsonl`; `PHLOW_CONSUMED_APPROVALS_FILE`
overrides in tests). Each line is `{"id": "...", "expires_ms": N}`.
Entries expire and are evicted on load; the file is written atomically
(write temp + rename). A reused approval ID fails with
`ApprovalReplayed` *before* any cryptography runs.

## Expiry and TTL

- The approval's `expires_ms` is checked against a trusted `Clock`
  (production: `SystemClock`; tests: `ManualClock`). Expired records
  fail with `ApprovalExpired`.
- Defense in depth: no approval may expire more than
  `APPROVAL_TTL_MAX_MS` (30 days) after the current time, or it fails
  with `ExpiryBeyondMaxTtl` — even a validly signed record cannot grant
  a year-long blank check. Operational policy (shorter TTLs per
  operator) is chosen by the human operator at enrollment time.

## The gate's check order

`PromotionGate::promote` fails closed, in this order (the order decides
which error surfaces first):

1. Empty acting agent → `ApprovalRejected`; acting agent equals the
   approving operator → `SelfApproval`. The acting agent is an explicit
   parameter, never an ambient lookup, so the check cannot be dodged.
2. Already-consumed approval ID → `ApprovalReplayed`.
3. Registry lookup → `UnknownOperator` / `RevokedOperator`.
4. Expiry vs the trusted clock → `ApprovalExpired`; TTL cap →
   `ExpiryBeyondMaxTtl`.
5. Dual signature verification → `BadSignature`.
6. The approval ID is recorded as consumed — only after every check
   passed.
7. The pre-existing gates: complete evidence, protected surfaces,
   unanimous reviewer approval.

## Enrollment, rotation, revocation

- **Enrollment** is a human-driven UX step (not yet built): the operator
  generates an Ed25519 + ML-DSA-65 keypair locally, and the public keys
  plus fingerprint are added to `operators.toml` out of band. The
  fingerprint is read back over a second channel before the entry is
  trusted.
- **Rotation** = enroll the new keys under the same operator name with
  a fresh `enrolled_ms`, then set `revoked = true` on the old entry.
  Revoked keys fail closed; in-flight approvals signed before
  revocation are rejected too (fail closed beats grace periods here).
- **Revocation** = set `revoked = true`. There is no un-revoke path
  except re-enrollment.

## Test identity

Tests use a dedicated identity named `gauntlet-test-operator` with
deterministic test-only seeds. These keys are **never** presented as any
real person's identity and must never appear in a production registry.
The real operator's key enrolls later through the documented UX above —
it is never invented by the tooling.

## PQC-only sunset criterion

There is no date for dropping the Ed25519 half. Both halves stay until
*all* of the following hold:

1. NIST/FIPS guidance evolves to recommend pure-PQC signatures for
   this use class, and
2. the ML-DSA implementation in use has independent audit or
   certification status (not just "pure Rust, no unsafe").

Until then, the hybrid stays: the classical half hedges the young PQC
implementation, and the PQC half hedges the quantum future.

## Non-goals: what this does not solve

- **Key management.** Private keys live with the operator; this design
  says nothing about how they are stored, backed up, or protected.
- **Enrollment ceremony.** The out-of-band fingerprint check is
  procedure, not code; a compromised enrollment channel defeats the
  registry.
- **Clock trust.** `SystemClock` trusts the OS clock; NTP attacks are
  out of scope.
- **Network transport.** Approval records are verified in-process.
  Where they travel over a network channel, transport security (the
  existing SSH layer already negotiates post-quantum key exchange) is a
  separate concern.
- **Side channels.** The implementation uses constant-time primitives
  from the underlying crates but makes no hardened side-channel claims
  for the gate itself.

## Sources

- FIPS 204, *Module-Lattice-Based Digital Signature Standard*
  (ML-DSA), NIST, August 2024.
- RFC 8032, *Edwards-Curve Digital Signature Algorithm (EdDSA)*.
- draft-ietf-lamps-pq-composite-sigs (composite/hybrid signature
  rationale — the hedging argument, not the encoding).
- draft-ietf-pquip-hybrid-signature-spectrums (hybrid design space).
