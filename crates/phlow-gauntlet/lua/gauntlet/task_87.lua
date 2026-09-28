-- task_87.lua -- gauntlet driver: MCP session resumption.
--
-- The design asks for resumption discipline across server restarts:
-- the client re-handshakes and resumes with at-most-once call
-- semantics; a non-idempotent in-flight call at restart must fail
-- closed with `unknown_call_outcome` (never blindly retried); a
-- restarted server returning a different tool list must invalidate
-- the session; the re-handshake must be observable in the session
-- record with a marked resumption boundary.
--
-- Seam mapping (verified, not invented): diver's `ai.mcp.client`
-- (`lua/ai/mcp/client.lua`) keeps sessions in a module-local table
-- keyed by server name. `teardown` on server exit bumps the
-- generation, resolves every pending request with a plain string
-- reason, closes the backend, and DELETES the session
-- (`sessions[name] = nil`). There is no re-handshake path, no
-- session record, no idempotency tracking, no tool-list identity,
-- and no resumption-boundary marking: after a death, `M.request`
-- reports 'server is not running' until the caller starts over by
-- hand, and a restarted server's changed tool list is served with
-- no invalidation.
--
-- This driver plays the REAL client against a scripted python3 mock
-- MCP stdio server (written into GAUNTLET_WORK_DIR at bootstrap):
--   restart-between  V: server dies between calls — no auto-resume
--                      ('server is not running'); a manual M.start
--                      re-handshakes and tools/list works again, but
--                      nothing marks the resumption boundary.
--   identity-change  V (adversarial-in-V): the restarted server
--                      returns a different tool list — the client
--                      serves it with no invalidation.
--   inflight         A: a tools/call is in flight when the server
--                      dies — the pending call resolves with the
--                      exit reason, is never retried (wire log shows
--                      exactly one tools/call), but there is no typed
--                      `unknown_call_outcome` and no idempotency
--                      tracking.
--   no-record        A: the client exposes no session-record API —
--                      no re-handshake observability, no resumption
--                      boundary marker.
--
-- All scenarios write machine-readable traces into
-- GAUNTLET_WORK_DIR (resume-trace.json) for the Rust harness
-- probes, print exactly one JSON verdict line to stdout, always
-- exit 0, write nothing outside GAUNTLET_WORK_DIR, and never
-- modify the diver repo.
--
-- Scenarios via GAUNTLET_SCENARIO (default "restart-between"):
--   restart-between  V: death between calls, manual re-handshake.
--   identity-change  V: changed tool list after restart, no invalidation.
--   inflight         A: in-flight call at restart, no retry, untyped error.
--   no-record        A: no session record / resumption boundary API.

local TASK_ID = 'task-87'
local SERVER = 'gauntlet-mcp-resume'
local MOCK_SCRIPT = 'gauntlet_mock_mcp_resume.py'
local PYTHON_BIN = '/usr/bin/python3'

local EVIDENCE_MAX = 64
local WAIT_POLL_MS = 25
local START_WAIT_MS = 20000
local CALL_WAIT_MS = 15000
local DEATH_WAIT_MS = 10000

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

---Mock MCP stdio server (python3). GAUNTLET_MCP_TOOLSET selects the
---tool list served at startup: 'a' -> gauntlet_echo + gauntlet_suicide,
---'b' -> gauntlet_other. The `gauntlet-die` method replies ok and then
---exits(0); tools/call for `gauntlet_suicide` sleeps briefly and then
---exits(1) WITHOUT replying (an in-flight call at restart).
---GAUNTLET_WIRE_LOG receives one JSON object per received request.
local MOCK_SOURCE = [==[
#!/usr/bin/env python3
"""Gauntlet mock MCP stdio server for task-87 (written by the driver)."""
import json
import os
import sys
import time

TOOLSET = os.environ.get("GAUNTLET_MCP_TOOLSET", "a")
WIRE_LOG = os.environ.get("GAUNTLET_WIRE_LOG", "")

TOOLS_A = [
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
    {
        "name": "gauntlet_suicide",
        "description": "Die mid-call without replying",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
    },
]

TOOLS_B = [
    {
        "name": "gauntlet_other",
        "description": "A different tool after the restart",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
    },
]

TOOLS = TOOLS_A if TOOLSET == "a" else TOOLS_B


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
                "serverInfo": {"name": "gauntlet-mock-resume", "version": "0.1.0"},
            })
        elif method == "notifications/initialized":
            pass
        elif method == "gauntlet-die":
            result(mid, {"ok": True})
            sys.stdout.flush()
            time.sleep(0.1)
            os._exit(0)
        elif method == "tools/list":
            result(mid, {"tools": TOOLS})
        elif method == "tools/call":
            name = (params or {}).get("name")
            args = (params or {}).get("arguments", {})
            if name == "gauntlet_suicide":
                # In-flight death: no reply, the process just goes away.
                time.sleep(0.3)
                os._exit(1)
            if name == "gauntlet_echo":
                result(mid, {"content": [{"type": "text", "text": args.get("text", "")}],
                             "isError": False})
            else:
                error(mid, -32602, "Unknown tool: %r" % (name,))
        elif mid is not None:
            error(mid, -32601, "Method not found: %r" % (method,))


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

