-- task-08 driver: MCP stdio tool bridging for diver's ai.harness.
--
-- Bridges the harness's REAL MCP adapter (ai.harness.adapters.mcp) to an
-- MCP stdio server and proves the list -> call round trip end to end:
-- harness.run({ adapter = 'mcp', extensions = { mcp = { server = name } } })
-- starts the server session through the real adapter, then the real
-- ai.mcp.tools list/call path drives tools/list and tools/call over the
-- real ai.mcp.client NDJSON JSON-RPC transport.
--
-- Server choice per scenario:
--   default      FIRST probes the REAL phlow MCP server (phlow serve):
--                NDJSON JSON-RPC 2.0 over stdio. The probe is expected to
--                fail the initialize handshake: diver's ai.mcp.client sends
--                "capabilities":[] (empty Lua table -> JSON array) while
--                phlow-mcp strictly requires a capabilities object per the
--                MCP spec, and rejects with -32602. That pinned interop gap
--                is asserted as evidence, then the scenario falls back to
--                the mock server for the tools/list -> tools/call round
--                trip with valid args (gauntlet_add{40,2} -> "42"), whose
--                result flows back into the run as a diagnostic event.
--   bad-args     a purpose-built python3 mock MCP server (written into
--                GAUNTLET_WORK_DIR at bootstrap) with strict schema
--                validation: invalid arguments surface as typed JSON-RPC
--                -32602 errors.
--   server-dies  the mock exits(1) mid-call with no reply: the client's
--                process-exit path must resolve the pending request with a
--                typed error instead of hanging.
--   oversize     the mock answers tools/call with a single ~10 MiB JSON
--                line, past the client's 8 MiB LINE_BYTES_MAX frame cap:
--                the frame must be dropped, the pending request must time
--                out (typed error), and the session must stay usable.
--
-- Tool-call confirmation: ai.mcp.tools.call consults ai.security, which
-- default-denies headless. The driver pre-approves the exact
-- (server, tool) pairs through the real persistent allowlist mechanism
-- (allowlist_add), with XDG_DATA_HOME pointed at GAUNTLET_WORK_DIR by the
-- Rust runner so nothing is written outside the work dir.
--
-- Prints exactly one JSON verdict line to stdout and always exits 0; the
-- verdict carries the outcome, not the exit code. Writes nothing outside
-- GAUNTLET_WORK_DIR. Never modifies the diver repo.

local TASK_ID = 'task-08'
local SERVER_REAL = 'gauntlet-mcp-real'
local SERVER_MOCK = 'gauntlet-mcp-mock'
local MOCK_SCRIPT = 'gauntlet_mock_mcp.py'
local PYTHON_BIN = '/usr/bin/python3'

local EVIDENCE_MAX = 64
local WAIT_POLL_MS = 25
local RUN_TIMEOUT_MS = 60000
local COMPLETE_WAIT_MS = 30000
local CALL_WAIT_MS = 20000
local OVERSIZE_TIMEOUT_MS = 4000
local OVERSIZE_ROUNDS = 2
local OVERSIZE_GROWTH_KB_MAX = 32 * 1024

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

