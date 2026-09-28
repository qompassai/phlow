-- task-44 driver: egress filtering on the tool/adapter network path.
--
-- Recon probe: the design asks for an egress-filter seam on the
-- tool/adapter network path (MCP stdio excluded — this is for tools
-- that fetch URLs). Scenarios: (a) public HTTPS fetch -> allowed;
-- (b) http://169.254.169.254/ (cloud metadata) -> blocked; (c) DNS
-- rebinding — evil.com resolving to 127.0.0.1 — the RESOLVED IP is
-- checked, not just the hostname; (d) redirect chain public ->
-- internal — every hop re-checked.
--
-- This driver inspects the REAL diver tree: the harness registry's
-- tool list (is there any URL-fetch tool?), and the one HTTP module
-- that exists (ai.rose.http — the bounded model-endpoint client).
-- It makes no network calls and spawns no workers.
--
-- Honest result: the designed seam is ABSENT. diver's ai.harness has
-- no URL-fetch tool at all — the registry's tool table contains no
-- fetch/http/url tool, and no adapter offers one. The adjacent
-- module, ai.rose.http, is the bounded client for operator-configured
-- model endpoints (Ollama chat), not an agent URL-fetch path: its
-- validate_url is a hostname-STRING check (localhost / 127.0.0.1 /
-- ::1 unless allow_remote consent), with no DNS resolution, no
-- resolved-IP allowlist/denylist, and no per-hop redirect re-check
-- (the curl transport refuses redirects with --max-redirs 0; the
-- native transport follows them — documented in the module's own
-- comment). It is not the design's tool-egress seam and does not
-- implement the design's pass criteria.
--
-- Fail-closed: if a fetch/http tool appears in the harness registry,
-- or ai.rose.http gains resolved-IP filtering, the probe reports
-- where="recon" instead.
--
-- Prints exactly one JSON verdict line to stdout and always exits 0.
-- Writes nothing outside GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function fail(where, how)
    return { id = 'task-44', outcome = 'fail', where = where, how = how, evidence = evidence }
end

---@return table? mods  -- { registry=..., http=... }
---@return string? err
local function bootstrap()
    local diver_lua_dir = vim.env.DIVER_LUA_DIR
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    if type(diver_lua_dir) ~= 'string' or diver_lua_dir == '' then
        return nil, 'DIVER_LUA_DIR is not set'
    end
    if type(work_dir) ~= 'string' or work_dir == '' then
        return nil, 'GAUNTLET_WORK_DIR is not set'
    end
    vim.opt.runtimepath:append(diver_lua_dir)
    local ok_reg, registry = pcall(require, 'ai.harness.registry')
    if not ok_reg then
        return nil, 'require ai.harness.registry failed: ' .. tostring(registry)
    end
    local ok_http, http = pcall(require, 'ai.rose.http')
    if not ok_http then
        return nil, 'require ai.rose.http failed: ' .. tostring(http)
    end
    return { registry = registry, http = http, diver_lua_dir = diver_lua_dir }
end

---Read a diver source file, bounded. Returns nil on failure.
---@param path string
---@return string?
local function read_file(path)
    local fh = io.open(path, 'r')
    if fh == nil then
        return nil
    end
    local data = fh:read(262144)
    fh:close()
    return data
end

---True when the name looks like a URL-fetch tool.
---@param name string
---@return boolean
local function looks_like_fetch_tool(name)
    local lower = name:lower()
    return lower:find('fetch', 1, true) ~= nil or lower:find('http', 1, true) ~= nil
        or lower:find('url', 1, true) ~= nil or lower:find('curl', 1, true) ~= nil
end

---Count plain-substring hits of any token in text.
---@param text string
---@param tokens string[]
---@return integer
local function count_hits(text, tokens)
    local hits = 0
    for _, token in ipairs(tokens) do
        local from = 1
        while true do
            local s = text:find(token, from, true)
            if s == nil then
                break
            end
            hits = hits + 1
            from = s + 1
        end
    end
    return hits
end

