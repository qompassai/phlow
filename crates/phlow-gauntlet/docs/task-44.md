# task-44: egress filtering

**Kind:** nvim-lua · **Status:** fail (seam absent — no URL-fetch tool in the harness, no resolved-destination filtering; diver-owned finding, flagged not fixed) · **Wave:** 41–45 · **Commits:** pending (wave 41-45)

## ELI5

When an AI tool fetches a URL, "allow example.com" is not enough —
attackers play tricks with *where the request really goes*. The
classic tricks: ask for the cloud metadata address
(`169.254.169.254`, which hands out credentials); DNS rebinding
(`evil.com` resolves to a public IP when checked but to
`127.0.0.1` when fetched); and redirect chains (start at a public
page, get bounced to an internal address). The safe design checks
the *resolved destination* — the actual IP after DNS — against the
allowlist on every request and every redirect hop, and blocks are
explicit and logged.

diver has no URL-fetch tool at all: the harness registry holds no
fetch/http/url tool, so the design's "tool requests an internal
address" scenarios have nothing to attach to. The nearest network
code, `ai.rose.http`, is the bounded client for operator-configured
model endpoints (Ollama chat requests) — not an agent tool. Its URL
check compares the hostname *string* (localhost / 127.0.0.1 / ::1
unless `allow_remote`), with no DNS resolution and no resolved-IP
filtering; its curl transport refuses redirects outright
(`--max-redirs 0`) rather than re-checking each hop. The design's
"every request's resolved destination is checked" has no seam.

## What this task attempts

- **Goal:** drive the real tool/adapter network path with the
  design's scenarios — public HTTPS fetch (allowed),
  `http://169.254.169.254/` (blocked), DNS rebinding
  (`evil.com` → 127.0.0.1, blocked on the resolved IP), redirect
  chain public → internal (each hop re-checked) — with a mock
  fetcher carrying controllable DNS/redirects against the real
  filtering logic.
- **Mechanism:** the `task_44.lua` driver in headless Neovim against
  the REAL diver Lua tree — the real adapter registry (tool
  inventory), the real `ai.rose.http` source read from
  `DIVER_LUA_DIR` (URL validation, transport redirect behavior).
  No network calls are made.
- **Success criterion:** every request's *resolved* destination is
  checked against the allowlist; blocks are explicit and logged.
- **Non-goals:** fixing diver. Diver-owned findings stay flagged,
  never fixed on gauntlet authority.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly. There
is no URL-fetch tool, and the adjacent client filters hostnames,
not resolved destinations:

- `no_url_fetch_tool` (V): the harness registry's tool inventory
  contains no fetch/http/url tool — and `register_builtins`
  registers adapters only (acp/a2a/mcp/phlow/rose/herd), never
  tools. The design's "tool requests an internal address" scenario
  has no tool to attach to.
- `hostname_string_check_only` (V/A): `ai.rose.http`'s
  `validate_url` compares the hostname STRING against localhost /
  127.0.0.1 / ::1 (unless `allow_remote`) — no DNS resolution, no
  `getaddrinfo`, no resolved-IP filtering anywhere in the file.
  DNS rebinding (`evil.com` → 127.0.0.1) has no check to defeat
  because no resolved-IP check exists at all.
- `redirects_refused_not_rechecked` (A): the curl transport sets
  `--max-redirs 0` — redirects are refused outright (3xx treated
  as failure), not re-checked per hop. Each hop's resolved
  destination is never validated.
- `adjacent_client_not_agent_tool` (A): `ai.rose.http` is the
  bounded client for operator-configured model endpoints — its
  URL comes from provider config with explicit `allow_remote`
  consent. It is not an agent URL-fetch tool, so even its
  hostname check does not cover the design's threat model.

## The fix — what changed and why

