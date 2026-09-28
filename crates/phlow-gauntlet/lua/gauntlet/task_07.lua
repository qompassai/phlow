-- task-07 driver: A2A task lifecycle through diver's REAL a2a harness adapter.
--
-- Proves the full A2A v0.3.x JSON-RPC lifecycle works end to end:
--   lua/ai/harness/adapters/a2a.lua
--     -> lua/ai/a2a/tasks.lua   (task supervisor: states, generations, timeouts)
--     -> lua/ai/a2a/client.lua  (JSON-RPC over curl, loopback http allowed)
-- against a mock A2A peer implemented below on vim.uv TCP. No extra
-- dependencies, no files outside GAUNTLET_WORK_DIR, diver repo untouched.
--
-- Wire facts verified against the adapter source (not assumed):
-- * streaming card  -> POST {"method":"message/stream"} with
--   params.message = {kind="message", role="user" (lowercase),
--   parts={{kind="text", text=<goal>}}}; progress arrives as SSE events
--   {kind="status-update", taskId, status={state=<kebab-case>}}.
-- * non-streaming   -> POST {"method":"message/send"}, same message shape.
-- * cancel          -> POST {"method":"tasks/cancel", params={id=<remote id>}}.
-- * ai.a2a.client implements tasks/get, but NOTHING in diver calls it
--   (verified by source grep): the supervisor never polls. The mock
--   implements it anyway for fidelity; no tasks/get appears on the wire.
-- * Agent card is fetched from /.well-known/agent-card.json and validated
--   (name, version, loopback endpoint URL) before the run starts.
-- * Unknown/foreign task states are IGNORED by tasks.set_state by design
--   ("a stray state must never corrupt the supervisor") — the bad-state
--   scenario proves the run still completes uncorrupted.
--
-- Scenarios via GAUNTLET_SCENARIO (default "default"):
--   default    message/stream -> working -> completed; wire shapes verified,
--              kebab-case states quoted, verdict recorded.
--   cancel     cancel mid-stream -> tasks/cancel hits the peer -> run
--              cancelled; no stale completion leaks through.
--   peer-dies  peer destroys the connection mid-stream -> run fails fast,
--              no hang (deadline-guarded).
--   bad-state  peer sends unknown state "frobnicate" mid-stream -> ignored
--              locally, run still completes.
--
-- Prints exactly one JSON verdict line to stdout and always exits 0; the
-- verdict carries the outcome, not the exit code.
--
-- KNOWN DIVER BUG FOUND BY THIS DRIVER (lua/ai/harness @ c84352c):
-- supervisor.create validates spec.goal but never copies it onto the run
-- table, so run.goal is nil for every adapter that reads it (a2a, acp,
-- herd, rose). The driver repairs the dropped field between create and
-- start_run (the two calls harness.run makes); the adapter code itself is
-- unmodified and fully exercised. Full analysis in docs/task-07.md.

local TASK_ID = 'task-07'
local PEER_TASK_ID = 'mock-task-1'
local GOAL_TEXT = 'gauntlet a2a lifecycle probe'
local EVIDENCE_MAX = 64
local WAIT_POLL_MS = 25
local RUN_TIMEOUT_MS = 90000
local WAIT_TIMEOUT_MS = 30000
local A2A_TIMEOUT_MS = 30000
local CARD_FETCH_TIMEOUT_MS = 10000
local CANCEL_DELAY_MS = 600
local PEER_DIE_DELAY_MS = 300
local STREAM_CLOSE_DELAY_MS = 150

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

-- ---------------------------------------------------------------------------
-- Mock A2A peer (vim.uv TCP, HTTP/1.1 + JSON-RPC + SSE).
-- ---------------------------------------------------------------------------