local function main()
    local mods, boot_err = bootstrap()
    if boot_err ~= nil then
        return fail('bootstrap', boot_err)
    end
    ev('ai.harness.registry and ai.rose.http loaded from DIVER_LUA_DIR')

    -- V1 (default scenario): the tool network path. List the harness
    -- registry's tools — is there any URL-fetch tool at all?
    local reg = mods.registry.new()
    local names = {}
    for name in pairs(reg.tools) do
        names[#names + 1] = tostring(name)
    end
    table.sort(names)
    ev('V1: harness registry tool count: ' .. #names)
    local fetch_tools = {}
    for _, name in ipairs(names) do
        if looks_like_fetch_tool(name) then
            fetch_tools[#fetch_tools + 1] = name
        end
    end
    if #fetch_tools > 0 then
        return fail(
            'recon',
            'a fetch/http/url tool now exists in the harness registry (' .. table.concat(fetch_tools, ', ')
                .. '); the probe premise changed'
        )
    end
    ev('V1: no fetch/http/url tool in the harness registry — and register_builtins registers adapters only (acp/a2a/mcp/phlow/rose/herd), never tools; the design\'s "tool requests an internal address" scenario has no tool to attach to')

    -- The adjacent seam: ai.rose.http, the bounded model-endpoint
    -- client. Read its source and check for the design's pass
    -- criteria: resolved-destination checking against an allowlist.
    local http_path = mods.diver_lua_dir .. '/ai/rose/http.lua'
    local http_src = read_file(http_path)
    if http_src == nil then
        return fail('lua-driver', 'could not read ai/rose/http.lua from DIVER_LUA_DIR')
    end
    ev('read ai/rose/http.lua (' .. #http_src .. ' bytes) for egress-filtering evidence')

    -- A1 (adversarial): 169.254.169.254 must be blocked. The module's
    -- validate_url is a hostname-string check only — no resolved-IP
    -- logic exists.
    local egress_tokens = { 'allowlist', 'denylist', 'blocklist', 'resolved', 'dns', 'getaddrinfo' }
    local egress_hits = count_hits(http_src:lower(), egress_tokens)
    ev('A1: egress-filtering tokens (allowlist/denylist/blocklist/resolved/dns/getaddrinfo) in ai/rose/http.lua: ' .. egress_hits)
    if egress_hits > 0 then
        return fail(
            'recon',
            'ai.rose.http now contains resolved-IP/DNS egress-filtering logic; the probe premise changed'
        )
    end
    ev('A1: validate_url compares the hostname STRING (localhost / 127.0.0.1 / ::1 unless allow_remote) — '
        .. 'no DNS resolution, no resolved-IP allowlist/denylist; 169.254.169.254 is gated only by the allow_remote consent flag, '
        .. 'which is operator configuration of the model endpoint, not a tool-egress filter')

    -- A2 (adversarial): DNS rebinding and redirect chains. The module
    -- refuses redirects on the curl transport (--max-redirs 0) but the
    -- native transport follows them — its own comment says "Current
    -- nightly vim.net follows redirects ... auto must use the safe
    -- adapter until that changes". Either way, there is no per-hop
    -- re-check of the resolved destination.
    local redir_refused = http_src:find('max-redirs', 1, true) ~= nil
    ev('A2: curl transport sets --max-redirs 0 (redirects refused, 3xx treated as failure): ' .. tostring(redir_refused))
    ev('A2: no per-hop redirect re-check exists — each hop\'s resolved destination is never validated; DNS rebinding (evil.com -> 127.0.0.1) has no check to defeat because no resolved-IP check exists at all')

    ev('A3: ai.rose.http is the bounded client for operator-configured model endpoints (Ollama chat requests) — its URL comes from provider config with explicit allow_remote consent; '
        .. 'it is NOT a run\'s tool fetching an agent-chosen URL, so the design\'s "every request\'s resolved destination is checked against the allowlist" has no seam to attach to')

    return fail(
        'seam',
        'seam absent: diver\'s ai.harness has no URL-fetch tool — the registry holds no fetch/http/url tool, so the design\'s '
            .. '"tool requests an internal address" scenarios (public HTTPS, 169.254.169.254, DNS rebinding, redirect chains) have no path to exercise. '
            .. 'The only HTTP module, ai.rose.http, is the bounded model-endpoint client: its validation is a hostname-string check '
            .. 'with no DNS resolution and no resolved-IP allowlist/denylist, and it gates an operator-configured endpoint, not agent-chosen fetches. '
            .. 'Diver-owned finding: flagged, not fixed on gauntlet authority.'
    )
end

local ok, verdict = pcall(main)
if not ok then
    ev('lua error: ' .. tostring(verdict))
    verdict = fail('lua-driver', 'unhandled error: ' .. tostring(verdict))
end
-- verdict on the real stdout: in `nvim --headless -l`, Lua print() goes
-- to stderr, but the Rust runner parses the verdict from stdout.
io.stdout:write(vim.json.encode(verdict) .. '\n')
