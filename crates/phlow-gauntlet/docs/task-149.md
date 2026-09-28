# task-149: secret non-disclosure

**Kind:** rust (adversarial) · **Status:** pass, with one documented scaffold gap · **Wave:** 146–150 · **Commit:** pending (wave 26)

## ELI5

A bug report often contains the keys to the kingdom — the login token that proves the hack, the password used in the demo. Task 149 checks that those secrets never leak into anything a human (or a log file, or a report) can read. Every place text gets written down — logs, reports, diffs, error messages — is scrubbed *at the moment of writing*, while the real secret stays locked in a memory-only vault that only the final submit call may open. All test secrets are clearly fake.

## What this task attempts

- **Goal:** verify write-time redaction on every text surface: tool output (with a token and password, including a diff block) → audit log; finding report rendering; exfiltration via the finding title; exfiltration via an error message. Zero secret substrings on any surface (asserted by scanning); the vault retains the real value for the authorized read path and is never serialized.
- **Mechanism:** `src/tasks/task_149.rs` drives the real `SecretVault`, `redact_text`, and `AuditLog` with fixture secrets (`sk-live-F4K3t0k3n9x2q7m`, `hunt3r2-f4k3-pw-2026` — FAKE, gauntlet-only): `tool_output_redacted_in_logs` (V1); `report_redacted_vault_retains_real_value` (V2: report clean, `vault.get("platform_token")` still returns the real value, and an `include_str!` check fails the test if `secret.rs` ever gains a `Serialize` impl); `title_exfiltration_redacted` (A1); `error_message_redacted_at_boundary` (A2). A `scan_clean` helper asserts no secret substring on each surface; failure messages name the surface, never the secret.
- **Success criterion:** zero secret substrings in logs, reports, diffs, and error output; vault never serialized.
- **Non-goals:** proving non-disclosure in general (pattern redaction is a mechanism, not a guarantee — the design doc's honest limitation); real secret detection (gitleaks-style patterns are the model, not the implementation here).

## What happened

PASS on the exact tree, all four scenarios:

- **V1:** every audit-log line containing the token or password (including the `+Authorization: Bearer …` diff line) was stored as `[REDACTED]`; `leaks_none()` true; a scan of all entries found zero secret substrings.
- **V2:** the rendered report contained the secrets nowhere; the vault's authorized `get("platform_token")` still returned the real value (the submit call's key still works); the `include_str!` source check confirmed no `Serialize` in `secret.rs`.
- **A1:** the hostile title (`xss in login token=…`) was redacted at write time — both the stored title and the audit-logged copy were clean.
- **A2:** the validator error echoing the token was redacted at the log boundary; the raw error existed only in memory, never in the log.

**Documented scaffold gap (reported, not fixed — four waves share the scaffold):** `SecretVault` derives `Debug`, so `format!("{:?}", vault)` prints the raw secret values. The driver asserts this as currently-observed behavior and records it in evidence. Never `Debug`-log the vault. (A second note: the struct docs say "no method that returns all secrets at once", but `secret_values()` does exactly that — it is the legitimate source of the redaction list for `AuditLog::new`, so the invariant to hold is that its output never reaches a text surface unredacted, which the V1/A2 scans verify.)

## Full technical depth

Write-time redaction is the correct layer because the raw secret *must* exist in memory — the tool output genuinely contains the token, the PoC genuinely uses the password, and the submit call genuinely needs the platform token to authenticate. Trying to keep secrets out of memory is hopeless; the enforceable boundary is the moment text becomes durable or visible. `AuditLog::append` redacts against the configured secret list on every write, so every entry is born clean, and `leaks_none()` gives the suite a single audit predicate over the whole log.

The vault's contract is minimal on purpose: no `Serialize` impl (memory-only — serialization is how secrets escape into files, crash dumps, and debug endpoints), and the only read path is per-key `get` for the authenticated submit call. The `include_str!` check in the test turns "never serialized" from a code-review claim into a gate: adding a `Serialize` derive to `secret.rs` fails the suite.

The adversarial cases target the two classic exfiltration shapes: the *title* (attacker-controlled metadata that gets displayed in UIs, indexes, and notifications) and the *error message* (the path where a well-meaning diagnostic echoes its inputs). Both are redacted because redaction happens on the text, not on the field — `redact_text` does not care which struct member the secret arrived in.

Honest limitation, stated plainly: pattern-based redaction (the gitleaks model) catches the secrets on the configured list; it is not a proof that no secret can ever appear on a surface. What the task verifies is the *mechanism* — every write goes through the redaction boundary — not an information-flow guarantee.

## Sources

- `crates/phlow-gauntlet/src/bounty/secret.rs` — `SecretVault`, `redact_text` (the scaffold under test; read-only for this wave)
- `crates/phlow-gauntlet/src/bounty/store.rs` — `AuditLog::append` (write-time redaction), `leaks_none()`
- gitleaks (https://github.com/gitleaks/gitleaks) — pattern/regex-based secret detection; the model for the redaction-list approach (per the design doc)
- `~/workspace/gauntlet-design-tasks-131-150.md` — task-149 design (Wave 26)
