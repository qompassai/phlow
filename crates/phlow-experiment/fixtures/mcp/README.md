# fixtures/mcp — malformed and boundary JSON-RPC frames

Fixtures for the MCP and editor-protocol adversarial cases: malformed JSON,
invalid JSON-RPC ids, oversized newline frames, partial frames, duplicate
`initialize`, calls before initialization, unknown methods, EOF while busy,
protocol-only stdout contamination, and diagnostics accidentally emitted to
stdout.

Contract:

- Frames are inert text; the harness feeds them to the protocol parser and
  asserts typed rejection, never a crash or a silent accept.
- Every frame is paired with the expected outcome (reject with a named
  error, or accept within explicit bounds).
- This directory ships empty: frames are added with the Phase 2 protocol
  hardening work, each reviewed before landing.
