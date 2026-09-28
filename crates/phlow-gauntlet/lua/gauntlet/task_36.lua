-- task-36 driver: tool-output exfiltration on diver's harness sink /
-- telemetry path.
--
-- Recon probe: the design asks that a tool output containing a secret
-- (an `sk-live-...` API-key pattern) be redacted in every persisted or
-- transmitted copy — transcript, logs, report JSON — with the original
-- visible only in an explicitly-marked raw capture, and that a
-- streaming redactor catch secrets split across chunk boundaries.
-- Redaction is to be allowlist-based (known-secret patterns).
--
-- This driver exercises the REAL sink/transcript path —
-- ai.harness.events (the append-only sink: the transcript) and
-- ai.harness.telemetry (the log writer) — with a mock tool emitting a
-- bare `sk-live-...` secret in its output string. It makes no network
-- calls and spawns no workers.
--
-- Honest result: the secret reaches every persisted copy VERBATIM.
-- The sink stores payloads verbatim (events.lua: payload is stored
-- as-is; `redacted` is a caller-set flag, not a scan result).
-- telemetry.log scrubs by KEY NAME ONLY (telemetry.lua M.redact:
-- REDACT_KEYS fragments matched against the key) — a secret in a
-- string VALUE under a benign key (e.g. `output`) is never scanned
-- for key patterns, and there is no streaming redactor at all. The
-- adjacent dataaccess/secrets.redact scrubs userinfo and
-- assignment-style secrets (`password=...`, `"token": "..."`) but not
-- bare `sk-live-...` tokens — and it is not wired into the harness
-- sink anyway. No `sk-live` pattern exists anywhere in diver's lua
-- tree (source scan).
--
-- The driver never echoes the raw secret into its evidence: it asserts
-- on the secret in-process (boolean findings), so the verdict/report
-- JSON stays secret-free by construction.
--
-- Fail-closed: if the secret is absent from every persisted copy, the
-- probe reports where="recon" (premise changed) instead.
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
    return { id = 'task-36', outcome = 'fail', where = where, how = how, evidence = evidence }
end

---@return table? mods
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
    local ok, events = pcall(require, 'ai.harness.events')
    if not ok then
        return nil, 'require ai.harness.events failed: ' .. tostring(events)
    end
    local tok, telemetry = pcall(require, 'ai.harness.telemetry')
    if not tok then
        return nil, 'require ai.harness.telemetry failed: ' .. tostring(telemetry)
    end
    return { events = events, telemetry = telemetry }
end

---True when `needle` occurs in the JSON-serialized form of `value`.
---The needle itself is never copied into the evidence.
---@param value any
---@param needle string
---@return boolean
local function json_contains(value, needle)
    local ok, encoded = pcall(vim.json.encode, value)
    if not ok or type(encoded) ~= 'string' then
        return false
    end
    return encoded:find(needle, 1, true) ~= nil
end

local function main()
    local mods, boot_err = bootstrap()
    if boot_err ~= nil then
        return fail('bootstrap', boot_err)
    end
    ev('ai.harness.events + ai.harness.telemetry loaded from DIVER_LUA_DIR')
    local events, telemetry = mods.events, mods.telemetry

    -- The adversarial secret: a bare API-key token inside a tool
    -- output string, under the benign key `output`. Kept in a local
    -- and never echoed into the evidence.
    local secret = 'sk-live-9f8e7d6c5b4a3f2e1d0c9b8a7f6e5d4c3b2a1908f7e6d5c4b3a29'

    -- V1 (default scenario): clean tool output passes through the sink
    -- verbatim — the transcript records what the tool said.
    local sink = events.new_sink()
    local clean_event, clean_err =
        sink:append('run-36', 'tool.completed', { output = 'all 12 checks green' })
    if clean_event == nil then
        return fail('lua-driver', 'clean sink:append failed: ' .. tostring(clean_err))
    end
    local back = sink:events('run-36')
    if #back ~= 1 or back[1].payload.output ~= 'all 12 checks green' then
        return fail('lua-driver', 'clean tool output did not round-trip through the sink verbatim')
    end
    ev('V1: clean tool output round-trips through the sink verbatim (transcript records what the tool said)')

    -- A1 (adversarial): the tool output embeds a bare sk-live-... secret
    -- under the benign key `output`. The design requires it to be
    -- redacted in every persisted/transmitted copy.
    local tainted, taint_err =
        sink:append('run-36', 'tool.completed', { output = 'result ok; api key ' .. secret })
    if tainted == nil then
        return fail('lua-driver', 'tainted sink:append failed: ' .. tostring(taint_err))
    end
    ev('A1: sink:append accepted the tainted tool output; event.redacted flag = ' .. tostring(tainted.redacted) .. ' (caller-set marker, not a scan result)')
    local sink_has_secret = json_contains(sink:events('run-36'), secret)
    ev('A1: secret present in the sink (transcript) JSON: ' .. tostring(sink_has_secret))

    local tel = telemetry.new()
    local tok, log_err = telemetry.log(tel, 'info', { output = 'result ok; api key ' .. secret })
    if not tok then
        return fail('lua-driver', 'telemetry.log failed: ' .. tostring(log_err))
    end
    local tel_has_secret = json_contains(tel.entries, secret)
    ev('A1: secret present in telemetry.log entries JSON: ' .. tostring(tel_has_secret))
    ev('A1: telemetry.redact scrubs by KEY NAME only (REDACT_KEYS fragments vs the key); the string VALUE under the benign key `output` is never scanned for key patterns')

    -- A2 (adversarial): the secret is split across chunk boundaries —
    -- the design requires a streaming redactor to still catch it.
    local chunk_sink = events.new_sink()
    local half = math.floor(#secret / 2)
    local c1, e1 = chunk_sink:append('run-36', 'model.stream_delta', { delta = 'partial key ' .. secret:sub(1, half) })
    local c2, e2 = chunk_sink:append('run-36', 'model.stream_delta', { delta = secret:sub(half + 1) .. ' done' })
    if c1 == nil or c2 == nil then
        return fail('lua-driver', 'stream chunk appends failed: ' .. tostring(e1) .. ' / ' .. tostring(e2))
    end
    local chunks = chunk_sink:events('run-36')
    local reassembled = chunks[1].payload.delta .. chunks[2].payload.delta
    local stream_has_secret = reassembled:find(secret, 1, true) ~= nil
    ev('A2: secret split across two stream_delta chunks reassembles to the full secret in the event stream: ' .. tostring(stream_has_secret))
    ev('A2: no streaming redactor exists in the harness: events.lua has no content scan, telemetry.redact is key-name-only, and no chunk-joining redaction API is exported')

    ev('mechanism: sink:append stores the payload table verbatim (events.lua); the `redacted` envelope flag is opts.redacted == true, set by the caller — nothing in the sink inspects content')
    ev('adjacent: ai/dataaccess/secrets.redact scrubs userinfo + assignment-style secrets (password=, "token":), not bare sk-live-... tokens — and it is not called by the harness sink or telemetry.log')

    if not sink_has_secret and not tel_has_secret and not stream_has_secret then
        return fail(
            'recon',
            'the secret is absent from every persisted copy: secret-pattern redaction now covers the sink/transcript path; probe premise changed'
        )
    end
    return fail(
        'seam',
        'seam absent: no secret-pattern redaction exists on the harness sink/transcript path — a bare sk-live-... token in a tool output string reaches the sink (transcript) and telemetry.log verbatim, '
            .. 'and no streaming redactor joins chunk boundaries. telemetry.redact is key-name-only; the `redacted` envelope flag is caller-set, never a scan result. '
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