---Register the mock server. The mock is steered via vim.env (see the
---task-86 note: registry-entry env is silently dropped by the real
---client's spawn_native).
---@param handles table
---@param toolset string
---@return string? err
local function register_server(handles, toolset)
    vim.env.GAUNTLET_MCP_TOOLSET = toolset
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
    ev('registered MCP server: ' .. SERVER .. ' toolset=' .. toolset)
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
---@return any err
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

---Wait until client.request reports the server as not running
---(teardown completed after the death), or the deadline.
---@param handles table
---@return boolean dead
local function wait_server_gone(handles)
    local deadline = vim.uv.hrtime() + (DEATH_WAIT_MS * 1000000)
    while vim.uv.hrtime() < deadline do
        local _, err = client_request_sync(handles, 'tools/list', {}, 2000)
        if type(err) == 'string' and err:find('not running', 1, true) ~= nil then
            return true
        end
        vim.wait(WAIT_POLL_MS)
    end
    return false
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

---Sorted tool names from a tools/list result.
---@param tools table
---@return string[] names
local function tool_names(tools)
    local names = {}
    if type(tools.tools) == 'table' then
        for _, tool in ipairs(tools.tools) do
            if type(tool.name) == 'string' then
                names[#names + 1] = tool.name
            end
        end
    end
    table.sort(names)
    return names
end

---Scenario "restart-between" (V): the server dies between calls. The
---client does NOT auto-resume (M.request -> 'server is not running');
---a manual M.start re-handshakes and tools/list works again — but no
---session record marks the resumption boundary.
---@param handles table
---@return table verdict
local function scenario_restart_between(handles)
    local reg_err = register_server(handles, 'a')
    if reg_err ~= nil then
        return fail('register', reg_err)
    end
    local start_err = client_start_sync(handles, START_WAIT_MS)
    if start_err ~= nil then
        return fail('handshake', 'client.start failed: ' .. tostring(start_err))
    end
    local tools, list_err = client_request_sync(handles, 'tools/list', {}, CALL_WAIT_MS)
    if tools == nil then
        return fail('tools-list', 'tools/list failed: ' .. tostring(list_err))
    end
    ev('tools/list ok before the death: ' .. table.concat(tool_names(tools), ','))
    local _, die_err = client_request_sync(handles, 'gauntlet-die', {}, CALL_WAIT_MS)
    if die_err ~= nil then
        return fail('die', 'gauntlet-die failed: ' .. tostring(die_err))
    end
    if not wait_server_gone(handles) then
        return fail('death', 'server never reported as gone after gauntlet-die')
    end
    ev("after the death M.request reports 'server is not running': no auto-resume")
    local start_err2 = client_start_sync(handles, START_WAIT_MS)
    if start_err2 ~= nil then
        return fail('rehandshake', 'manual re-start failed: ' .. tostring(start_err2))
    end
    ev('manual M.start re-handshook successfully')
    local tools2, list_err2 = client_request_sync(handles, 'tools/list', {}, CALL_WAIT_MS)
    if tools2 == nil then
        return fail('tools-list-2', 'tools/list after restart failed: ' .. tostring(list_err2))
    end
    ev('tools/list ok after the restart: ' .. table.concat(tool_names(tools2), ','))
    handles.client.stop(SERVER)
    local trace = {
        scenario = 'restart-between',
        auto_resume = false,
        rehandshake_ok = true,
        resumption_boundary_marked = false,
    }
    local werr = write_work_file(handles, 'resume-trace.json', vim.json.encode(trace))
    if werr ~= nil then
        return fail('trace', werr)
    end
    return pass()
end

---Scenario "identity-change" (V, adversarial-in-V): the restarted
---server returns a DIFFERENT tool list. The client serves the new
---list with no invalidation — there is no tool-list identity to
---compare against.
---@param handles table
---@return table verdict
local function scenario_identity_change(handles)
    local reg_err = register_server(handles, 'a')
    if reg_err ~= nil then
        return fail('register', reg_err)
    end
    local start_err = client_start_sync(handles, START_WAIT_MS)
    if start_err ~= nil then
        return fail('handshake', 'client.start failed: ' .. tostring(start_err))
    end
    local tools = client_request_sync(handles, 'tools/list', {}, CALL_WAIT_MS)
    if tools == nil then
        return fail('tools-list', 'tools/list failed before the restart')
    end
    local names_a = tool_names(tools)
    ev('tool list before restart: ' .. table.concat(names_a, ','))
    local _, die_err = client_request_sync(handles, 'gauntlet-die', {}, CALL_WAIT_MS)
    if die_err ~= nil then
        return fail('die', 'gauntlet-die failed: ' .. tostring(die_err))
    end
    if not wait_server_gone(handles) then
        return fail('death', 'server never reported as gone after gauntlet-die')
    end
    vim.env.GAUNTLET_MCP_TOOLSET = 'b'
    local start_err2 = client_start_sync(handles, START_WAIT_MS)
    if start_err2 ~= nil then
        return fail('rehandshake', 'manual re-start failed: ' .. tostring(start_err2))
    end
    local tools2 = client_request_sync(handles, 'tools/list', {}, CALL_WAIT_MS)
    if tools2 == nil then
        return fail('tools-list-2', 'tools/list failed after the restart')
    end
    local names_b = tool_names(tools2)
    ev('tool list after restart: ' .. table.concat(names_b, ','))
    if table.concat(names_a, ',') == table.concat(names_b, ',') then
        return fail('identity-change', 'tool list did not change; the mock steer failed')
    end
    ev('changed tool list served with no invalidation — no tool-list identity exists')
    handles.client.stop(SERVER)
    local trace = {
        scenario = 'identity-change',
        old_tools = names_a,
        new_tools = names_b,
        session_invalidated = false,
    }
    local werr = write_work_file(handles, 'resume-trace.json', vim.json.encode(trace))
    if werr ~= nil then
        return fail('trace', werr)
    end
    return pass()
end

---Scenario "inflight" (A): a tools/call is in flight when the server
---dies (gauntlet_suicide exits(1) with no reply). The pending call
---resolves with the exit reason and is NEVER retried (the wire log
---shows exactly one tools/call) — but the error is a plain string:
---no typed `unknown_call_outcome`, and no idempotency tracking that
---could distinguish a safe retry from an unsafe one.
---@param handles table
---@return table verdict
local function scenario_inflight(handles)
    local reg_err = register_server(handles, 'a')
    if reg_err ~= nil then
        return fail('register', reg_err)
    end
    local start_err = client_start_sync(handles, START_WAIT_MS)
    if start_err ~= nil then
        return fail('handshake', 'client.start failed: ' .. tostring(start_err))
    end
    local result, call_err = client_request_sync(
        handles,
        'tools/call',
        { name = 'gauntlet_suicide', arguments = {} },
        CALL_WAIT_MS
    )
    if result ~= nil then
        return fail('inflight', 'suicide call unexpectedly returned a result')
    end
    if type(call_err) ~= 'string' then
        return fail('inflight', 'expected a string error, got: ' .. type(call_err))
    end
    ev('in-flight call resolved with: ' .. call_err)
    local calls = 0
    for _, entry in ipairs(read_wire_log(handles)) do
        if entry.method == 'tools/call' then
            calls = calls + 1
        end
    end
    ev('tools/call requests on the wire: ' .. calls)
    if calls ~= 1 then
        return fail('inflight', 'expected exactly one tools/call on the wire, saw ' .. calls)
    end
    ev('the dead call was never retried — at-most-once by teardown, not by discipline')
    local typed = type(call_err) == 'table'
    local trace = {
        scenario = 'inflight',
        error_text = call_err,
        wire_calls = calls,
        auto_retry = false,
        typed_unknown_call_outcome = typed,
        idempotency_tracked = false,
    }
    local werr = write_work_file(handles, 'resume-trace.json', vim.json.encode(trace))
    if werr ~= nil then
        return fail('trace', werr)
    end
    return pass()
end

---Scenario "no-record" (A): the client exposes no session-record
---API — the re-handshake is not observable and no resumption
---boundary is ever marked.
---@param handles table
---@return table verdict
local function scenario_no_record(handles)
    local names = {}
    for k, v in pairs(handles.client) do
        if type(v) == 'function' then
            names[#names + 1] = k
        end
    end
    table.sort(names)
    ev('client functions: ' .. table.concat(names, ', '))
    for _, name in ipairs(names) do
        local lower = name:lower()
        if lower:find('record', 1, true) ~= nil
            or lower:find('histor', 1, true) ~= nil
            or lower:find('resum', 1, true) ~= nil
            or lower:find('generation', 1, true) ~= nil
        then
            return fail('no-record', 'unexpected session-record API: ' .. name)
        end
    end
    ev('no session-record / resumption-boundary API in the client')
    local trace = {
        scenario = 'no-record',
        session_record_api = false,
        resumption_boundary = false,
    }
    local werr = write_work_file(handles, 'resume-trace.json', vim.json.encode(trace))
    if werr ~= nil then
        return fail('trace', werr)
    end
    return pass()
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'restart-between'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return fail('bootstrap', boot_err)
    end
    ev('scenario=' .. scenario)
    if scenario == 'restart-between' then
        return scenario_restart_between(handles)
    elseif scenario == 'identity-change' then
        return scenario_identity_change(handles)
    elseif scenario == 'inflight' then
        return scenario_inflight(handles)
    elseif scenario == 'no-record' then
        return scenario_no_record(handles)
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
