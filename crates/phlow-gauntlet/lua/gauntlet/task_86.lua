-- task_86.lua -- gauntlet driver: MCP capability negotiation mismatch.
--
-- The design asks for negotiation integrity on the MCP client
-- handshake: the client must verify that the server's *behavior*
-- matches its *advertised* capabilities, never issue calls for
-- unadvertised capabilities, surface mismatches as typed errors
-- naming the capability, and record the negotiated capability set
-- in the session record.
--
-- Seam mapping (verified, not invented): diver's `ai.mcp.client`
-- (`lua/ai/mcp/client.lua`) owns the initialize handshake. Its
-- `run_handshake` sends `protocolVersion = '2024-11-05'` with
-- `capabilities = {}` and DISCARDS the initialize result
-- (`function(err, _result)`): the server's advertised capabilities
-- are never read, never verified, never recorded. `M.request`
-- sends any method for any ready session — there is no gating on
-- negotiated capabilities, no `capability_mismatch` typed error,
-- and no session record of what was negotiated.
--
-- This driver plays the REAL client against a scripted python3 mock
-- MCP stdio server (written into GAUNTLET_WORK_DIR at bootstrap):
--   match          V: server advertises {tools:{}} and honors it —
--                    handshake completes, tools/list -> tools/call
--                    round-trips. (The mechanism works.)
--   record         V: documents the absent negotiation record — the
--                    client module exposes no capability/session
--                    introspection, and the wire log shows the client
--                    advertised `"capabilities":[]` (empty Lua table
--                    encodes as a JSON array; the task-08 interop gap).
--   lie            A: server advertises `tools` but tools/list
--                    errors -32602 — the client surfaces a plain
--                    string error; no typed `capability_mismatch`.
--   unadvertised   A: server advertises only `tools`; the driver
--                    calls `resources/list` and the mock's wire log
--                    proves the client SENT it — a call issued for
--                    an unadvertised capability.
--
-- All scenarios write machine-readable traces into
-- GAUNTLET_WORK_DIR (cap-trace.json) for the Rust harness probes,
-- print exactly one JSON verdict line to stdout, always exit 0,
-- write nothing outside GAUNTLET_WORK_DIR, and never modify the
-- diver repo.
--
-- Scenarios via GAUNTLET_SCENARIO (default "match"):
--   match          V: matching capabilities round-trip.
--   record         V: negotiation record is absent.
--   lie            A: advertised-but-broken capability, untyped error.
--   unadvertised   A: call issued for an unadvertised capability.

local TASK_ID = 'task-86'
local SERVER = 'gauntlet-mcp-caps'
local MOCK_SCRIPT = 'gauntlet_mock_mcp_caps.py'
local PYTHON_BIN = '/usr/bin/python3'

local EVIDENCE_MAX = 64
local WAIT_POLL_MS = 25
local START_WAIT_MS = 20000
local CALL_WAIT_MS = 15000

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function fail(where, how)
    return { id = TASK_ID, outcome = 'fail', where = where, how = how, evidence = evidence }
end

local function pass()
    return { id = TASK_ID, outcome = 'pass', evidence = evidence }
end

