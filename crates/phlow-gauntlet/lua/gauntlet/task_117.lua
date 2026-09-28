-- task-117 driver: completion-report integrity probe (diver Fix 2).
--
-- The defect: the real contract in `ai/a2a/tasks.lua` is
-- `on_done? fun(task: A2aTask)` — one argument — but the harness adapter
-- declares `function(result, task_err)`, so `task_err` is always nil and
-- the outcome is always `'completed'`. Failed remote tasks are recorded
-- as completed.
--
-- Diver probed: c84352cc850d507df477706b9166b6541ebe9e1c (main; no Phase-2
-- branch exists).
--
-- The driver stubs `ai.a2a.tasks.submit` (via package.preload) so the
-- REAL `adapters/a2a.lua` runs; the stub captures the submit opts and the
-- driver invokes the captured `on_done` with fabricated task tables in
-- each terminal state, then reads back the `model.completed` event the
-- adapter appends.
--
-- Scenarios via GAUNTLET_SCENARIO (default "default"):
--   default                  failed -> failed (V)
--   completed-maps-completed completed -> completed (V)
--   rejected-canceled        rejected -> failed, canceled -> cancelled (A)
--   garbage-nil-double       garbage state -> failed (fail-closed, never
--                            completed), state=nil -> failed, double
--                            on_done -> exactly one run.finished (A)
--
-- A scenario reports outcome='fail', where='fix-2-absent' when its probes
-- show the outcome was misreported; 'pass' when every probe holds. Today
-- only completed-maps-completed passes (correct by accident: task_err is
-- nil for every invocation). The record IS the Phase-2 acceptance
-- artifact.
--
-- Prints exactly one JSON verdict line to stdout and always exits 0.
-- Writes nothing outside GAUNTLET_WORK_DIR. Never modifies the diver repo.

local DIVER_SHA = 'c84352cc850d507df477706b9166b6541ebe9e1c'
local EVIDENCE_MAX = 64

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function fix_absent(how)
    return { id = 'task-117', outcome = 'fail', where = 'fix-2-absent', how = how, evidence = evidence }
end

local function scenario_pass(note)
    ev(note)
    return { id = 'task-117', outcome = 'pass', evidence = evidence }
end

local function driver_fail(where, how)
    return { id = 'task-117', outcome = 'fail', where = where, how = how, evidence = evidence }
end

---Wire up the harness and stub ai.a2a.tasks. Returns handles or (nil, err).
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
    -- Stub the a2a transport: capture submit opts (message, on_done) and
    -- return a fake task id WITHOUT invoking on_done, so the driver
    -- controls exactly what the adapter's callback receives.
    local submitted = {}
    package.preload['ai.a2a.tasks'] = function()
        return {
            submit = function(opts)
                submitted[#submitted + 1] = opts
                return 'task-fake-' .. #submitted, nil
            end,
            cancel = function()
                return true
            end,
        }
    end
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
        submitted = submitted,
    }
end