No product fix was made — diver-owned, flagged not fixed. The
gauntlet-side work was an honest probe:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_44.lua` (new) —
  inventories the real registry tools, reads the real
  `ai/rose/http.lua` for egress-filtering evidence (allowlist /
  denylist / resolved / dns / getaddrinfo tokens), and checks the
  redirect behavior of both transports; fail-closed
  (`where = "recon"` if a fetch tool or resolved-IP filtering ever
  appears).
- **Changed:** `crates/phlow-gauntlet/src/tasks/task_44.rs` (new) —
  thin `nvim-lua` shim, mirroring `task_40.rs`.
- **Why:** a filtering claim needs a fetch path. The probe proves
  the harness has no URL-fetch tool and the adjacent client
  filters hostname strings, not resolved destinations — so the
  honest verdict is seam-absent, not a faked pass on "localhost is
  blocked".
- **Source:** `~/workspace/repos/diver/lua/ai/harness/registry.lua`
  (tool inventory; adapters only), `~/workspace/repos/diver/lua/ai/rose/http.lua`
  (`validate_url` hostname-string check; `--max-redirs 0`).
- **Validation agents:** the 2 validation tests pin the
  `fail`-at-`seam` verdict and prove the real registry inventory
  and the real http.lua source were inspected before concluding.
- **Adversarial agents:** the 2 adversarial tests rule out a
  crashing probe masquerading as the finding and pin the
  hostname-only / redirects-refused evidence.

## Full technical depth

The probe starts where the design starts: the tool inventory.
`registry.list_adapters` / the tool registry shows the harness's
tool surface, and `register_builtins` registers exactly six
adapters (acp, a2a, mcp, phlow, rose, herd) — adapters, not tools,
and none of them fetches URLs. MCP stdio is explicitly out of
scope for this task (it is a transport, not URL fetching), so the
harness genuinely has no URL-fetch seam: the design's four
scenarios (public fetch allowed; metadata IP blocked; DNS
rebinding blocked on resolved IP; redirect chain re-checked per
hop) describe requests that no diver tool can make.

The probe then examines the nearest real network code rather
than stopping at "no tool". `ai.rose.http` is the HTTP client
Rose uses to talk to operator-configured model endpoints
(Ollama). Its `validate_url` parses the URL and compares the
hostname string against the loopback set (`localhost`,
`127.0.0.1`, `::1`) unless the provider config sets
`allow_remote` — an explicit operator consent flag. A token scan
of the file for egress-filtering vocabulary
(allowlist/denylist/blocklist/resolved/dns/getaddrinfo) finds no
DNS resolution and no resolved-IP filtering: `evil.com`
resolving to `127.0.0.1` at fetch time would pass the hostname
check, because the check never resolves. On redirects, the curl
transport passes `--max-redirs 0`: any 3xx is a failure, not a
follow — so there is no per-hop re-check because there are no
hops. (The native transport may follow redirects; neither
transport validates a hop's resolved destination.)

The honest summary: diver's network boundary for agent tools does
not exist (no URL-fetch tool), and the operator-endpoint client's
hostname-string check plus redirect refusal is a different,
narrower posture than the design's resolved-destination filtering.
The design's pass criteria — every request's *resolved*
destination checked against the allowlist, per-hop re-checks,
explicit logged blocks — have no seam to attach to.

What egress filtering would need (banked for Matt as a diver
consideration, not implemented here): if diver ever gains a
URL-fetch tool, its fetch path needs resolved-IP allowlisting
(DNS at request time, check the returned IPs, re-resolve and
re-check every redirect hop), with blocks explicit and logged —
the design's four scenarios are the acceptance battery.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/registry.lua`
  (tool inventory; `register_builtins` registers adapters only).
- Primary: `~/workspace/repos/diver/lua/ai/rose/http.lua`
  (`validate_url` hostname-string check; `--max-redirs 0`).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_44.lua` (real
  registry + real http.lua source, headless Neovim).
- Shim: `crates/phlow-gauntlet/src/tasks/task_44.rs`.
- Tests: `crates/phlow-gauntlet/tests/task_44.rs` (2V/2A).