---Mock MCP stdio server (python3). GAUNTLET_MCP_MODE steers the
---capability story: 'caps-match' advertises {tools:{}} and honors
---it; 'caps-lie' advertises {tools:{}} but errors on tools/list;
---'no-resources' advertises only {tools:{}} and answers anything.
---GAUNTLET_WIRE_LOG (when set) receives one JSON object per
---received request: {"method":..., "params":...}.
local MOCK_SOURCE = [==[
#!/usr/bin/env python3
"""Gauntlet mock MCP stdio server for task-86 (written by the driver)."""
import json
import os
import sys

MODE = os.environ.get("GAUNTLET_MCP_MODE", "caps-match")
WIRE_LOG = os.environ.get("GAUNTLET_WIRE_LOG", "")

TOOLS = [
    {
        "name": "gauntlet_echo",
        "description": "Echo the input text back to the caller",
        "inputSchema": {
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
            "additionalProperties": False,
        },
    },
]


def send(obj):
    sys.stdout.write(json.dumps(obj, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def result(mid, value):
    send({"jsonrpc": "2.0", "id": mid, "result": value})


def error(mid, code, message):
    send({"jsonrpc": "2.0", "id": mid, "error": {"code": code, "message": message}})


def log_wire(method, params):
    if not WIRE_LOG:
        return
    try:
        with open(WIRE_LOG, "a") as fh:
            fh.write(json.dumps({"method": method, "params": params}) + "\n")
    except OSError:
        pass


def main():
    stdin = sys.stdin
    while True:
        line = stdin.readline()
        if line == "":
            break
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        if not isinstance(msg, dict):
            continue
        method = msg.get("method")
        mid = msg.get("id")
        params = msg.get("params")
        log_wire(method, params)
        if method == "initialize":
            result(mid, {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "gauntlet-mock-caps", "version": "0.1.0"},
            })
        elif method == "notifications/initialized":
            pass
        elif method == "tools/list":
            if MODE == "caps-lie":
                error(mid, -32602, "capability not implemented: tools")
            else:
                result(mid, {"tools": TOOLS})
        elif method == "tools/call":
            name = (params or {}).get("name")
            args = (params or {}).get("arguments", {})
            if name == "gauntlet_echo":
                result(mid, {"content": [{"type": "text", "text": args.get("text", "")}],
                             "isError": False})
            else:
                error(mid, -32602, "Unknown tool: %r" % (name,))
        elif mid is not None:
            # Unknown methods (e.g. resources/list in no-resources mode)
            # are answered so the wire log proves the client sent them.
            result(mid, {"ok": True})


if __name__ == "__main__":
    main()
]==]

---Write the mock server into the work dir. Returns path or (nil, err).
---@param work_dir string
---@return string? path
---@return string? err
local function write_mock_server(work_dir)
    local path = work_dir .. '/' .. MOCK_SCRIPT
    local handle, open_err = io.open(path, 'w')
    if handle == nil then
        return nil, 'cannot write mock server: ' .. tostring(open_err)
    end
    handle:write(MOCK_SOURCE)
    handle:close()
    return path, nil
end

---Wire up the rtp, the MCP registry (redirected into the work dir),
---and the mock server file.
---@return table? handles
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
    local registry = require('ai.mcp.registry')
    registry._test_set_data_dir(work_dir .. '/mcp-registry')
    local mock_path, mock_err = write_mock_server(work_dir)
    if mock_path == nil then
        return nil, mock_err
    end
    ev('mock server written: ' .. mock_path)
    local client = require('ai.mcp.client')
    -- Prove we are driving the REAL client module: resolve the source
    -- file of client.start and require the diver path segment.
    local info = debug.getinfo(client.start, 'S')
    local src = (info ~= nil and info.source) or ''
    if not src:find('ai/mcp/client.lua', 1, true) then
        return nil, 'client.start is not the real ai/mcp/client.lua: ' .. src
    end
    ev('driving real module: ' .. src)
    return {
        registry = registry,
        client = client,
        work_dir = work_dir,
        mock_path = mock_path,
    }
end

---NOTE (diver bug, read-only here): ai.mcp.client's spawn_native builds
---the registry entry's `env` as a LIST of "K=V" strings, but vim.system's
---`env` option takes a DICT ({ K = V }); the list form is silently
---ignored, so per-server registry env never reaches the child (verified
---in task-08). The driver steers the mock via vim.env, which spawned
---children inherit.
---@param handles table
---@param mode string
---@return string? err
local function register_server(handles, mode)
    vim.env.GAUNTLET_MCP_MODE = mode
    vim.env.GAUNTLET_WIRE_LOG = handles.work_dir .. '/wire.log'
    local f = io.open(handles.work_dir .. '/wire.log', 'w')
    if f ~= nil then
        f:close()
    end
    -- The registry persists on disk in the work dir, and the task-level
    -- driver runs all four scenarios against the same work dir: clear a
    -- stale entry so each scenario re-registers from a clean slate.
    handles.registry.remove(SERVER)
    local ok, err = handles.registry.add({
        name = SERVER,
        command = PYTHON_BIN,
        args = { handles.mock_path },
        enabled = true,
    })
    if not ok then
        return 'registry.add failed: ' .. tostring(err)
    end
    ev('registered MCP server: ' .. SERVER .. ' mode=' .. mode)
    return nil
end

---Synchronous wrapper over the callback-style client.start.
---@param handles table
---@param timeout_ms integer
---@return string? err
local function client_start_sync(handles, timeout_ms)
    local done = false
    local err_out = nil
    handles.client.start(SERVER, function(err)
        err_out = err
        done = true
    end)
    local deadline = vim.uv.hrtime() + (timeout_ms * 1000000)
    while not done do
        if vim.uv.hrtime() >= deadline then
            return 'client.start wait timed out'
        end
        vim.wait(WAIT_POLL_MS)
    end
    return err_out
end

---Synchronous wrapper over the callback-style client.request.
---@param handles table
---@param method string
---@param params table
---@param timeout_ms integer
---@return table? result
---@return any err (string on client errors; kept untyped on purpose)
local function client_request_sync(handles, method, params, timeout_ms)
    local done = false
    local err_out = nil
    local result_out = nil
    handles.client.request(SERVER, method, params, function(err, result)
        err_out = err
        result_out = result
        done = true
    end, timeout_ms)
    local deadline = vim.uv.hrtime() + (timeout_ms * 1000000)
    while not done do
        if vim.uv.hrtime() >= deadline then
            return nil, 'client.request wait timed out'
        end
        vim.wait(WAIT_POLL_MS)
    end
    return result_out, err_out
end

---Write `text` to a file inside GAUNTLET_WORK_DIR.
---@param handles table
---@param name string
---@param text string
---@return string? err
local function write_work_file(handles, name, text)
    local path = handles.work_dir .. '/' .. name
    local f, err = io.open(path, 'w')
    if f == nil then
        return 'cannot open ' .. path .. ': ' .. tostring(err)
    end
    f:write(text)
    f:close()
    return nil
end

---Read the wire log into a list of {method=..., params=...}.
---@param handles table
---@return table[] entries
local function read_wire_log(handles)
    local entries = {}
    local f = io.open(handles.work_dir .. '/wire.log', 'r')
    if f == nil then
        return entries
    end
    for line in f:lines() do
        local ok, entry = pcall(vim.json.decode, line)
        if ok and type(entry) == 'table' then
            entries[#entries + 1] = entry
        end
    end
    f:close()
    return entries
end

---Sorted list of the module's function names.
---@param mod table
---@return string[] names
local function module_functions(mod)
    local names = {}
    for k, v in pairs(mod) do
        if type(v) == 'function' then
            names[#names + 1] = k
        end
    end
    table.sort(names)
    return names
end

---Scenario "match" (V): matching capabilities — handshake completes
---and tools/list -> tools/call round-trips against the real client.
---@param handles table
---@return table verdict
local function scenario_match(handles)
    local reg_err = register_server(handles, 'caps-match')
    if reg_err ~= nil then
        return fail('register', reg_err)
    end
    local start_err = client_start_sync(handles, START_WAIT_MS)
    if start_err ~= nil then
        return fail('handshake', 'client.start failed: ' .. tostring(start_err))
    end
    ev('handshake completed against caps-match mock')
    local tools, list_err = client_request_sync(handles, 'tools/list', {}, CALL_WAIT_MS)
    if tools == nil then
        return fail('tools-list', 'tools/list failed: ' .. tostring(list_err))
    end
    if type(tools.tools) ~= 'table' or #tools.tools ~= 1 then
        return fail('tools-list', 'unexpected tools/list result')
    end
    ev('tools/list returned 1 tool')
    local result, call_err = client_request_sync(
        handles,
        'tools/call',
        { name = 'gauntlet_echo', arguments = { text = 'hello' } },
        CALL_WAIT_MS
    )
    if result == nil then
        return fail('tools-call', 'tools/call failed: ' .. tostring(call_err))
    end
    local text = result.content ~= nil and result.content[1] ~= nil and result.content[1].text
    if text ~= 'hello' then
        return fail('tools-call', 'echo round-trip mismatch')
    end
    ev('tools/call echo round-trip ok')
    handles.client.stop(SERVER)
    local trace = {
        scenario = 'match',
        handshake_ok = true,
        roundtrip_ok = true,
        server_advertised = { 'tools' },
    }
    local werr = write_work_file(handles, 'cap-trace.json', vim.json.encode(trace))
    if werr ~= nil then
        return fail('trace', werr)
    end
    return pass()
end

---Scenario "record" (V): the negotiation record does not exist. The
---client module exposes no capability/session introspection, and the
---wire log shows the client advertised `"capabilities":[]`.
---@param handles table
---@return table verdict
local function scenario_record(handles)
    local reg_err = register_server(handles, 'caps-match')
    if reg_err ~= nil then
        return fail('register', reg_err)
    end
    local start_err = client_start_sync(handles, START_WAIT_MS)
    if start_err ~= nil then
        return fail('handshake', 'client.start failed: ' .. tostring(start_err))
    end
    local names = module_functions(handles.client)
    ev('client functions: ' .. table.concat(names, ', '))
    for _, name in ipairs(names) do
        local lower = name:lower()
        if lower:find('capab', 1, true) ~= nil or lower:find('negotiat', 1, true) ~= nil then
            return fail('record', 'unexpected capability introspection: ' .. name)
        end
    end
    ev('no capability/negotiation introspection in the client API')
    local advertised = nil
    for _, entry in ipairs(read_wire_log(handles)) do
        if entry.method == 'initialize' and type(entry.params) == 'table' then
            advertised = entry.params.capabilities
        end
    end
    -- An empty Lua table encodes as a JSON array: the client advertises
    -- "capabilities":[] — the task-08 interop gap, observed on the wire.
    local advertised_is_array = type(advertised) == 'table' and #advertised == 0
        and next(advertised) == nil
    ev('client advertised capabilities on the wire: ' .. vim.json.encode(advertised))
    if not advertised_is_array then
        return fail('record', 'expected the client to advertise "capabilities":[]')
    end
    handles.client.stop(SERVER)
    local trace = {
        scenario = 'record',
        client_advertised_caps = '[]',
        server_advertised_caps = { 'tools' },
        negotiated_record = false,
    }
    local werr = write_work_file(handles, 'cap-trace.json', vim.json.encode(trace))
    if werr ~= nil then
        return fail('trace', werr)
    end
    return pass()
end

---Scenario "lie" (A): the server advertises `tools` but tools/list
---errors -32602. The client surfaces a plain string error — there is
---no typed `capability_mismatch` naming the capability.
---@param handles table
---@return table verdict
local function scenario_lie(handles)
    local reg_err = register_server(handles, 'caps-lie')
    if reg_err ~= nil then
        return fail('register', reg_err)
    end
    local start_err = client_start_sync(handles, START_WAIT_MS)
    if start_err ~= nil then
        return fail('handshake', 'client.start failed: ' .. tostring(start_err))
    end
    ev('handshake completed against the lying mock (advertised tools)')
    local tools, list_err = client_request_sync(handles, 'tools/list', {}, CALL_WAIT_MS)
    if tools ~= nil then
        return fail('lie', 'tools/list unexpectedly succeeded against the lying mock')
    end
    if type(list_err) ~= 'string' then
        return fail('lie', 'expected a string error, got: ' .. type(list_err))
    end
    ev('tools/list error surfaced as a plain string: ' .. list_err)
    local typed = type(list_err) == 'table'
    handles.client.stop(SERVER)
    local trace = {
        scenario = 'lie',
        advertised = { 'tools' },
        error_text = list_err,
        typed_capability_mismatch = typed,
    }
    local werr = write_work_file(handles, 'cap-trace.json', vim.json.encode(trace))
    if werr ~= nil then
        return fail('trace', werr)
    end
    return pass()
end

---Scenario "unadvertised" (A): the server advertises only `tools`;
---the driver calls `resources/list`. The mock's wire log proves the
---client SENT the request — a call issued for an unadvertised
---capability, with no gating and no complaint.
---@param handles table
---@return table verdict
local function scenario_unadvertised(handles)
    local reg_err = register_server(handles, 'no-resources')
    if reg_err ~= nil then
        return fail('register', reg_err)
    end
    local start_err = client_start_sync(handles, START_WAIT_MS)
    if start_err ~= nil then
        return fail('handshake', 'client.start failed: ' .. tostring(start_err))
    end
    ev('handshake completed; server advertised only tools')
    local _, req_err =
        client_request_sync(handles, 'resources/list', {}, CALL_WAIT_MS)
    if req_err ~= nil then
        return fail(
            'unadvertised',
            'resources/list was refused client-side (gating exists?): ' .. tostring(req_err)
        )
    end
    local sent = false
    for _, entry in ipairs(read_wire_log(handles)) do
        if entry.method == 'resources/list' then
            sent = true
        end
    end
    if not sent then
        return fail('unadvertised', 'resources/list is missing from the wire log')
    end
    ev('wire log proves resources/list was SENT for an unadvertised capability')
    handles.client.stop(SERVER)
    local trace = {
        scenario = 'unadvertised',
        advertised = { 'tools' },
        unadvertised_call_sent = sent,
    }
    local werr = write_work_file(handles, 'cap-trace.json', vim.json.encode(trace))
    if werr ~= nil then
        return fail('trace', werr)
    end
    return pass()
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'match'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return fail('bootstrap', boot_err)
    end
    ev('scenario=' .. scenario)
    if scenario == 'match' then
        return scenario_match(handles)
    elseif scenario == 'record' then
        return scenario_record(handles)
    elseif scenario == 'lie' then
        return scenario_lie(handles)
    elseif scenario == 'unadvertised' then
        return scenario_unadvertised(handles)
    end
    return fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
end

local ok, verdict = pcall(main)
if not ok then
    ev('lua error: ' .. tostring(verdict))
    verdict = fail('lua-driver', 'unhandled error: ' .. tostring(verdict))
end
-- verdict on the real stdout: in `nvim --headless -l`, Lua print() goes
-- to stderr, but the Rust runner parses the verdict from stdout.
io.stdout:write(vim.json.encode(verdict) .. '\n')