---Mock MCP stdio server (python3). Speaks newline-delimited JSON-RPC 2.0,
---the framing ai.mcp.client expects. GAUNTLET_MCP_MOCK steers tools/call:
---'bad-args' validates strictly, 'server-dies' exits(1) mid-call,
---'oversize' answers gauntlet_blob with one ~10 MiB JSON line.
local MOCK_SOURCE = [==[
#!/usr/bin/env python3
"""Gauntlet mock MCP stdio server for task-08 (written by the driver)."""
import json
import os
import sys

SCENARIO = os.environ.get("GAUNTLET_MCP_MOCK", "bad-args")
OVERSIZE_BYTES = 10 * 1024 * 1024

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
        "name": "gauntlet_add",
        "description": "Add two integers and return the sum",
        "inputSchema": {
            "type": "object",
            "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}},
            "required": ["a", "b"],
            "additionalProperties": False,
        },
    },
    {
        "name": "gauntlet_blob",
        "description": "Return a large test payload for bound checks",
        "inputSchema": {
            "type": "object",
            "properties": {},
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


def validate(tool, args):
    if not isinstance(args, dict):
        return "arguments must be an object"
    schema = tool["inputSchema"]
    props = schema["properties"]
    for key in args:
        if key not in props:
            return "unknown argument: %r" % (key,)
    for key in schema.get("required", []):
        if key not in args:
            return "missing required argument: %r" % (key,)
    for key, value in args.items():
        want = props[key]["type"]
        if want == "string" and not isinstance(value, str):
            return "argument %r must be a string" % (key,)
        if want == "integer" and not isinstance(value, int):
            return "argument %r must be an integer" % (key,)
    return None


def tool_text(text):
    return {"content": [{"type": "text", "text": text}], "isError": False}


def handle_call(mid, params):
    name = params.get("name")
    args = params.get("arguments", {})
    tool = next((t for t in TOOLS if t["name"] == name), None)
    if tool is None:
        error(mid, -32602, "Unknown tool: %r" % (name,))
        return
    problem = validate(tool, args)
    if problem is not None:
        error(mid, -32602, "Invalid params: " + problem)
        return
    if SCENARIO == "server-dies":
        os._exit(1)
    if SCENARIO == "oversize" and name == "gauntlet_blob":
        result(mid, tool_text("x" * OVERSIZE_BYTES))
        return
    if name == "gauntlet_echo":
        result(mid, tool_text(args["text"]))
    elif name == "gauntlet_add":
        result(mid, tool_text(str(args["a"] + args["b"])))
    else:
        result(mid, tool_text("small"))


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
        if method == "initialize":
            result(mid, {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "gauntlet-mock-mcp", "version": "0.1.0"},
            })
        elif method == "notifications/initialized":
            pass
        elif method == "tools/list":
            result(mid, {"tools": TOOLS})
        elif method == "tools/call":
            params = msg.get("params")
            if not isinstance(params, dict):
                error(mid, -32602, "Invalid params: params must be an object")
            else:
                handle_call(mid, params)
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

---Wire up the harness, the MCP registry (redirected into the work dir),
---the mock server file, and the security allowlist for the exact
---(server, tool) pairs this driver calls.
---@return table? handles
---@return string? err
local function bootstrap()
    local diver_lua_dir = vim.env.DIVER_LUA_DIR
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    local phlow_bin = vim.env.GAUNTLET_PHLOW_BIN
    for _, pair in ipairs({
        { 'DIVER_LUA_DIR', diver_lua_dir },
        { 'GAUNTLET_WORK_DIR', work_dir },
        { 'GAUNTLET_PHLOW_BIN', phlow_bin },
    }) do
        if type(pair[2]) ~= 'string' or pair[2] == '' then
            return nil, pair[1] .. ' is not set'
        end
    end
    assert(diver_lua_dir ~= nil and work_dir ~= nil and phlow_bin ~= nil)
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
    local security = require('ai.security')
    local approvals = {
        { SERVER_REAL, 'flow_status' },
        { SERVER_MOCK, 'gauntlet_echo' },
        { SERVER_MOCK, 'gauntlet_add' },
        { SERVER_MOCK, 'gauntlet_blob' },
        -- 'no_such_tool' is allowlisted so the bad-args scenario can prove
        -- the SERVER rejects unknown tools (the gate must not stop it).
        { SERVER_MOCK, 'no_such_tool' },
    }
    for _, pair in ipairs(approvals) do
        local allow_ok, allow_err = security.allowlist_add(pair[1], pair[2])
        if not allow_ok then
            return nil, 'allowlist_add failed: ' .. tostring(allow_err)
        end
    end
    ev('allowlisted ' .. #approvals .. ' (server, tool) pairs')
    return {
        harness = harness,
        sup = st.supervisor,
        sink = st.sink,
        supervisor = require('ai.harness.supervisor'),
        types = require('ai.harness.types'),
        client = require('ai.mcp.client'),
        mcp_tools = require('ai.mcp.tools'),
        registry = registry,
        work_dir = work_dir,
        phlow_bin = phlow_bin,
        mock_path = mock_path,
    }
end

---NOTE (diver bug, read-only here): ai.mcp.client's spawn_native builds
---the registry entry's `env` as a LIST of "K=V" strings, but vim.system's
---`env` option takes a DICT ({ K = V }); the list form is silently
---ignored, so per-server registry env never reaches the child. Verified
---2026-09-28 against nvim nightly: list form -> child sees ABSENT, dict
---form -> child sees the value. The driver therefore steers the mock via
---vim.env (inherited by spawned children) instead of the registry entry.
---@param handles table
---@param name string
---@param command string
---@param args string[]
---@return string? err
local function register_server(handles, name, command, args)
    local ok, err = handles.registry.add({
        name = name,
        command = command,
        args = args,
        enabled = true,
    })
    if not ok then
        return 'registry.add failed: ' .. tostring(err)
    end
    ev('registered MCP server: ' .. name)
    return nil
end

---@param handles table
---@param run_id string
---@return string state
local function run_state(handles, run_id)
    local run = handles.supervisor.get(handles.sup, run_id)
    if run == nil then
        return 'missing'
    end
    return run.state
end

---Drive the supervisor clock until the run reaches `want` or the deadline.
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
        local state = run_state(handles, run_id)
        if state == want then
            return true, state
        end
        if vim.uv.hrtime() >= deadline then
            return false, state
        end
        vim.wait(WAIT_POLL_MS)
    end
end

---@param handles table
---@param server_name string
---@return string? run_id
---@return string? err
local function start_mcp_run(handles, server_name)
    local spec = {
        workflow = 'gauntlet-mcp-bridge',
        goal = 'MCP stdio tool bridging probe for ' .. server_name,
        workspace = handles.work_dir,
        adapter = 'mcp',
        timeout_ms = RUN_TIMEOUT_MS,
        extensions = { mcp = { server = server_name } },
    }
    return handles.harness.run(spec)
end

---Start the run through the real MCP adapter and wait for completion.
---The adapter's on_ready reports model.completed, which the supervisor
---turns into the terminal state; the session is then closed.
---@param handles table
---@param server_name string
---@return string? run_id
---@return string? fail_where
---@return string? fail_how
local function run_adapter_session(handles, server_name)
    local run_id, run_err = start_mcp_run(handles, server_name)
    if run_id == nil then
        return nil, 'run-start', 'harness.run failed: ' .. tostring(run_err)
    end
    ev('run_id=' .. run_id .. ' server=' .. server_name)
    local reached, state = wait_state(handles, run_id, 'completed', COMPLETE_WAIT_MS)
    if not reached then
        return nil, 'run-start', 'run never completed, state=' .. state
    end
    local completed_seen = false
    for _, event in ipairs(handles.sink:events(run_id)) do
        if event.kind == 'model.completed' and event.payload.outcome == 'completed' then
            completed_seen = true
        end
    end
    if not completed_seen then
        return nil, 'run-start', 'no model.completed event with outcome=completed'
    end
    ev('adapter session completed: model.completed outcome=completed')
    return run_id, nil, nil
end

---Synchronous wrapper over the callback-style ai.mcp.tools.list.
---@param handles table
---@param server string
---@param timeout_ms integer
---@return table? tools
---@return string? err
local function tools_list_sync(handles, server, timeout_ms)
    local done = false
    local err_out = nil
    local tools_out = nil
    handles.mcp_tools.list(server, function(err, tools)
        err_out = err
        tools_out = tools
        done = true
    end)
    local deadline = vim.uv.hrtime() + (timeout_ms * 1000000)
    while not done do
        if vim.uv.hrtime() >= deadline then
            return nil, 'tools/list wait timed out'
        end
        vim.wait(WAIT_POLL_MS)
    end
    return tools_out, err_out
end

---Synchronous wrapper over the callback-style ai.mcp.tools.call.
---@param handles table
---@param server string
---@param tool string
---@param args table
---@param timeout_ms integer
---@return table? result
---@return string? err
local function tools_call_sync(handles, server, tool, args, timeout_ms)
    local done = false
    local err_out = nil
    local result_out = nil
    handles.mcp_tools.call(server, tool, args, function(err, result)
        err_out = err
        result_out = result
        done = true
    end)
    local deadline = vim.uv.hrtime() + (timeout_ms * 1000000)
    while not done do
        if vim.uv.hrtime() >= deadline then
            return nil, 'tools/call wait timed out'
        end
        vim.wait(WAIT_POLL_MS)
    end
    return result_out, err_out
end

---Synchronous wrapper over ai.mcp.client.request with an explicit timeout.
---Used where ai.mcp.tools.call's fixed 60 s timeout would only slow the
---test; the pending-request timeout mechanism exercised is identical.
---@param handles table
---@param server string
---@param method string
---@param params table
---@param timeout_ms integer
---@param wait_ms integer
---@return table? result
---@return string? err
local function client_request_sync(handles, server, method, params, timeout_ms, wait_ms)
    local done = false
    local err_out = nil
    local result_out = nil
    handles.client.request(server, method, params, function(err, result)
        err_out = err
        result_out = result
        done = true
    end, timeout_ms)
    local deadline = vim.uv.hrtime() + (wait_ms * 1000000)
    while not done do
        if vim.uv.hrtime() >= deadline then
            return nil, 'client.request wait timed out'
        end
        vim.wait(WAIT_POLL_MS)
    end
    return result_out, err_out
end

---@param tools table
---@return string names comma-separated, sorted
local function tool_names(tools)
    local names = {}
    for _, tool in ipairs(tools) do
        names[#names + 1] = tool.name
    end
    table.sort(names)
    return table.concat(names, ',')
end

---Steer the mock for this scenario. Goes through vim.env (inherited by
---spawned children) because the registry entry's env is silently dropped
---by ai.mcp.client (see register_server note).
---@param scenario string one of 'default', 'bad-args', 'server-dies', 'oversize'
local function steer_mock(scenario)
    vim.env.GAUNTLET_MCP_MOCK = scenario
    ev('mock steered via process env: GAUNTLET_MCP_MOCK=' .. scenario)
end

---Empty JSON object for tool arguments. A bare {} would encode as [] and
---trip strict servers (same empty-table gotcha as the capabilities bug).
---@return table
local function empty_args()
    return vim.empty_dict()
end

---Default: probe the real phlow server (pins the known initialize
---handshake interop gap as evidence), then run the list -> call round
---trip against the mock server and record the result into the run.
local function scenario_default(handles)
    local reg_err = register_server(handles, SERVER_REAL, handles.phlow_bin, { 'serve' })
    if reg_err ~= nil then
        return fail('register', reg_err)
    end
    -- Step 1: the real-server probe. diver's ai.mcp.client encodes the
    -- empty capabilities table as a JSON array ("capabilities":[]);
    -- phlow-mcp requires a capabilities object (MCP spec) and rejects the
    -- handshake with -32602. The adapter surfaces that as a failed run
    -- with the typed error in the model.completed payload.
    local real_run_id, real_run_err = start_mcp_run(handles, SERVER_REAL)
    if real_run_id == nil then
        return fail('run-start', 'harness.run failed: ' .. tostring(real_run_err))
    end
    local reached, state = wait_state(handles, real_run_id, 'failed', COMPLETE_WAIT_MS)
    if not reached then
        return fail(
            'real-server-probe',
            'expected the real-server run to fail the handshake, state=' .. state
        )
    end
    local probe_err = nil
    for _, event in ipairs(handles.sink:events(real_run_id)) do
        if event.kind == 'model.completed' and event.payload.outcome == 'failed' then
            probe_err = event.payload.error
        end
    end
    ev('real-server probe: run failed as expected, error=' .. tostring(probe_err))
    if probe_err == nil or probe_err:find('Initialize requires capabilities and clientInfo', 1, true) == nil then
        return fail(
            'real-server-probe',
            'expected the capabilities-object rejection, got: ' .. tostring(probe_err)
        )
    end
    ev('interop gap pinned: diver sends "capabilities":[] (empty Lua table -> JSON array);'
        .. ' phlow-mcp requires an object per MCP spec (server.rs initialize)')
    -- Step 2: the round trip, against the mock (behavior-neutral here).
    steer_mock('default')
    reg_err = register_server(handles, SERVER_MOCK, PYTHON_BIN, { handles.mock_path })
    if reg_err ~= nil then
        return fail('register', reg_err)
    end
    local run_id, where, how = run_adapter_session(handles, SERVER_MOCK)
    if run_id == nil then
        return fail(where, how)
    end
    local tools, list_err = tools_list_sync(handles, SERVER_MOCK, CALL_WAIT_MS)
    if tools == nil then
        return fail('tools-list', 'tools/list failed: ' .. tostring(list_err))
    end
    ev('tools/list: count=' .. #tools .. ' names=' .. tool_names(tools))
    if tool_names(tools) ~= 'gauntlet_add,gauntlet_blob,gauntlet_echo' then
        return fail('tools-list', 'unexpected tool names: ' .. tool_names(tools))
    end
    local result, call_err =
        tools_call_sync(handles, SERVER_MOCK, 'gauntlet_add', { a = 40, b = 2 }, CALL_WAIT_MS)
    if result == nil then
        return fail('tools-call', 'tools/call gauntlet_add failed: ' .. tostring(call_err))
    end
    local content = result.content
    if type(content) ~= 'table' or type(content[1]) ~= 'table' or content[1].text ~= '42' then
        return fail('tools-call', 'gauntlet_add{40,2} did not return "42"')
    end
    ev('tools/call gauntlet_add{a=40,b=2} -> "42": round trip through the real adapter')
    handles.sink:append(run_id, 'diagnostic.observed', {
        kind = 'gauntlet_mcp_roundtrip',
        server = SERVER_MOCK,
        tool = 'gauntlet_add',
        result_text = content[1].text,
    }, { source = 'gauntlet' })
    ev('tool result recorded into run ' .. run_id .. ' as diagnostic.observed')
    handles.client.stop(SERVER_MOCK)
    ev('session stopped cleanly')
    return pass()
end

---bad-args: invalid arguments surface as typed JSON-RPC errors; the run
---and the driver survive them.
local function scenario_bad_args(handles)
    steer_mock('bad-args')
    local reg_err = register_server(handles, SERVER_MOCK, PYTHON_BIN, { handles.mock_path })
    if reg_err ~= nil then
        return fail('register', reg_err)
    end
    local run_id, where, how = run_adapter_session(handles, SERVER_MOCK)
    if run_id == nil then
        return fail(where, how)
    end
    local tools, list_err = tools_list_sync(handles, SERVER_MOCK, CALL_WAIT_MS)
    if tools == nil then
        return fail('tools-list', 'tools/list failed: ' .. tostring(list_err))
    end
    ev('tools/list: count=' .. #tools .. ' names=' .. tool_names(tools))
    local _, missing_err =
        tools_call_sync(handles, SERVER_MOCK, 'gauntlet_echo', empty_args(), CALL_WAIT_MS)
    if missing_err == nil or missing_err:find('missing required argument', 1, true) == nil then
        return fail('bad-args', 'missing-arg call did not yield the typed error: ' .. tostring(missing_err))
    end
    ev('gauntlet_echo{} -> typed error: ' .. missing_err)
    local _, type_err =
        tools_call_sync(handles, SERVER_MOCK, 'gauntlet_add', { a = 'nope', b = 2 }, CALL_WAIT_MS)
    if type_err == nil or type_err:find('must be an integer', 1, true) == nil then
        return fail('bad-args', 'wrong-type call did not yield the typed error: ' .. tostring(type_err))
    end
    ev('gauntlet_add{a=string} -> typed error: ' .. type_err)
    local _, unknown_err =
        tools_call_sync(handles, SERVER_MOCK, 'no_such_tool', empty_args(), CALL_WAIT_MS)
    if unknown_err == nil or unknown_err:find('Unknown tool', 1, true) == nil then
        return fail('bad-args', 'unknown-tool call did not yield Unknown tool: ' .. tostring(unknown_err))
    end
    ev('no_such_tool -> typed error: ' .. unknown_err)
    if run_state(handles, run_id) ~= 'completed' then
        return fail('bad-args', 'run left its terminal state by the error path')
    end
    ev('run still completed after three typed errors: no crash')
    handles.client.stop(SERVER_MOCK)
    return pass()
end

---server-dies: the server exits mid-call; the pending request must resolve
---with the typed process-exit error, not hang.
local function scenario_server_dies(handles)
    steer_mock('server-dies')
    local reg_err = register_server(handles, SERVER_MOCK, PYTHON_BIN, { handles.mock_path })
    if reg_err ~= nil then
        return fail('register', reg_err)
    end
    local run_id, where, how = run_adapter_session(handles, SERVER_MOCK)
    if run_id == nil then
        return fail(where, how)
    end
    local tools, list_err = tools_list_sync(handles, SERVER_MOCK, CALL_WAIT_MS)
    if tools == nil then
        return fail('tools-list', 'tools/list failed: ' .. tostring(list_err))
    end
    ev('tools/list ok: session alive before the fatal call')
    local started = vim.uv.hrtime()
    local _, call_err =
        tools_call_sync(handles, SERVER_MOCK, 'gauntlet_echo', { text = 'hello' }, CALL_WAIT_MS)
    local elapsed_ms = math.floor((vim.uv.hrtime() - started) / 1000000)
    if call_err == nil or call_err:find('server exited', 1, true) == nil then
        return fail('server-dies', 'mid-call exit did not yield a server-exited error: ' .. tostring(call_err))
    end
    ev('mid-call exit -> typed error: ' .. call_err .. ' (after ' .. elapsed_ms .. ' ms, no hang)')
    if run_state(handles, run_id) ~= 'completed' then
        return fail('server-dies', 'run left its terminal state on server death')
    end
    ev('run still completed: server death did not wedge the harness')
    handles.client.stop(SERVER_MOCK)
    return pass()
end

---oversize: a ~10 MiB single-line response exceeds the client's 8 MiB
---LINE_BYTES_MAX frame cap. The frame must be dropped, the pending request
---must time out with a typed error, memory growth must stay bounded, and
---the session must remain usable afterwards.
local function scenario_oversize(handles)
    steer_mock('oversize')
    local reg_err = register_server(handles, SERVER_MOCK, PYTHON_BIN, { handles.mock_path })
    if reg_err ~= nil then
        return fail('register', reg_err)
    end
    local _, where, how = run_adapter_session(handles, SERVER_MOCK)
    if where ~= nil then
        return fail(where, how)
    end
    local tools, list_err = tools_list_sync(handles, SERVER_MOCK, CALL_WAIT_MS)
    if tools == nil then
        return fail('tools-list', 'tools/list failed: ' .. tostring(list_err))
    end
    collectgarbage('collect')
    local mem_before_kb = collectgarbage('count')
    for round = 1, OVERSIZE_ROUNDS do
        local _, call_err = client_request_sync(
            handles,
            SERVER_MOCK,
            'tools/call',
            { name = 'gauntlet_blob', arguments = empty_args() },
            OVERSIZE_TIMEOUT_MS,
            CALL_WAIT_MS
        )
        if call_err == nil or call_err:find('timed out', 1, true) == nil then
            return fail(
                'oversize',
                'round ' .. round .. ': oversize frame did not time out: ' .. tostring(call_err)
            )
        end
        ev('round ' .. round .. ': 10 MiB frame dropped -> typed error: ' .. call_err)
    end
    collectgarbage('collect')
    local mem_after_kb = collectgarbage('count')
    local growth_kb = mem_after_kb - mem_before_kb
    ev(('lua memory: before=%.0f KB after=%.0f KB growth=%.0f KB (cap %d KB)'):format(
        mem_before_kb,
        mem_after_kb,
        growth_kb,
        OVERSIZE_GROWTH_KB_MAX
    ))
    if growth_kb > OVERSIZE_GROWTH_KB_MAX then
        return fail('oversize', 'memory grew without bound: ' .. math.floor(growth_kb) .. ' KB')
    end
    local tools2, list_err2 = tools_list_sync(handles, SERVER_MOCK, CALL_WAIT_MS)
    if tools2 == nil then
        return fail('oversize', 'session unusable after oversize frames: ' .. tostring(list_err2))
    end
    ev('tools/list still ok after oversize rounds: session survived, buffer reset')
    handles.client.stop(SERVER_MOCK)
    return pass()
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'default'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return fail('bootstrap', boot_err)
    end
    ev('scenario=' .. scenario)
    if scenario == 'default' then
        return scenario_default(handles)
    elseif scenario == 'bad-args' then
        return scenario_bad_args(handles)
    elseif scenario == 'server-dies' then
        return scenario_server_dies(handles)
    elseif scenario == 'oversize' then
        return scenario_oversize(handles)
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
