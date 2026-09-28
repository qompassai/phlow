-- task_90.lua -- gauntlet driver: namespaced cross-protocol dispatch.
--
-- The design asks for a namespaced cross-protocol dispatcher: every
-- addressable target carries its protocol namespace (`mcp:summarize`
-- vs `a2a:summarize`), a name collision across protocols can never
-- cause cross-protocol dispatch, unqualified ambiguous names are
-- rejected (never guessed), and namespaces are assigned by the
-- router — never parsed from advertised names.
--
-- Seam mapping (verified, not invented): diver's harness has NO
-- namespaced dispatcher. Routing is per-run adapter binding:
-- `supervisor.launch` picks exactly one adapter from
-- `spec.adapter` (or capability negotiation) and the run's tool
-- calls stay native to that adapter — MCP tool calls stay on
-- `ai.mcp.client`, A2A calls stay on `ai.a2a.tasks` (see
-- `lua/ai/harness/adapters/mcp.lua`: "tool calls stay on the native
-- ai.mcp.tools API"). The tool registry (`ai.harness.registry`)
-- rejects any name outside `^[a-z][a-z0-9_]*$`, so a `proto:name`
-- address is inexpressible: `register_tool(reg, 'mcp:summarize')`
-- fails the name pattern. No function in ai.harness parses a
-- protocol prefix.
--
-- This driver plays the REAL harness/registry/client against a
-- scripted python3 mock MCP stdio server (written into
-- GAUNTLET_WORK_DIR at bootstrap):
--   distinct      V: the 'mcp' and 'a2a' adapters are distinct
--                   registrations with protocol-pure start paths;
--                   an mcp run completes against the mock with only
--                   mcp-sourced events — no cross traffic.
--   no-namespace  V: no dispatch entry point exists — the harness
--                   surface is setup/run/cancel/resume/version, and
--                   register_tool rejects 'mcp:summarize' (colon
--                   violates the name pattern).
--   spoof         A: the mock advertises a tool literally named
--                   `a2a:send`; calling it sends the literal name on
--                   the MCP wire — no prefix parsing, no A2A traffic.
--   ambiguous     A (adversarial-in-V): a bare-name duplicate is
--                   rejected ('already registered') but there is no
--                   namespace to omit or collide on — cross-protocol
--                   name collision is inexpressible, not rejected.
--
-- All scenarios write machine-readable traces into
-- GAUNTLET_WORK_DIR (dispatch-trace.json) for the Rust harness
-- probes, print exactly one JSON verdict line to stdout, always
-- exit 0, write nothing outside GAUNTLET_WORK_DIR, and never
-- modify the diver repo.
--
-- Scenarios via GAUNTLET_SCENARIO (default "distinct"):
--   distinct       V: adapter disjointness, mcp run completes.
--   no-namespace   V: no dispatch entry; colon names rejected.
--   spoof          A: spoofed-prefix tool stays on the MCP wire.
--   ambiguous      A: bare duplicates rejected; namespaces inexpressible.

local TASK_ID = 'task-90'
local SERVER = 'gauntlet-mcp-dispatch'
local MOCK_SCRIPT = 'gauntlet_mock_mcp_dispatch.py'
local PYTHON_BIN = '/usr/bin/python3'

local EVIDENCE_MAX = 64
local WAIT_POLL_MS = 25
local RUN_TIMEOUT_MS = 60000
local COMPLETE_WAIT_MS = 30000
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

---Mock MCP stdio server (python3). Advertises `gauntlet_echo` and a
---tool literally named `a2a:send` (the namespace-spoofing prefix).
---GAUNTLET_WIRE_LOG receives one JSON object per received request.
local MOCK_SOURCE = [==[
#!/usr/bin/env python3
"""Gauntlet mock MCP stdio server for task-90 (written by the driver)."""
import json
import os
import sys

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
    {
        "name": "a2a:send",
        "description": "A tool wearing a foreign protocol prefix",
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
                "serverInfo": {"name": "gauntlet-mock-dispatch", "version": "0.1.0"},
            })
        elif method == "notifications/initialized":
            pass
        elif method == "tools/list":
            result(mid, {"tools": TOOLS})
        elif method == "tools/call":
            name = (params or {}).get("name")
            args = (params or {}).get("arguments", {})
            tool = next((t for t in TOOLS if t["name"] == name), None)
            if tool is None:
                error(mid, -32602, "Unknown tool: %r" % (name,))
            else:
                result(mid, {"content": [{"type": "text", "text": args.get("text", "")}],
                             "isError": False})
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

---Wire up the harness, the MCP registry (redirected into the work
---dir), and the mock server file.
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
    local harness = require('ai.harness')
    local ok, err = harness.setup({})
    if not ok then
        return nil, 'harness.setup failed: ' .. tostring(err)
    end
    local st = harness._state
    if st == nil then
        return nil, 'harness internal state unavailable after setup'
    end
    local registry = require('ai.mcp.registry')
    registry._test_set_data_dir(work_dir .. '/mcp-registry')
    local mock_path, mock_err = write_mock_server(work_dir)
    if mock_path == nil then
        return nil, mock_err
    end
    ev('mock server written: ' .. mock_path)
    vim.env.GAUNTLET_WIRE_LOG = work_dir .. '/wire.log'
    local f = io.open(work_dir .. '/wire.log', 'w')
    if f ~= nil then
        f:close()
    end
    -- The registry persists on disk in the work dir, and the task-level
    -- driver runs all four scenarios against the same work dir: clear a
    -- stale entry so each scenario re-registers from a clean slate.
    registry.remove(SERVER)
    local reg_ok, reg_err = registry.add({
        name = SERVER,
        command = PYTHON_BIN,
        args = { mock_path },
        enabled = true,
    })
    if not reg_ok then
        return nil, 'registry.add failed: ' .. tostring(reg_err)
    end
    ev('registered MCP server: ' .. SERVER)
    return {
        harness = harness,
        sup = st.supervisor,
        sink = st.sink,
        reg_state = st.registry,
        supervisor = require('ai.harness.supervisor'),
        hregistry = require('ai.harness.registry'),
        types = require('ai.harness.types'),
        client = require('ai.mcp.client'),
        work_dir = work_dir,
        mock_path = mock_path,
    }
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

---Drive the supervisor clock until the run reaches `want`.
---@param handles table
---@param run_id string
---@param want string
---@param timeout_ms integer
---@return boolean reached
---@return string state
local function wait_state(handles, run_id, want, timeout_ms)
    local deadline = vim.uv.hrtime() + (timeout_ms * 1000000)
    while true do
        handles.supervisor.tick(handles.sup, handles.types.now_ns())
        local run = handles.supervisor.get(handles.sup, run_id)
        local state = (run == nil) and 'missing' or run.state
        if state == want then
            return true, state
        end
        if vim.uv.hrtime() >= deadline then
            return false, state
        end
        vim.wait(WAIT_POLL_MS)
    end
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

---Sorted list of a module's function names.
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

---Scenario "distinct" (V): the 'mcp' and 'a2a' adapters are distinct
---registrations with protocol-pure start paths, and an mcp run
---completes against the mock with only mcp-sourced events — routing
---is per-run adapter binding, and no call crosses protocols.
---@param handles table
---@return table verdict
local function scenario_distinct(handles)
    local hreg = handles.hregistry
    local mcp = hreg.get_adapter(handles.reg_state, 'mcp')
    local a2a = hreg.get_adapter(handles.reg_state, 'a2a')
    if mcp == nil or a2a == nil then
        return fail('adapters', 'builtin mcp/a2a adapters not registered')
    end
    if mcp == a2a then
        return fail('adapters', 'mcp and a2a adapters are the same table')
    end
    local mcp_src = (debug.getinfo(mcp.start, 'S') or {}).source or ''
    local a2a_src = (debug.getinfo(a2a.start, 'S') or {}).source or ''
    ev('mcp adapter start: ' .. mcp_src)
    ev('a2a adapter start: ' .. a2a_src)
    if not mcp_src:find('adapters/mcp.lua', 1, true) then
        return fail('adapters', 'mcp adapter is not the real ai.harness.adapters.mcp')
    end
    if not a2a_src:find('adapters/a2a.lua', 1, true) then
        return fail('adapters', 'a2a adapter is not the real ai.harness.adapters.a2a')
    end
    local run_id, run_err = handles.harness.run({
        workflow = 'gauntlet-dispatch',
        goal = 'namespaced dispatch probe',
        workspace = handles.work_dir,
        adapter = 'mcp',
        timeout_ms = RUN_TIMEOUT_MS,
        extensions = { mcp = { server = SERVER } },
    })
    if run_id == nil then
        return fail('run-start', 'harness.run failed: ' .. tostring(run_err))
    end
    local reached, state = wait_state(handles, run_id, 'completed', COMPLETE_WAIT_MS)
    if not reached then
        return fail('run-start', 'mcp run never completed, state=' .. state)
    end
    ev('mcp run completed against the mock')
    for _, event in ipairs(handles.sink:events(run_id)) do
        local source = event.source
        if source ~= nil and source ~= 'mcp' and source ~= 'supervisor' then
            return fail('distinct', 'non-mcp event source on the mcp run: ' .. tostring(source))
        end
    end
    ev('all run events are mcp/supervisor-sourced: no cross-protocol traffic')
    local trace = {
        scenario = 'distinct',
        adapters_disjoint = true,
        mcp_run_completed = true,
        cross_protocol_traffic = false,
    }
    local werr = write_work_file(handles, 'dispatch-trace.json', vim.json.encode(trace))
    if werr ~= nil then
        return fail('trace', werr)
    end
    return pass()
end

---Scenario "no-namespace" (V): there is no dispatch entry point —
---the harness surface is setup/run/cancel/resume/version, and the
---registry rejects 'mcp:summarize' (the colon violates the
---`^[a-z][a-z0-9_]*$` name pattern), so `proto:name` addressing is
---inexpressible.
---@param handles table
---@return table verdict
local function scenario_no_namespace(handles)
    local harness_fns = module_functions(handles.harness)
    ev('ai.harness functions: ' .. table.concat(harness_fns, ', '))
    for _, name in ipairs(harness_fns) do
        local lower = name:lower()
        if lower:find('dispatch', 1, true) ~= nil
            or lower:find('namespace', 1, true) ~= nil
            or lower:find('route', 1, true) ~= nil
        then
            return fail('no-namespace', 'unexpected dispatch entry point: ' .. name)
        end
    end
    local reg_fns = module_functions(handles.hregistry)
    ev('ai.harness.registry functions: ' .. table.concat(reg_fns, ', '))
    for _, name in ipairs(reg_fns) do
        local lower = name:lower()
        if lower:find('dispatch', 1, true) ~= nil or lower:find('namespace', 1, true) ~= nil then
            return fail('no-namespace', 'unexpected registry dispatch entry: ' .. name)
        end
    end
    local ok, reg_err = handles.hregistry.register_tool(
        handles.reg_state,
        'mcp:summarize',
        { description = 'namespaced probe' }
    )
    if ok then
        return fail('no-namespace', "register_tool accepted 'mcp:summarize'")
    end
    ev("register_tool rejected 'mcp:summarize': " .. tostring(reg_err))
    if not tostring(reg_err):find('name must match', 1, true) then
        return fail('no-namespace', 'unexpected rejection reason: ' .. tostring(reg_err))
    end
    local trace = {
        scenario = 'no-namespace',
        dispatch_entry = false,
        colon_names_rejected = true,
    }
    local werr = write_work_file(handles, 'dispatch-trace.json', vim.json.encode(trace))
    if werr ~= nil then
        return fail('trace', werr)
    end
    return pass()
end

---Scenario "spoof" (A): the mock advertises a tool literally named
---`a2a:send`. Calling it sends the literal name on the MCP wire —
---no prefix is parsed, no namespace is assigned, and no A2A traffic
---occurs.
---@param handles table
---@return table verdict
local function scenario_spoof(handles)
    local start_err = client_start_sync(handles, COMPLETE_WAIT_MS)
    if start_err ~= nil then
        return fail('handshake', 'client.start failed: ' .. tostring(start_err))
    end
    local tools, list_err = client_request_sync(handles, 'tools/list', {}, CALL_WAIT_MS)
    if tools == nil then
        return fail('tools-list', 'tools/list failed: ' .. tostring(list_err))
    end
    local saw_spoof = false
    for _, tool in ipairs(tools.tools or {}) do
        if tool.name == 'a2a:send' then
            saw_spoof = true
        end
    end
    if not saw_spoof then
        return fail('spoof', 'mock did not advertise the a2a:send tool')
    end
    ev('mock advertises a tool literally named a2a:send')
    local result, call_err = client_request_sync(
        handles,
        'tools/call',
        { name = 'a2a:send', arguments = { text = 'ping' } },
        CALL_WAIT_MS
    )
    if result == nil then
        return fail('tools-call', 'tools/call for a2a:send failed: ' .. tostring(call_err))
    end
    local sent_name = nil
    for _, entry in ipairs(read_wire_log(handles)) do
        if entry.method == 'tools/call'
            and type(entry.params) == 'table'
            and entry.params.name == 'a2a:send'
        then
            sent_name = entry.params.name
        end
    end
    if sent_name == nil then
        return fail('spoof', 'tools/call for a2a:send missing from the wire log')
    end
    ev('wire shows the literal name a2a:send: treated opaquely, never parsed')
    handles.client.stop(SERVER)
    local trace = {
        scenario = 'spoof',
        advertised_spoof = 'a2a:send',
        wire_name = sent_name,
        prefix_parsed = false,
        cross_protocol_dispatch = false,
    }
    local werr = write_work_file(handles, 'dispatch-trace.json', vim.json.encode(trace))
    if werr ~= nil then
        return fail('trace', werr)
    end
    return pass()
end

---Scenario "ambiguous" (A, adversarial-in-V): a bare-name duplicate
---is rejected ('already registered'), but there is no namespace to
---omit or collide on — a cross-protocol name collision is
---inexpressible, not rejected as ambiguous.
---@param handles table
---@return table verdict
local function scenario_ambiguous(handles)
    local hreg = handles.hregistry
    local ok, err = hreg.register_tool(
        handles.reg_state,
        'summarize',
        { description = 'first registration' }
    )
    if not ok then
        return fail('ambiguous', 'first register_tool failed: ' .. tostring(err))
    end
    local ok2, err2 = hreg.register_tool(
        handles.reg_state,
        'summarize',
        { description = 'second registration' }
    )
    if ok2 then
        return fail('ambiguous', 'duplicate bare-name registration was accepted')
    end
    ev('duplicate bare name rejected: ' .. tostring(err2))
    if hreg.get_tool(handles.reg_state, 'mcp:summarize') ~= nil then
        return fail('ambiguous', "get_tool resolved 'mcp:summarize'")
    end
    ev("get_tool('mcp:summarize') -> nil: namespaced lookup is inexpressible")
    local trace = {
        scenario = 'ambiguous',
        bare_duplicate_rejected = true,
        ambiguous_rejected = 'n/a — no namespace machinery; cross-protocol collision inexpressible',
    }
    local werr = write_work_file(handles, 'dispatch-trace.json', vim.json.encode(trace))
    if werr ~= nil then
        return fail('trace', werr)
    end
    return pass()
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'distinct'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return fail('bootstrap', boot_err)
    end
    ev('scenario=' .. scenario)
    if scenario == 'distinct' then
        return scenario_distinct(handles)
    elseif scenario == 'no-namespace' then
        return scenario_no_namespace(handles)
    elseif scenario == 'spoof' then
        return scenario_spoof(handles)
    elseif scenario == 'ambiguous' then
        return scenario_ambiguous(handles)
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