---@param peer table
---@param line string
local function plog(peer, line)
    peer.log[#peer.log + 1] = tostring(line)
end

---@param client userdata
---@param body string
---@param content_type string
local function respond_and_close(client, body, content_type)
    local head = 'HTTP/1.1 200 OK\r\nContent-Type: ' .. content_type
        .. '\r\nContent-Length: ' .. #body .. '\r\nConnection: close\r\n\r\n'
    client:write(head .. body, function()
        client:close()
    end)
end

---@param client userdata
---@param status string
---@param text string
local function respond_text(client, status, text)
    local head = 'HTTP/1.1 ' .. status .. '\r\nContent-Type: text/plain'
        .. '\r\nContent-Length: ' .. #text .. '\r\nConnection: close\r\n\r\n'
    client:write(head .. text, function()
        client:close()
    end)
end

---@param client userdata
---@param id any
---@param result table
local function jsonrpc_result(client, id, result)
    respond_and_close(client, vim.json.encode({ jsonrpc = '2.0', id = id, result = result }))
end

---@param client userdata
---@param id any
---@param code integer
---@param message string
local function jsonrpc_error(client, id, code, message)
    respond_and_close(
        client,
        vim.json.encode({ jsonrpc = '2.0', id = id, error = { code = code, message = message } })
    )
end

---@param client userdata
local function sse_begin(client)
    local head = 'HTTP/1.1 200 OK\r\nContent-Type: text/event-stream'
        .. '\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n'
    client:write(head)
end

---Begin an SSE stream that will die truncated: declare far more bytes than
---will ever arrive, so the peer's mid-stream close surfaces to curl as a
---transfer error (exit 18) rather than a clean EOF. A clean FIN would make
---ai.a2a.tasks treat the stream as "closed without a terminal state" and
---optimistically complete the task; the truncated body forces the failure
---path the peer-dies scenario must exercise.
---@param client userdata
local function sse_begin_truncated(client)
    local head = 'HTTP/1.1 200 OK\r\nContent-Type: text/event-stream'
        .. '\r\nCache-Control: no-cache\r\nContent-Length: 1048576\r\nConnection: close\r\n\r\n'
    client:write(head)
end

---@param client userdata
---@param payload table
local function sse_send(client, payload)
    client:write('data: ' .. vim.json.encode(payload) .. '\n\n')
end

---@param client userdata
---@param delay_ms integer
local function close_soon(client, delay_ms)
    vim.defer_fn(function()
        if not client:is_closing() then
            client:close()
        end
    end, delay_ms)
end

---Verify the exact wire shape ai.a2a.client sends: lowercase role "user",
---kind discriminators on the message and its text part, goal text intact.
---@param params any
---@param goal string
---@return boolean ok
---@return string? err
local function check_message_shape(params, goal)
    if type(params) ~= 'table' then
        return false, 'params not a table'
    end
    local msg = params.message
    if type(msg) ~= 'table' then
        return false, 'params.message missing'
    end
    if msg.role ~= 'user' then
        return false, 'role=' .. tostring(msg.role)
    end
    if msg.kind ~= 'message' then
        return false, 'kind=' .. tostring(msg.kind)
    end
    local parts = msg.parts
    if type(parts) ~= 'table' or type(parts[1]) ~= 'table' then
        return false, 'parts missing'
    end
    if parts[1].kind ~= 'text' then
        return false, 'part kind=' .. tostring(parts[1].kind)
    end
    if parts[1].text ~= goal then
        return false, 'goal text mismatch'
    end
    return true
end

---@param state string
---@return table SSE status-update event with a kebab-case state.
local function status_event(state)
    return {
        kind = 'status-update',
        taskId = PEER_TASK_ID,
        status = { state = state },
        final = state == 'completed',
    }
end

---@return table SSE artifact-update event.
local function artifact_event()
    return {
        kind = 'artifact-update',
        taskId = PEER_TASK_ID,
        artifact = {
            artifactId = 'mock-artifact-1',
            name = 'verdict',
            parts = { { kind = 'text', text = 'lifecycle-ok' } },
        },
    }
end

---@param peer table
---@param client userdata
local function peer_die(peer)
    plog(peer, 'peer-dies: destroyed connection mid-stream')
    if peer.stream_client ~= nil and not peer.stream_client:is_closing() then
        peer.stream_client:close()
        peer.stream_client = nil
    end
    if peer.server ~= nil and not peer.server:is_closing() then
        peer.server:close()
        peer.server = nil
    end
end

---@param peer table
---@param client userdata
---@param doc table Decoded JSON-RPC request.
local function handle_message_stream(peer, client, doc)
    local shape_ok, shape_err = check_message_shape(doc.params, peer.goal_text)
    if not shape_ok then
        jsonrpc_error(client, doc.id, -32602, 'bad message shape: ' .. tostring(shape_err))
        return
    end
    plog(peer, 'wire-shape-ok role=user kind=message part-kind=text goal-match')
    peer.task_state = 'working'
    peer.stream_client = client
    if peer.mode == 'default' then
        sse_begin(client)
        sse_send(client, status_event('working'))
        sse_send(client, status_event('completed'))
        sse_send(client, artifact_event())
        plog(peer, 'sse: working -> completed (kebab-case states on the wire)')
        close_soon(client, STREAM_CLOSE_DELAY_MS)
    elseif peer.mode == 'cancel' then
        sse_begin(client)
        sse_send(client, status_event('working'))
        plog(peer, 'sse: working (holding stream open for cancel)')
        -- Hold the stream: the driver cancels mid-flight, which must make
        -- the adapter POST tasks/cancel and settle the run as cancelled.
    elseif peer.mode == 'peer-dies' then
        sse_begin_truncated(client)
        sse_send(client, status_event('working'))
        plog(peer, 'sse: working (truncated stream: peer dies in ' .. PEER_DIE_DELAY_MS .. 'ms)')
        vim.defer_fn(function()
            peer_die(peer)
        end, PEER_DIE_DELAY_MS)
    elseif peer.mode == 'bad-state' then
        sse_begin(client)
        sse_send(client, status_event('working'))
        sse_send(client, status_event('frobnicate'))
        sse_send(client, status_event('completed'))
        plog(peer, 'sse: working -> frobnicate(unknown) -> completed')
        close_soon(client, STREAM_CLOSE_DELAY_MS)
    else
        jsonrpc_error(client, doc.id, -32601, 'unknown peer mode')
    end
end

---@param peer table
---@param client userdata
---@param doc table Decoded JSON-RPC request.
local function handle_message_send(peer, client, doc)
    local shape_ok, shape_err = check_message_shape(doc.params, peer.goal_text)
    if not shape_ok then
        jsonrpc_error(client, doc.id, -32602, 'bad message shape: ' .. tostring(shape_err))
        return
    end
    plog(peer, 'wire-shape-ok role=user kind=message part-kind=text goal-match')
    peer.task_state = 'completed'
    jsonrpc_result(client, doc.id, {
        taskId = PEER_TASK_ID,
        status = { state = 'completed' },
        artifacts = {
            {
                artifactId = 'mock-artifact-1',
                parts = { { kind = 'text', text = 'lifecycle-ok' } },
            },
        },
    })
end

---@param peer table
---@return table Agent card served at /.well-known/agent-card.json.
local function peer_card(peer)
    return {
        name = 'gauntlet-mock-agent',
        version = '0.0.1',
        url = 'http://127.0.0.1:' .. peer.port,
        description = 'gauntlet mock A2A peer',
        capabilities = { streaming = true },
        skills = {},
    }
end

---@param peer table
---@param client userdata
---@param state table Per-connection parse state.
---@param body string Request body.
local function route_request(peer, client, state, body)
    if state.method == 'GET' and state.path == '/.well-known/agent-card.json' then
        plog(peer, 'served agent card at /.well-known/agent-card.json')
        respond_and_close(client, vim.json.encode(peer_card(peer)), 'application/json')
        return
    end
    if state.method ~= 'POST' then
        respond_text(client, '404 Not Found', 'not found')
        return
    end
    local ok, doc = pcall(vim.json.decode, body)
    if not ok or type(doc) ~= 'table' then
        respond_text(client, '400 Bad Request', 'bad json')
        return
    end
    plog(peer, 'rpc method=' .. tostring(doc.method) .. ' id=' .. tostring(doc.id))
    local method = doc.method
    if method == 'message/stream' then
        handle_message_stream(peer, client, doc)
    elseif method == 'message/send' then
        handle_message_send(peer, client, doc)
    elseif method == 'tasks/get' then
        jsonrpc_result(client, doc.id, {
            taskId = PEER_TASK_ID,
            status = { state = peer.task_state },
        })
    elseif method == 'tasks/cancel' then
        plog(peer, 'rpc tasks/cancel received for id=' .. tostring(doc.params and doc.params.id))
        peer.task_state = 'canceled'
        jsonrpc_result(client, doc.id, {
            taskId = PEER_TASK_ID,
            status = { state = 'canceled' },
            final = true,
        })
    else
        jsonrpc_error(client, doc.id, -32601, 'method not found: ' .. tostring(method))
    end
end

---@param peer table
---@param client userdata
---@param state table Per-connection parse state.
---@param rerr any
---@param data string?
local function on_client_data(peer, client, state, rerr, data)
    if rerr then
        client:close()
        return
    end
    if data == nil then
        -- EOF: curl went away (e.g. teardown killed it after cancel).
        client:close()
        return
    end
    if state.streaming then
        return
    end
    state.buf = state.buf .. data
    if not state.headers_done then
        local eoh = state.buf:find('\r\n\r\n', 1, true)
        if not eoh then
            return
        end
        local head = state.buf:sub(1, eoh - 1)
        state.buf = state.buf:sub(eoh + 4)
        local request_line = head:match('^([^\r\n]*)') or ''
        state.method, state.path = request_line:match('^(%S+)%s+(%S+)')
        state.method = state.method or ''
        state.path = state.path or ''
        for name, value in head:gmatch('\r\n([^:%s]+):%s*([^\r\n]*)') do
            if name:lower() == 'content-length' then
                state.content_length = tonumber(value) or 0
            end
        end
        state.headers_done = true
    end
    if #state.buf >= state.content_length then
        local body = state.buf:sub(1, state.content_length)
        state.buf = ''
        state.streaming = true -- one request per connection; SSE holds it
        route_request(peer, client, state, body)
    end
end

---@param peer table
---@param listen_err any
local function on_listen(peer, listen_err)
    if listen_err then
        plog(peer, 'listen error: ' .. tostring(listen_err))
        return
    end
    local client = vim.uv.new_tcp()
    if client == nil then
        return
    end
    peer.server:accept(client)
    local state = { buf = '', headers_done = false, content_length = 0, streaming = false }
    client:read_start(function(rerr, data)
        on_client_data(peer, client, state, rerr, data)
    end)
end

---@param mode string
---@param goal_text string
---@return table? peer
---@return string? err
local function peer_start(mode, goal_text)
    local peer = {
        mode = mode,
        goal_text = goal_text,
        server = nil,
        port = nil,
        log = {},
        task_state = 'submitted',
        stream_client = nil,
    }
    local server = vim.uv.new_tcp()
    if server == nil then
        return nil, 'uv tcp handle unavailable'
    end
    peer.server = server
    local bok, berr = pcall(function()
        server:bind('127.0.0.1', 0)
    end)
    if not bok then
        server:close()
        return nil, 'bind failed: ' .. tostring(berr)
    end
    local name = server:getsockname()
    if type(name) ~= 'table' or type(name.port) ~= 'number' then
        server:close()
        return nil, 'getsockname did not return a port'
    end
    peer.port = name.port
    local lok, lerr = pcall(function()
        server:listen(16, function(err)
            on_listen(peer, err)
        end)
    end)
    if not lok then
        server:close()
        return nil, 'listen failed: ' .. tostring(lerr)
    end
    return peer
end

---@param peer table
local function peer_stop(peer)
    if peer.stream_client ~= nil and not peer.stream_client:is_closing() then
        peer.stream_client:close()
        peer.stream_client = nil
    end
    if peer.server ~= nil and not peer.server:is_closing() then
        peer.server:close()
        peer.server = nil
    end
end

-- ---------------------------------------------------------------------------
-- Harness bootstrap and wait helpers.
-- ---------------------------------------------------------------------------

---Wire up the harness. Returns driver handles.
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
    return {
        harness = harness,
        sup = st.supervisor,
        sink = st.sink,
        supervisor = require('ai.harness.supervisor'),
        types = require('ai.harness.types'),
    }
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

---@param peer table
---@param needle string
---@return boolean
local function log_has(peer, needle)
    for _, line in ipairs(peer.log) do
        if line:find(needle, 1, true) then
            return true
        end
    end
    return false
end

---Wait until the mock peer logs a line containing `needle` or the deadline.
---@param peer table
---@param needle string
---@param timeout_ms integer
---@return boolean
local function wait_log(peer, needle, timeout_ms)
    local deadline = vim.uv.hrtime() + (timeout_ms * 1000000)
    while true do
        if log_has(peer, needle) then
            return true
        end
        if vim.uv.hrtime() >= deadline then
            return false
        end
        vim.wait(WAIT_POLL_MS)
    end
end

---Fetch and validate the agent card from the mock peer's well-known URI.
---@param port integer
---@param timeout_ms integer
---@return table? card
---@return string? err
local function fetch_card(port, timeout_ms)
    local agent_card = require('ai.a2a.agent_card')
    local finished = false
    local fetched, fetch_err = nil, nil
    agent_card.fetch('http://127.0.0.1:' .. port, function(ok, card, err)
        finished = true
        fetched = ok and card or nil
        fetch_err = err
    end)
    local deadline = vim.uv.hrtime() + (timeout_ms * 1000000)
    while not finished do
        if vim.uv.hrtime() >= deadline then
            return nil, 'agent card fetch timed out'
        end
        vim.wait(WAIT_POLL_MS)
    end
    if fetched == nil then
        return nil, 'agent card fetch failed: ' .. tostring(fetch_err)
    end
    return fetched
end

---@param handles table
---@param card table
---@return string? run_id
---@return string? err
local function start_run(handles, card)
    local spec = {
        workflow = 'gauntlet-a2a-lifecycle',
        goal = GOAL_TEXT,
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = 'a2a',
        timeout_ms = RUN_TIMEOUT_MS,
        extensions = { a2a = { agent = card, timeout_ms = A2A_TIMEOUT_MS } },
    }
    -- DIVER BUG (lua/ai/harness @ c84352c): supervisor.create validates
    -- spec.goal (types.validate_run_spec requires it) but never copies it
    -- onto the run table, so run.goal is nil when the adapter reads it and
    -- every goal-consuming adapter (a2a, acp, herd, rose) breaks the same
    -- way ("message must be nonempty" from ai.a2a.tasks.submit). The diver
    -- repo is read-only to this driver, so the dropped field is repaired
    -- here between create and start_run — the exact two calls that
    -- harness.run() makes, with identical failure handling — and the REAL
    -- adapter code runs unmodified. If the harness is fixed, the repair
    -- becomes a no-op and the evidence notes it.
    local sup_mod = handles.supervisor
    local run, create_err = sup_mod.create(handles.sup, spec)
    if run == nil then
        return nil, create_err
    end
    if run.goal == nil then
        ev('HARNESS BUG: spec.goal validated but run.goal nil after create; repairing pre-start')
    else
        ev('note: harness propagates spec.goal now; repair unneeded')
    end
    run.goal = spec.goal
    local ok, start_err = sup_mod.start_run(handles.sup, run.id, spec.adapter)
    if not ok then
        sup_mod.finish(handles.sup, run.id, 'failed', 'invalid_adapter: ' .. tostring(start_err))
        return nil, start_err
    end
    return run.id
end

---@param task_id string
---@param want string
---@param timeout_ms integer
---@return boolean ok
---@return string state
---@return table? task
local function wait_task_state(task_id, want, timeout_ms)
    local tasks_mod = require('ai.a2a.tasks')
    local deadline = vim.uv.hrtime() + (timeout_ms * 1000000)
    while true do
        local task = tasks_mod.get(task_id)
        local state = (task ~= nil) and tostring(task.state) or 'missing'
        if state == want then
            return true, state, task
        end
        if vim.uv.hrtime() >= deadline then
            return false, state, task
        end
        vim.wait(WAIT_POLL_MS)
    end
end

---@param handles table
---@param run_id string
---@param kind string
---@return table[] events
local function sink_events(handles, run_id, kind)
    local out = {}
    for _, event in ipairs(handles.sink:events(run_id)) do
        if event.kind == kind then
            out[#out + 1] = event
        end
    end
    return out
end

-- ---------------------------------------------------------------------------
-- Scenarios.
-- ---------------------------------------------------------------------------

---Default: full lifecycle through the real adapter; wire shapes verified.
local function scenario_default(handles, peer, card, transitions)
    local run_id, run_err = start_run(handles, card)
    if run_id == nil then
        return fail('run-start', 'harness.run failed: ' .. tostring(run_err))
    end
    ev('run_id=' .. run_id)
    local reached, state = wait_state(handles, run_id, 'completed', WAIT_TIMEOUT_MS)
    if not reached then
        return fail('lifecycle', 'run never reached completed, state=' .. state)
    end
    for _, line in ipairs(peer.log) do
        ev('peer: ' .. line)
    end
    if not log_has(peer, 'rpc method=message/stream') then
        return fail('wire', 'message/stream never reached the mock peer')
    end
    if not log_has(peer, 'wire-shape-ok') then
        return fail('wire-shape', 'mock peer rejected the message wire shape')
    end
    ev('wire shapes verified: lowercase role=user, kind=message, part kind=text')
    ev('local a2a task transitions: ' .. table.concat(transitions, ' -> '))
    if table.concat(transitions, ',') ~= 'working,completed' then
        return fail('states', 'unexpected transitions: ' .. table.concat(transitions, ','))
    end
    local done_events = sink_events(handles, run_id, 'model.completed')
    if #done_events ~= 1 or done_events[1].payload.outcome ~= 'completed' then
        return fail('verdict', 'expected one model.completed with outcome=completed')
    end
    ev('verdict recorded: model.completed outcome=completed')
    ev('kebab-case states on the wire: working -> completed')
    return pass()
end

---Cancel: mid-stream cancel must POST tasks/cancel and settle cancelled.
local function scenario_cancel(handles, peer, card, _transitions)
    local run_id, run_err = start_run(handles, card)
    if run_id == nil then
        return fail('run-start', 'harness.run failed: ' .. tostring(run_err))
    end
    ev('run_id=' .. run_id)
    if not wait_log(peer, 'rpc method=message/stream', WAIT_TIMEOUT_MS) then
        return fail('wire', 'message/stream never reached the mock peer')
    end
    -- Let the working SSE event round-trip so the remote task id is known
    -- and the adapter's best-effort tasks/cancel actually goes out.
    vim.wait(CANCEL_DELAY_MS)
    local cancel_ok, cancel_err = handles.harness.cancel(run_id, 'gauntlet-cancel')
    if not cancel_ok then
        return fail('cancel', 'harness.cancel failed: ' .. tostring(cancel_err))
    end
    local state = run_state(handles, run_id)
    ev('state after cancel(): ' .. state)
    if state ~= 'cancelled' then
        return fail('cancel', 'expected state cancelled, got ' .. state)
    end
    if not wait_log(peer, 'rpc tasks/cancel', WAIT_TIMEOUT_MS) then
        return fail('wire', 'tasks/cancel never reached the mock peer')
    end
    ev('peer observed tasks/cancel: remote cancel requested for ' .. PEER_TASK_ID)
    for _, line in ipairs(peer.log) do
        ev('peer: ' .. line)
    end
    for _, event in ipairs(sink_events(handles, run_id, 'model.completed')) do
        if event.payload.outcome == 'completed' then
            return fail('cancel', 'run completed after cancel: stale callback leaked')
        end
    end
    local cancelled_seen = false
    for _, event in ipairs(sink_events(handles, run_id, 'run.finished')) do
        if event.payload.state == 'cancelled' then
            cancelled_seen = true
            ev('run.finished: state=cancelled reason=' .. tostring(event.payload.reason))
        end
    end
    if not cancelled_seen then
        return fail('cancel', 'no run.finished with state=cancelled')
    end
    return pass()
end

---Peer-dies: connection destroyed mid-stream must fail the run, not hang it.
---This scenario checks the REAL local task (ai.a2a.tasks) AND the harness
---run. The A2A layer must detect the truncated stream (curl exit 18) and
---record the task as failed; the harness run must then report failed. If the
---run reports completed while the task failed, that is the adapter bug
---diagnosed below, and the scenario fails with the exact mechanism.
local function scenario_peer_dies(handles, peer, card, _transitions, task_ids)
    local run_id, run_err = start_run(handles, card)
    if run_id == nil then
        return fail('run-start', 'harness.run failed: ' .. tostring(run_err))
    end
    ev('run_id=' .. run_id)
    local task_id = task_ids[1]
    if task_id == nil then
        return fail('peer-dies', 'no local a2a task observed')
    end
    ev('local a2a task id: ' .. tostring(task_id))
    -- Deadline-bounded: if the death is not detected, this fails instead of
    -- hanging. That is the no-hang proof.
    local started_ns = vim.uv.hrtime()
    local task_ok, task_state, task = wait_task_state(task_id, 'failed', WAIT_TIMEOUT_MS)
    local elapsed_ms = math.floor((vim.uv.hrtime() - started_ns) / 1000000)
    for _, line in ipairs(peer.log) do
        ev('peer: ' .. line)
    end
    ev('local a2a task state: ' .. task_state .. ' after ' .. elapsed_ms .. 'ms')
    if not task_ok then
        return fail('peer-dies', 'local a2a task never reached failed; state=' .. task_state)
    end
    local task_err = tostring(task ~= nil and task.error or nil)
    ev('local a2a task error: ' .. task_err)
    if not task_err:find('stream ended', 1, true) then
        return fail('peer-dies', 'unexpected local task error: ' .. task_err)
    end
    ev('A2A layer correctly recorded failure (curl transfer error, no hang)')
    -- Drain the harness: the adapter's on_done fires on the next ticks, the
    -- supervisor then marks the run. Bounded by the same no-hang deadline.
    local run_ok, run_state_now = wait_state(handles, run_id, 'failed', WAIT_TIMEOUT_MS)
    if not run_ok then
        run_state_now = run_state(handles, run_id)
    end
    for _, event in ipairs(sink_events(handles, run_id, 'model.completed')) do
        ev('sink model.completed: outcome=' .. tostring(event.payload.outcome)
            .. ' error=' .. tostring(event.payload.error))
    end
    if run_ok then
        ev('run terminated failed: peer death propagated correctly')
        return pass()
    end
    -- The task failed but the run did not. Root cause, verified against the
    -- primary sources: adapters/a2a.lua:63 registers
    --   on_done = function(result, task_err) ... outcome = task_err == nil
    --             and 'completed' or 'failed' ...
    -- but ai/a2a/tasks.lua:43 documents `on_done? fun(task: A2aTask)` and
    -- tasks.lua:170 calls `on_done(task)`. The adapter's `task_err` parameter
    -- is therefore always nil, so EVERY failed task is reported completed.
    -- (ai/a2a/fanout.lua:84 uses the correct `function(task)` shape.)
    return fail('peer-dies',
        'HARNESS ADAPTER BUG: local a2a task is failed but the harness run is '
            .. tostring(run_state_now)
            .. '; adapters/a2a.lua on_done(result, task_err) mismatches '
            .. 'ai.a2a.tasks on_done(task) (tasks.lua:170), so task_err is '
            .. 'always nil and failures are reported completed')
end

---Bad-state: an unknown state string must be ignored, not corrupt the run.
local function scenario_bad_state(handles, peer, card, transitions)
    local run_id, run_err = start_run(handles, card)
    if run_id == nil then
        return fail('run-start', 'harness.run failed: ' .. tostring(run_err))
    end
    ev('run_id=' .. run_id)
    local reached, state = wait_state(handles, run_id, 'completed', WAIT_TIMEOUT_MS)
    if not reached then
        return fail('bad-state', 'run never reached completed, state=' .. state)
    end
    for _, line in ipairs(peer.log) do
        ev('peer: ' .. line)
    end
    if not log_has(peer, 'frobnicate') then
        return fail('bad-state', 'mock peer never sent the unknown state')
    end
    ev('wire carried unknown state string: frobnicate')
    for _, s in ipairs(transitions) do
        if s == 'frobnicate' then
            return fail('bad-state', 'unknown state leaked into the task state machine')
        end
    end
    ev('local a2a task transitions: ' .. table.concat(transitions, ' -> '))
    if table.concat(transitions, ',') ~= 'working,completed' then
        return fail('states', 'unexpected transitions: ' .. table.concat(transitions, ','))
    end
    ev('unknown state ignored by set_state: run completed uncorrupted')
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
    local peer, peer_err = peer_start(scenario, GOAL_TEXT)
    if peer == nil then
        return fail('peer', 'mock peer failed to start: ' .. tostring(peer_err))
    end
    ev('mock peer listening on 127.0.0.1:' .. peer.port)
    local card, card_err = fetch_card(peer.port, CARD_FETCH_TIMEOUT_MS)
    if card == nil then
        peer_stop(peer)
        return fail('card-fetch', tostring(card_err))
    end
    ev('agent card fetched: name=' .. card.name .. ' version=' .. card.version)
    -- Observe the REAL local task state machine (ai.a2a.tasks), not the
    -- driver's assumptions: every state transition is recorded, along with
    -- the local task id that produced it.
    local transitions = {}
    local task_ids = {}
    require('ai.a2a.tasks').subscribe(function(event)
        if type(event) == 'table' and event.type == 'state' then
            transitions[#transitions + 1] = tostring(event.state)
            task_ids[#task_ids + 1] = event.id
        end
    end)
    local verdict
    if scenario == 'default' then
        verdict = scenario_default(handles, peer, card, transitions)
    elseif scenario == 'cancel' then
        verdict = scenario_cancel(handles, peer, card, transitions)
    elseif scenario == 'peer-dies' then
        verdict = scenario_peer_dies(handles, peer, card, transitions, task_ids)
    elseif scenario == 'bad-state' then
        verdict = scenario_bad_state(handles, peer, card, transitions)
    else
        verdict = fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
    end
    peer_stop(peer)
    return verdict
end

local ok, verdict = pcall(main)
if not ok then
    ev('lua error: ' .. tostring(verdict))
    verdict = fail('lua-driver', 'unhandled error: ' .. tostring(verdict))
end
-- verdict on the real stdout: in `nvim --headless -l`, Lua print() goes
-- to stderr, but the Rust runner parses the verdict from stdout.
io.stdout:write(vim.json.encode(verdict) .. '\n')
