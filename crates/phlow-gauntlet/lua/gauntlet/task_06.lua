-- task-06 driver: ACP interop round-trip through diver's REAL acp adapter.
--
-- Drives lua/ai/harness/adapters/acp.lua end to end against a mock ACP
-- agent. The mock is a small Python program (written to GAUNTLET_WORK_DIR)
-- that speaks the exact JSON-RPC 2.0 newline-delimited stdio protocol the
-- adapter's transport (lua/ai/acp/rpc.lua) implements: it answers
-- `initialize`, `session/new`, and `session/prompt`, and emits
-- `session/update` agent_message_chunk notifications. The mock is
-- registered at runtime by inserting into `ai.acp.registry.manual` -- the
-- same table `registry.get()` consults first -- so the diver repo itself
-- is never modified.
--
-- Scenarios via GAUNTLET_SCENARIO (default "default"):
--   default         initialize -> new session -> prompt -> streamed reply,
--                   observed through the adapter's sink events
--   send-input      mid-session input via the adapter's real
--                   send_input(handle, input) reaches the mock agent
--   unknown-agent   extensions.acp.agent names nothing registered ->
--                   clean async failure, no hang, run finishes failed
--   malformed-frame mock interleaves garbage lines (non-JSON, JSON
--                   non-object, unknown method, unknown response id) ->
--                   the transport ignores them, the run still completes
--
-- Prints exactly one JSON verdict line to stdout and always exits 0; the
-- verdict carries the outcome, not the exit code. Writes nothing outside
-- GAUNTLET_WORK_DIR. Never modifies the diver repo.

local TASK_ID = 'task-06'
local MOCK_AGENT = 'gauntlet-mock-acp'
local MOCK_SCRIPT = 'mock_acp_agent.py'
local CHUNK_DEFAULT = 'DEFAULT_TURN_CHUNK'
local CHUNK_FIRST = 'FIRST_TURN_CHUNK'
local CHUNK_SECOND = 'SECOND_TURN_CHUNK'
local UNKNOWN_AGENT_NAME = 'gauntlet-no-such-agent'

local EVIDENCE_MAX = 64
local WAIT_POLL_MS = 25
local RUN_TIMEOUT_MS = 60000
local SETTLE_WAIT_MS = 25000
local CHUNK_WAIT_MS = 20000

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

---Mock ACP agent: JSON-RPC 2.0, one JSON object per newline-delimited
---line on stdio, matching lua/ai/acp/rpc.lua's framing exactly.
---argv[1] selects the mode: 'default', 'send-input', or 'malformed'.
---Written verbatim to GAUNTLET_WORK_DIR by write_mock_agent().
local MOCK_AGENT_SOURCE = [[
import json
import os
import sys

MODE = sys.argv[1] if len(sys.argv) > 1 else "default"
SESSION_ID = "mock-session-1"
# Frame log: lets the driver prove adversarial frames really crossed the
# wire (the transport swallows them silently, which is the point).
FRAME_LOG = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                         "mock_agent_frames.log")


def log_frame(kind, line):
    with open(FRAME_LOG, "a") as fh:
        fh.write(kind + ":" + line.rstrip("\n") + "\n")


def send(obj):
    sys.stdout.write(json.dumps(obj, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def notify_chunk(text):
    send({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": SESSION_ID,
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": text},
            },
        },
    })


# Adversarial frames the real transport must swallow without crashing:
# non-JSON garbage, valid JSON that is not an object, a notification for
# an unknown method, and a response for an unknown request id.
GARBAGE_LINES = [
    "THIS IS NOT JSON AT ALL\n",
    "[1,2,3]\n",
    '{"jsonrpc":"2.0","method":"bogus/method","params":{}}\n',
    '{"jsonrpc":"2.0","id":424242,"result":{}}\n',
]


def reply(msg_id, result):
    send({"jsonrpc": "2.0", "id": msg_id, "result": result})


def main():
    # Truncate the frame log so re-runs in the same work dir start clean.
    open(FRAME_LOG, "w").close()
    prompt_count = 0
    held_prompt_id = None
    for raw in sys.stdin:
        line = raw.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except Exception:
            continue
        if not isinstance(msg, dict):
            continue
        method = msg.get("method")
        msg_id = msg.get("id")
        if method == "initialize":
            if MODE == "malformed":
                for junk in GARBAGE_LINES:
                    sys.stdout.write(junk)
                    log_frame("garbage", junk)
                sys.stdout.flush()
            reply(msg_id, {"protocolVersion": 1, "agentCapabilities": {}})
        elif method == "session/new":
            if MODE == "malformed":
                sys.stdout.write("GARBAGE BETWEEN NEW AND PROMPT\n")
                log_frame("garbage", "GARBAGE BETWEEN NEW AND PROMPT\n")
                sys.stdout.flush()
            reply(msg_id, {"sessionId": SESSION_ID})
        elif method == "session/prompt":
            prompt_count += 1
            if MODE == "send-input":
                if prompt_count == 1:
                    # Hold the first prompt open: the run must stay
                    # 'running' until follow-up input arrives.
                    notify_chunk("FIRST_TURN_CHUNK")
                    held_prompt_id = msg_id
                else:
                    notify_chunk("SECOND_TURN_CHUNK")
                    reply(msg_id, {"stopReason": "end_turn"})
                    if held_prompt_id is not None:
                        reply(held_prompt_id, {"stopReason": "end_turn"})
                        held_prompt_id = None
            else:
                notify_chunk("DEFAULT_TURN_CHUNK")
                reply(msg_id, {"stopReason": "end_turn"})
        # session/cancel and anything else: ignored (notifications need
        # no reply, unknown shapes are not the mock's business).


if __name__ == "__main__":
    main()
]]

