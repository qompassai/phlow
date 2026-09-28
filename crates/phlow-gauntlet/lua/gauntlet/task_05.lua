-- task-05 driver: cancel/resume lifecycle integrity for diver's ai.harness.
--
-- Proves three things: (1) a run cancelled mid-flight lands in `cancelled`
-- with the cancel reason recorded; (2) resume() re-queues it and it reaches
-- `completed` exactly once (no double-completion, no resurrection);
-- (3) resume() of a completed run and cancel() of a terminal or unknown run
-- are clean rejections that leave state unchanged.
--
-- The driver registers its own fake slow adapter (`gauntlet_slow`) in the
-- harness registry. Real protocol adapters (e.g. herd) spawn live external
-- workers, which is non-deterministic for a lifecycle-integrity probe. The
-- fake adapter honors the adapter contract (probe/start/cancel/close) and
-- completes a run only when run.attempt reaches
-- extensions.gauntlet.complete_on_attempt, so attempt 1 stays running
-- (cancellable mid-flight) while the resumed attempt finishes.
--
-- Scenarios via GAUNTLET_SCENARIO (default "default"):
--   default           cancel mid-run with reason, then resume to completion
--   resume-completed  resume a completed run -> must be rejected
--   cancel-terminal   cancel an already-terminal run -> clean error
--   cancel-unknown    cancel a bogus run id -> clean error
--
-- Prints exactly one JSON verdict line to stdout and always exits 0; the
-- verdict carries the outcome, not the exit code. Writes nothing outside
-- GAUNTLET_WORK_DIR. Never modifies the diver repo.

local SLOW_ADAPTER = 'gauntlet_slow'
local CANCEL_REASON = 'gauntlet-test'
local EVIDENCE_MAX = 64
local WAIT_POLL_MS = 25
local RUN_TIMEOUT_MS = 60000
local COMPLETE_WAIT_MS = 15000
local MIDFLIGHT_WAIT_MS = 1200

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function fail(where, how)
    return { id = 'task-05', outcome = 'fail', where = where, how = how, evidence = evidence }
end

local function pass()
    return { id = 'task-05', outcome = 'pass', evidence = evidence }
end

---Fake slow worker. Completes only on attempt >= complete_on_attempt so the
---first attempt is a sitting duck for cancel().
local slow_adapter = { name = SLOW_ADAPTER }

function slow_adapter.probe()
    return {
        available = true,
        streaming = false,
        cancellation = true,
        resume = false,
        permissions = false,
        artifacts = false,
        remote = false,
        tools = false,
    }
end

---@param run table
---@param sink table
---@return table? handle
function slow_adapter.start(run, sink)
    assert(run ~= nil, 'run required')
    assert(sink ~= nil, 'sink required')
    local ext = (run.extensions ~= nil and run.extensions.gauntlet) or {}
    local complete_on_attempt = ext.complete_on_attempt or math.huge
    if run.attempt >= complete_on_attempt then
        sink:append(
            run.id,
            'model.completed',
            { outcome = 'completed' },
            { source = SLOW_ADAPTER }
        )
    else
        sink:append(run.id, 'diagnostic.observed', {
            adapter = SLOW_ADAPTER,
            kind = 'slow_work_started',
            attempt = run.attempt,
        }, { source = SLOW_ADAPTER })
    end
    return { adapter = SLOW_ADAPTER, run_id = run.id, attempt = run.attempt, closed = false }
end

---@param handle table
---@return boolean
function slow_adapter.cancel(handle, _reason)
    assert(handle ~= nil, 'handle required')
    return true
end

---@param handle table
function slow_adapter.close(handle)
    assert(handle ~= nil, 'handle required')
    handle.closed = true
end

---Wire up the harness and the fake adapter. Returns driver handles.
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
    -- Test introspection, not public API: the gauntlet needs the registry
    -- to install its fake adapter and the supervisor to observe states.
    local st = harness._state
    if st == nil then
        return nil, 'harness internal state unavailable after setup'
    end
    local registry = require('ai.harness.registry')
    local reg_ok, reg_err = registry.register_adapter(st.registry, SLOW_ADAPTER, slow_adapter)
    if not reg_ok then
        return nil, 'fake adapter registration failed: ' .. tostring(reg_err)
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