---Run one a2a run, invoke the adapter's captured on_done with a
---fabricated task table, drain, and return the recorded outcome.
---@return string? outcome
---@return string? run_id_or_err
local function probe_state(handles, state_value)
    local run_id, run_err = handles.harness.run({
        workflow = 'gauntlet_a2a_flow',
        goal = 'a2a completion probe',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = 'a2a',
        extensions = { a2a = { agent = 'gauntlet-fake-agent' } },
        timeout_ms = 30000,
    })
    if run_id == nil then
        return nil, 'harness.run failed: ' .. tostring(run_err)
    end
    local sub = handles.submitted[#handles.submitted]
    if sub == nil or type(sub.on_done) ~= 'function' then
        return nil, 'stub captured no on_done for the submit'
    end
    -- The real contract is on_done(task): one argument. The adapter
    -- declares function(result, task_err), so task_err is always nil here.
    sub.on_done({ state = state_value, id = 'task-fake-probe' })
    handles.supervisor.tick(handles.sup, handles.types.now_ns())
    for _, event in ipairs(handles.sink:events(run_id)) do
        if event.kind == 'model.completed' and event.source == 'a2a' then
            return event.payload.outcome, run_id
        end
    end
    return nil, 'no model.completed event from the a2a adapter'
end

local function count_events(handles, run_id, kind)
    local n = 0
    for _, event in ipairs(handles.sink:events(run_id)) do
        if event.kind == kind then
            n = n + 1
        end
    end
    return n
end

local function defect_how()
    return 'fix-2 absent: adapters/a2a.lua line 63 declares '
        .. 'on_done = function(result, task_err), but the real contract in '
        .. 'ai/a2a/tasks.lua line 43 is on_done? fun(task: A2aTask) — one '
        .. 'argument, invoked as on_done(task). task_err is therefore always '
        .. 'nil and the recorded outcome is always "completed": failed remote '
        .. 'tasks are recorded as completed. Phase-2 acceptance: outcome '
        .. 'derived from task.state — completed->completed, canceled->cancelled, '
        .. 'failed/rejected->failed, anything else->failed (never completed).'
end

---default (V): failed -> failed.
local function scenario_default(handles)
    local outcome, run_id = probe_state(handles, 'failed')
    if outcome == nil then
        return driver_fail('probe', run_id)
    end
    if outcome == 'failed' then
        return scenario_pass("failed remote task recorded as failed: Fix 2 present")
    end
    ev('FAIL today: task.state=failed recorded as outcome=' .. vim.inspect(outcome))
    ev('file evidence: adapters/a2a.lua lines 63-68 — task_err always nil, outcome always completed')
    return fix_absent(defect_how())
end

---completed-maps-completed (V): completed -> completed.
local function scenario_completed(handles)
    local outcome, run_id = probe_state(handles, 'completed')
    if outcome == nil then
        return driver_fail('probe', run_id)
    end
    if outcome == 'completed' then
        return scenario_pass('completed task recorded as completed (correct by accident: task_err is nil for every call)')
    end
    ev('FAIL today: task.state=completed recorded as outcome=' .. vim.inspect(outcome))
    return fix_absent(defect_how())
end

---rejected-canceled (A): rejected -> failed; canceled -> cancelled.
local function scenario_rejected_canceled(handles)
    local failures = 0
    local outcome, err = probe_state(handles, 'rejected')
    if outcome == nil then
        return driver_fail('probe', err)
    end
    if outcome == 'failed' then
        ev('PASS today: rejected -> failed')
    else
        failures = failures + 1
        ev('FAIL today: task.state=rejected recorded as outcome=' .. vim.inspect(outcome))
    end
    local outcome2, err2 = probe_state(handles, 'canceled')
    if outcome2 == nil then
        return driver_fail('probe', err2)
    end
    if outcome2 == 'cancelled' then
        ev('PASS today: canceled -> cancelled')
    else
        failures = failures + 1
        ev('FAIL today: task.state=canceled recorded as outcome=' .. vim.inspect(outcome2))
    end
    if failures == 0 then
        return scenario_pass('rejected/canceled mapped correctly: Fix 2 present')
    end
    return fix_absent(defect_how())
end

---garbage-nil-double (A): garbage state -> failed (fail-closed, never
---completed); state=nil -> failed; double on_done -> one run.finished.
local function scenario_garbage_nil_double(handles)
    local failures = 0
    local outcome, err = probe_state(handles, 'bogus-state')
    if outcome == nil then
        return driver_fail('probe', err)
    end
    if outcome == 'failed' then
        ev('PASS today: garbage state failed closed')
    else
        failures = failures + 1
        ev('FAIL today: task.state=bogus-state recorded as outcome='
            .. vim.inspect(outcome)
            .. ' (fail-closed demands failed, never completed)')
    end
    local outcome2, err2 = probe_state(handles, nil)
    if outcome2 == nil then
        return driver_fail('probe', err2)
    end
    if outcome2 == 'failed' then
        ev('PASS today: state=nil failed closed')
    else
        failures = failures + 1
        ev('FAIL today: task.state=nil recorded as outcome=' .. vim.inspect(outcome2))
    end
    -- Double invocation for the same task: the second finish on a terminal
    -- run must be a clean no-op — exactly one run.finished, no corruption.
    local run_id, run_err = handles.harness.run({
        workflow = 'gauntlet_a2a_flow',
        goal = 'a2a double-invoke probe',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = 'a2a',
        extensions = { a2a = { agent = 'gauntlet-fake-agent' } },
        timeout_ms = 30000,
    })
    if run_id == nil then
        return driver_fail('run', 'harness.run failed: ' .. tostring(run_err))
    end
    local sub = handles.submitted[#handles.submitted]
    sub.on_done({ state = 'completed', id = 'task-fake-double' })
    sub.on_done({ state = 'completed', id = 'task-fake-double' })
    handles.supervisor.tick(handles.sup, handles.types.now_ns())
    local finished = count_events(handles, run_id, 'run.finished')
    if finished == 1 then
        ev('PASS today: double on_done -> exactly one run.finished (idempotent finish holds)')
    else
        failures = failures + 1
        ev('FAIL today: double on_done -> ' .. finished .. ' run.finished events')
    end
    if failures == 0 then
        return scenario_pass('garbage/nil fail closed and double-invoke is a clean no-op: Fix 2 present')
    end
    return fix_absent(defect_how())
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'default'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return driver_fail('bootstrap', boot_err)
    end
    ev('diver probed: ' .. DIVER_SHA .. ' (main; no Phase-2 branch exists)')
    ev('scenario=' .. scenario)
    ev('driving the REAL adapters/a2a.lua with a stubbed ai.a2a.tasks transport')
    if scenario == 'default' then
        return scenario_default(handles)
    elseif scenario == 'completed-maps-completed' then
        return scenario_completed(handles)
    elseif scenario == 'rejected-canceled' then
        return scenario_rejected_canceled(handles)
    elseif scenario == 'garbage-nil-double' then
        return scenario_garbage_nil_double(handles)
    end
    return driver_fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
end

local ok, verdict = pcall(main)
if not ok then
    ev('lua error: ' .. tostring(verdict))
    verdict = driver_fail('lua-driver', 'unhandled error: ' .. tostring(verdict))
end
-- verdict on the real stdout: in `nvim --headless -l`, Lua print() goes
-- to stderr, but the Rust runner parses the verdict from stdout.
io.stdout:write(vim.json.encode(verdict) .. '\n')