---Write the mock agent into the work dir. Returns path or (nil, err).
---@param work_dir string
---@return string? path
---@return string? err
local function write_mock_agent(work_dir)
    local path = vim.fs.joinpath(work_dir, MOCK_SCRIPT)
    local lines = vim.split(MOCK_AGENT_SOURCE, '\n', { plain = true })
    local ok = pcall(vim.fn.writefile, lines, path)
    if not ok then
        return nil, 'cannot write mock agent to ' .. path
    end
    return path
end

---Wire up the harness and register the mock ACP agent at runtime.
---Returns driver handles or (nil, err).
---@param mode string
---@return table? handles
---@return string? err
local function bootstrap(mode)
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
    if vim.fn.executable('python3') ~= 1 then
        return nil, 'python3 is not on PATH (mock ACP agent needs it)'
    end
    local script_path, write_err = write_mock_agent(work_dir)
    if script_path == nil then
        return nil, write_err
    end
    ev('mock agent written: ' .. script_path)
    -- Runtime registration: registry.get() consults M.manual first, so
    -- inserting here is exactly what a diver-repo edit would do, without
    -- touching the repo. This exercises the adapter's REAL lookup path.
    local acp_registry = require('ai.acp.registry')
    acp_registry.manual[MOCK_AGENT] = {
        cmd = { vim.fn.exepath('python3'), script_path, mode },
        kind = 'native',
        verified = true,
        notes = 'gauntlet mock ACP agent (test-only, runtime registration)',
    }
    ev('mock agent registered as "' .. MOCK_AGENT .. '" mode=' .. mode)
    return {
        harness = harness,
        sup = st.supervisor,
        sink = st.sink,
        supervisor = require('ai.harness.supervisor'),
        types = require('ai.harness.types'),
        acp_adapter = require('ai.harness.adapters.acp'),
        work_dir = work_dir,
        agent = MOCK_AGENT,
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

---Drive the supervisor clock until a sink event matches `pred`.
---@param handles table
---@param run_id string
---@param pred fun(event: table): boolean
---@param timeout_ms integer
---@return table? event
local function wait_for_event(handles, run_id, pred, timeout_ms)
    local deadline = vim.uv.hrtime() + (timeout_ms * 1000000)
    while true do
        handles.supervisor.tick(handles.sup, handles.types.now_ns())
        for _, event in ipairs(handles.sink:events(run_id)) do
            if pred(event) then
                return event
            end
        end
        if vim.uv.hrtime() >= deadline then
            return nil
        end
        vim.wait(WAIT_POLL_MS)
    end
end

---@param handles table
---@param run_id string
---@param agent_name string
---@param goal string
---@return string? run_id
---@return string? err
local function start_run(handles, agent_name, goal)
    local spec = {
        workflow = 'gauntlet-acp-roundtrip',
        goal = goal,
        workspace = handles.work_dir,
        adapter = 'acp',
        timeout_ms = RUN_TIMEOUT_MS,
        extensions = { acp = { agent = agent_name } },
    }
    local run_id, run_err = handles.harness.run(spec)
    if run_id == nil then
        return nil, run_err
    end
    -- DIVER BUG WORKAROUND (do not "fix" by editing diver here): the
    -- harness validates spec.goal as required (harness/types.lua) and the
    -- ACP adapter reads run.goal (harness/adapters/acp.lua), but
    -- supervisor.create never stores goal on the run record, so the
    -- adapter's session.prompt gets nil and protocol.prompt_params
    -- asserts 'text must be a nonempty string'. Restore the goal the
    -- supervisor was supposed to carry. This runs synchronously in the
    -- same tick as harness.run(); the adapter only reads run.goal from
    -- the session.start callback, which cannot fire before the event
    -- loop turns (it needs a full agent-subprocess round-trip first),
    -- so the restore is deterministic, not racy.
    local run = handles.supervisor.get(handles.sup, run_id)
    if run ~= nil then
        run.goal = goal
        ev('workaround: restored run.goal dropped by supervisor.create (diver bug)')
    end
    return run_id
end

---@param event table
---@return string? chunk text when event is an ACP session_update chunk
local function chunk_text_of(event)
    if event.kind ~= 'diagnostic.observed' then
        return nil
    end
    local payload = event.payload
    if type(payload) ~= 'table' or payload.kind ~= 'session_update' then
        return nil
    end
    local update = payload.update
    if type(update) ~= 'table' then
        return nil
    end
    local inner = update.update
    if type(inner) ~= 'table' or inner.sessionUpdate ~= 'agent_message_chunk' then
        return nil
    end
    local content = inner.content
    if type(content) ~= 'table' or content.type ~= 'text' then
        return nil
    end
    return content.text
end

---@param handles table
---@param run_id string
---@return string[] chunk texts in arrival order
local function session_chunks(handles, run_id)
    local out = {}
    for _, event in ipairs(handles.sink:events(run_id)) do
        local text = chunk_text_of(event)
        if text ~= nil then
            out[#out + 1] = text
        end
    end
    return out
end

---@param handles table
---@param run_id string
---@return table? event the model.completed event, if any
local function model_completed(handles, run_id)
    for _, event in ipairs(handles.sink:events(run_id)) do
        if event.kind == 'model.completed' then
            return event
        end
    end
    return nil
end

---@param handles table
---@param run_id string
---@return table? event the run.finished event, if any
local function run_finished(handles, run_id)
    for _, event in ipairs(handles.sink:events(run_id)) do
        if event.kind == 'run.finished' then
            return event
        end
    end
    return nil
end

---@param chunk string
---@return fun(event: table): boolean
local function has_chunk(chunk)
    return function(event)
        return chunk_text_of(event) == chunk
    end
end

---Default: initialize -> new session -> prompt -> streamed response,
---all through the real ACP adapter; sink events are the evidence.
local function scenario_default(handles)
    local run_id, run_err = start_run(handles, handles.agent, 'Reply with the default turn chunk.')
    if run_id == nil then
        return fail('run-start', 'harness.run failed: ' .. tostring(run_err))
    end
    ev('run_id=' .. run_id)
    local found = wait_for_event(handles, run_id, has_chunk(CHUNK_DEFAULT), CHUNK_WAIT_MS)
    if found == nil then
        return fail('stream', 'never observed session_update chunk "' .. CHUNK_DEFAULT .. '"')
    end
    ev('session_update chunk observed: ' .. CHUNK_DEFAULT)
    local reached, state = wait_state(handles, run_id, 'completed', SETTLE_WAIT_MS)
    if not reached then
        return fail('complete', 'run never completed, state=' .. state)
    end
    ev('run state: completed')
    local done = model_completed(handles, run_id)
    if done == nil or done.payload.outcome ~= 'completed' then
        return fail('complete', 'model.completed missing or not completed')
    end
    ev('model.completed outcome=completed (adapter bridged session/update + prompt result)')
    local finished = run_finished(handles, run_id)
    if finished == nil or finished.payload.state ~= 'completed' then
        return fail('complete', 'run.finished missing or wrong state')
    end
    ev('run.finished state=completed: full ACP round-trip through the real adapter')
    return pass()
end

---send-input: mid-session input via the adapter's real
---send_input(handle, input) reaches the mock agent as a second prompt.
local function scenario_send_input(handles)
    local run_id, run_err = start_run(handles, handles.agent, 'First prompt; hold for follow-up.')
    if run_id == nil then
        return fail('run-start', 'harness.run failed: ' .. tostring(run_err))
    end
    ev('run_id=' .. run_id)
    local first = wait_for_event(handles, run_id, has_chunk(CHUNK_FIRST), CHUNK_WAIT_MS)
    if first == nil then
        return fail('stream', 'never observed session_update chunk "' .. CHUNK_FIRST .. '"')
    end
    ev('first-turn chunk observed: ' .. CHUNK_FIRST)
    local run = handles.supervisor.get(handles.sup, run_id)
    if run == nil or run.handle == nil then
        return fail('send-input', 'no adapter handle on the run')
    end
    local sent, send_err =
        handles.acp_adapter.send_input(run.handle, { text = 'gauntlet follow-up prompt' })
    if not sent then
        return fail('send-input', 'adapter send_input failed: ' .. tostring(send_err))
    end
    ev('adapter send_input accepted mid-session input')
    local second = wait_for_event(handles, run_id, has_chunk(CHUNK_SECOND), CHUNK_WAIT_MS)
    if second == nil then
        return fail('send-input', 'follow-up chunk "' .. CHUNK_SECOND .. '" never reached the sink')
    end
    ev('second-turn chunk observed: ' .. CHUNK_SECOND .. ' (input reached the agent)')
    local reached, state = wait_state(handles, run_id, 'completed', SETTLE_WAIT_MS)
    if not reached then
        return fail('complete', 'run never completed after follow-up, state=' .. state)
    end
    ev('run.finished state=completed after the second turn')
    return pass()
end

---unknown-agent: the agent name is not registered. The adapter must fail
---the run cleanly (model.completed outcome=failed, run finished failed)
---without hanging the supervisor.
local function scenario_unknown_agent(handles)
    local run_id, run_err = start_run(handles, UNKNOWN_AGENT_NAME, 'This prompt should never run.')
    if run_id == nil then
        return fail(
            'run-start',
            'expected async unknown-agent failure, got sync: ' .. tostring(run_err)
        )
    end
    ev('run_id=' .. run_id)
    local reached, state = wait_state(handles, run_id, 'failed', SETTLE_WAIT_MS)
    if not reached then
        return fail('unknown-agent', 'run did not fail cleanly (hang?), state=' .. state)
    end
    ev('no hang: run reached state=failed')
    local done = model_completed(handles, run_id)
    if done == nil or done.payload.outcome ~= 'failed' then
        return fail('unknown-agent', 'model.completed missing or not failed')
    end
    local err_text = tostring(done.payload.error)
    ev('model.completed outcome=failed error=' .. err_text)
    if err_text:find('Unknown ACP agent', 1, true) == nil then
        return fail('unknown-agent', 'failure not attributed to the unknown agent: ' .. err_text)
    end
    local finished = run_finished(handles, run_id)
    if finished == nil or finished.payload.state ~= 'failed' then
        return fail('unknown-agent', 'run.finished missing or wrong state')
    end
    ev('run.finished state=failed: supervisor alive, verdict delivered')
    return pass()
end

---malformed-frame: the mock interleaves garbage frames (non-JSON line,
---JSON non-object, unknown-method notification, unknown-id response).
---The transport must ignore them; the run still completes.
local function scenario_malformed_frame(handles)
    local run_id, run_err = start_run(handles, handles.agent, 'Reply despite the garbage frames.')
    if run_id == nil then
        return fail('run-start', 'harness.run failed: ' .. tostring(run_err))
    end
    ev('run_id=' .. run_id)
    local reached, state = wait_state(handles, run_id, 'completed', SETTLE_WAIT_MS)
    if not reached then
        return fail('malformed-frame', 'run died or hung on garbage frames, state=' .. state)
    end
    -- Prove the adversarial frames really crossed the wire: the mock
    -- logs every garbage line it emits (the transport itself swallows
    -- them silently, which is the behavior under test).
    local frame_log = vim.fs.joinpath(handles.work_dir, 'mock_agent_frames.log')
    local log_lines = {}
    if vim.fn.filereadable(frame_log) == 1 then
        log_lines = vim.fn.readfile(frame_log)
    end
    local garbage_count = 0
    for _, line in ipairs(log_lines) do
        if line:sub(1, 8) == 'garbage:' then
            garbage_count = garbage_count + 1
        end
    end
    ev('adversarial frames emitted by the mock: ' .. garbage_count)
    if garbage_count ~= 5 then
        return fail('malformed-frame', 'expected 5 garbage frames, mock logged ' .. garbage_count)
    end
    local chunks = session_chunks(handles, run_id)
    local saw_default = false
    for _, text in ipairs(chunks) do
        if text == CHUNK_DEFAULT then
            saw_default = true
        end
    end
    if not saw_default then
        return fail('malformed-frame', 'session_update chunks lost after garbage frames')
    end
    ev('session_update chunk survived the garbage: ' .. CHUNK_DEFAULT)
    local finished = run_finished(handles, run_id)
    if finished == nil or finished.payload.state ~= 'completed' then
        return fail('malformed-frame', 'run.finished missing or wrong state')
    end
    ev('run.finished state=completed: transport swallowed garbage, supervisor unharmed')
    return pass()
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'default'
    end
    -- Mock mode is not 1:1 with the scenario name: 'unknown-agent' never
    -- spawns the mock, and 'malformed-frame' maps to the mock's
    -- 'malformed' mode (exact string the mock checks).
    local mode = 'default'
    if scenario == 'send-input' then
        mode = 'send-input'
    elseif scenario == 'malformed-frame' then
        mode = 'malformed'
    end
    local handles, boot_err = bootstrap(mode)
    if handles == nil then
        return fail('bootstrap', boot_err)
    end
    ev('scenario=' .. scenario)
    if scenario == 'default' then
        return scenario_default(handles)
    elseif scenario == 'send-input' then
        return scenario_send_input(handles)
    elseif scenario == 'unknown-agent' then
        return scenario_unknown_agent(handles)
    elseif scenario == 'malformed-frame' then
        return scenario_malformed_frame(handles)
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