---@param handles table
---@param run_id string
---@return table[] events
local function finished_events(handles, run_id)
    local out = {}
    for _, event in ipairs(handles.sink:events(run_id)) do
        if event.kind == 'run.finished' then
            out[#out + 1] = event
        end
    end
    return out
end

---@param handles table
---@param complete_on_attempt integer
---@return string? run_id
---@return string? err
local function start_run(handles, complete_on_attempt)
    local spec = {
        workflow = 'gauntlet-cancel-resume',
        goal = 'lifecycle integrity probe: slow cancellable work',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = SLOW_ADAPTER,
        timeout_ms = RUN_TIMEOUT_MS,
        extensions = { gauntlet = { complete_on_attempt = complete_on_attempt } },
    }
    return handles.harness.run(spec)
end

---Default: cancel mid-flight with a reason, then resume to completion.
local function scenario_default(handles)
    local run_id, run_err = start_run(handles, 2)
    if run_id == nil then
        return fail('run-start', 'harness.run failed: ' .. tostring(run_err))
    end
    ev('run_id=' .. run_id)
    local state = run_state(handles, run_id)
    ev('state after run(): ' .. state)
    if state ~= 'running' then
        return fail('run-start', 'expected state running, got ' .. state)
    end
    -- Mid-flight pause: attempt 1 never self-completes, so without the
    -- cancel below this run would stay running.
    vim.wait(MIDFLIGHT_WAIT_MS)
    local cancel_ok, cancel_err = handles.harness.cancel(run_id, CANCEL_REASON)
    if not cancel_ok then
        return fail('cancel', 'harness.cancel failed: ' .. tostring(cancel_err))
    end
    state = run_state(handles, run_id)
    ev('state after cancel(): ' .. state)
    if state ~= 'cancelled' then
        return fail('cancel', 'expected state cancelled, got ' .. state)
    end
    local cancel_reason_seen = nil
    for _, event in ipairs(finished_events(handles, run_id)) do
        if event.payload.state == 'cancelled' then
            cancel_reason_seen = event.payload.reason
        end
    end
    if cancel_reason_seen ~= CANCEL_REASON then
        return fail(
            'cancel',
            'run.finished missing or wrong cancel reason: ' .. tostring(cancel_reason_seen)
        )
    end
    ev('run.finished observed: state=cancelled reason=' .. cancel_reason_seen)
    local resume_ok, resume_err = handles.harness.resume(run_id)
    if not resume_ok then
        return fail('resume', 'harness.resume failed: ' .. tostring(resume_err))
    end
    local run = handles.supervisor.get(handles.sup, run_id)
    ev('attempt after resume: ' .. tostring(run.attempt))
    local reached, final_state = wait_state(handles, run_id, 'completed', COMPLETE_WAIT_MS)
    if not reached then
        return fail('resume', 'resumed run never completed, state=' .. final_state)
    end
    local finished = finished_events(handles, run_id)
    local completed_count = 0
    for _, event in ipairs(finished) do
        if event.payload.state == 'completed' then
            completed_count = completed_count + 1
        end
    end
    ev('run.finished events: total=' .. #finished .. ' completed=' .. completed_count)
    if #finished ~= 2 or completed_count ~= 1 then
        return fail('resume', 'expected exactly one completion (cancelled, then completed)')
    end
    ev('resumed run completed exactly once: no double-complete, no resurrection')
    return pass()
end

---resume() of a completed run must be rejected, state untouched.
local function scenario_resume_completed(handles)
    local run_id, run_err = start_run(handles, 1)
    if run_id == nil then
        return fail('run-start', 'harness.run failed: ' .. tostring(run_err))
    end
    local reached, state = wait_state(handles, run_id, 'completed', COMPLETE_WAIT_MS)
    if not reached then
        return fail('run-start', 'run never completed, state=' .. state)
    end
    ev('run completed on attempt 1')
    local resume_ok, resume_err = handles.harness.resume(run_id)
    if resume_ok then
        return fail('resume-completed', 'resume() accepted a completed run: lifecycle violated')
    end
    ev('resume() rejected: ' .. tostring(resume_err))
    local want = 'invalid transition completed -> queued'
    if tostring(resume_err):find(want, 1, true) == nil then
        return fail('resume-completed', 'unexpected rejection error: ' .. tostring(resume_err))
    end
    state = run_state(handles, run_id)
    if state ~= 'completed' then
        return fail('resume-completed', 'state changed after rejected resume: ' .. state)
    end
    ev('state still completed: completed run was not resurrected')
    return pass()
end

---cancel() of an already-terminal run is a clean error, state unchanged.
local function scenario_cancel_terminal(handles)
    local run_id, run_err = start_run(handles, 1)
    if run_id == nil then
        return fail('run-start', 'harness.run failed: ' .. tostring(run_err))
    end
    local reached, state = wait_state(handles, run_id, 'completed', COMPLETE_WAIT_MS)
    if not reached then
        return fail('run-start', 'run never completed, state=' .. state)
    end
    ev('run completed on attempt 1')
    local cancel_ok, cancel_err = handles.harness.cancel(run_id, CANCEL_REASON)
    if cancel_ok then
        return fail('cancel-terminal', 'cancel() accepted an already-terminal run')
    end
    ev('cancel() rejected: ' .. tostring(cancel_err))
    if tostring(cancel_err):find('already terminal', 1, true) == nil then
        return fail('cancel-terminal', 'unexpected rejection error: ' .. tostring(cancel_err))
    end
    state = run_state(handles, run_id)
    if state ~= 'completed' then
        return fail('cancel-terminal', 'state changed after rejected cancel: ' .. state)
    end
    ev('state still completed: terminal state unchanged')
    return pass()
end

---cancel() of a bogus run id is a clean error and creates nothing.
local function scenario_cancel_unknown(handles)
    local bogus = 'run-0000-bogus-id'
    local cancel_ok, cancel_err = handles.harness.cancel(bogus, CANCEL_REASON)
    if cancel_ok then
        return fail('cancel-unknown', 'cancel() accepted an unknown run id')
    end
    ev('cancel() rejected: ' .. tostring(cancel_err))
    if tostring(cancel_err):find('unknown run', 1, true) == nil then
        return fail('cancel-unknown', 'unexpected rejection error: ' .. tostring(cancel_err))
    end
    local runs = handles.supervisor.list(handles.sup)
    if #runs ~= 0 then
        return fail('cancel-unknown', 'runs table not empty after bogus cancel: ' .. #runs)
    end
    ev('no run created: clean error for unknown id')
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
    elseif scenario == 'resume-completed' then
        return scenario_resume_completed(handles)
    elseif scenario == 'cancel-terminal' then
        return scenario_cancel_terminal(handles)
    elseif scenario == 'cancel-unknown' then
        return scenario_cancel_unknown(handles)
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
