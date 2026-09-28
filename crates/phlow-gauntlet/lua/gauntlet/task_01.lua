-- crates/phlow-gauntlet/lua/gauntlet/task_01.lua
-- task-01 driver: fan-out/fan-in verdict aggregation over diver's ai.harness.
-- Copyright (C) 2026 Qompass AI. MIT OR Apache-2.0.
-- ---------------------------------------------------------------------------
-- Launches 5 harness runs against a fake `gauntlet` adapter registered at
-- runtime (see docs/task-01.md for why the fake, not the herd adapter),
-- polls with a bounded deadline until all 5 reach terminal states, then
-- fans in: reads terminal states/reasons from the sink's run.finished
-- events and computes aggregate = pass iff all 5 completed.
--
-- Prints EXACTLY ONE JSON verdict line and always exits 0; the verdict
-- carries the outcome, not the exit code. Writes nothing outside
-- GAUNTLET_WORK_DIR. Never modifies the diver repo.
--
-- Env: DIVER_LUA_DIR (diver/lua), GAUNTLET_WORK_DIR (scratch),
--      GAUNTLET_SCENARIO ("default" | "all-succeed" | "adapter-raises").

local RUN_COUNT = 5
local RUN_TIMEOUT_MS = 60000
local POLL_TIMEOUT_MS = 15000
local POLL_STEP_MS = 25
local EVIDENCE_LINES_MAX = 64
local ADAPTER_NAME = 'gauntlet'
local TASK_ID = 'task-01'

---Fake-adapter shared state: the id of the run whose start() raised, so the
---driver can finish that run as failed instead of losing it in 'queued'.
---(harness.run() cannot return the id when start() raises.)
local FAKE = { last_raised_run_id = nil }

---@param run table harness run (fields .id, .extensions)
---@param sink table event sink
---@return table? handle
local function fake_start(run, sink)
    assert(run ~= nil, 'fake start: run required')
    assert(sink ~= nil, 'fake start: sink required')
    local ext = (run.extensions ~= nil and run.extensions.gauntlet) or {}
    local behavior = ext.behavior or 'succeed'
    if behavior == 'raise' then
        FAKE.last_raised_run_id = run.id
        error('gauntlet fake adapter: start() raised by scenario')
    end
    local handle = { adapter = ADAPTER_NAME, run_id = run.id, closed = false }
    if behavior == 'succeed' then
        sink:append(run.id, 'model.completed', { outcome = 'completed' }, { source = ADAPTER_NAME })
    elseif behavior == 'fail' then
        sink:append(run.id, 'model.completed', {
            outcome = 'failed',
            error = 'gauntlet fake worker: reported failure',
        }, { source = ADAPTER_NAME })
    elseif behavior ~= 'pending' then
        error('gauntlet fake adapter: unknown behavior ' .. tostring(behavior))
    end
    -- 'pending': no completion event; the run stays running until cancelled.
    return handle
end

---The fake adapter: a declared test double registered at runtime. It drives
---the REAL harness lifecycle (registry -> supervisor.launch -> transition ->
---tick -> drain_completions -> finish); only the worker side is faked.
---@return table adapter honoring the AiHarnessAdapter contract
local function fake_adapter()
    return {
        name = ADAPTER_NAME,
        probe = function()
            return {
                available = true,
                streaming = false,
                cancellation = true,
                resume = false,
                permissions = false,
                artifacts = false,
                remote = false,
                tools = false,
                wired_start = true,
            }
        end,
        start = fake_start,
        cancel = function(handle, _reason)
            assert(handle ~= nil, 'fake cancel: handle required')
            handle.cancelled = true
            return true
        end,
        close = function(handle)
            assert(handle ~= nil, 'fake close: handle required')
            handle.closed = true
        end,
    }
end

---@return table? env {diver_lua_dir: string, work_dir: string, scenario: string}
---@return string? err
local function read_env()
    local diver_lua_dir = vim.env.DIVER_LUA_DIR
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    if type(diver_lua_dir) ~= 'string' or diver_lua_dir == '' then
        return nil, 'DIVER_LUA_DIR is missing or empty'
    end
    if type(work_dir) ~= 'string' or work_dir == '' then
        return nil, 'GAUNTLET_WORK_DIR is missing or empty'
    end
    local scenario = vim.env.GAUNTLET_SCENARIO
    if scenario == nil or scenario == '' then
        scenario = 'default'
    end
    return { diver_lua_dir = diver_lua_dir, work_dir = work_dir, scenario = scenario }
end

---@param scenario string
---@return table[]? plan list of {behavior: string, purpose: string}
---@return string? err
local function scenario_plan(scenario)
    if scenario == 'default' then
        return {
            { behavior = 'succeed', purpose = 'worker completes' },
            { behavior = 'succeed', purpose = 'worker completes' },
            { behavior = 'succeed', purpose = 'worker completes' },
            { behavior = 'fail', purpose = 'worker reports failure' },
            { behavior = 'pending', purpose = 'cancelled mid-run by driver' },
        }
    elseif scenario == 'all-succeed' then
        return {
            { behavior = 'succeed', purpose = 'worker completes' },
            { behavior = 'succeed', purpose = 'worker completes' },
            { behavior = 'succeed', purpose = 'worker completes' },
            { behavior = 'succeed', purpose = 'worker completes' },
            { behavior = 'succeed', purpose = 'worker completes' },
        }
    elseif scenario == 'adapter-raises' then
        return {
            { behavior = 'succeed', purpose = 'worker completes' },
            { behavior = 'succeed', purpose = 'worker completes' },
            { behavior = 'succeed', purpose = 'worker completes' },
            { behavior = 'succeed', purpose = 'worker completes' },
            { behavior = 'raise', purpose = 'adapter start() raises' },
        }
    end
    return nil, 'unknown GAUNTLET_SCENARIO: ' .. tostring(scenario)
end

---Fan out: launch one run per plan item. A start() that raises is caught;
---the run is finished as failed via supervisor.finish so it is counted,
---never lost in 'queued'.
---@param harness table
---@param sup table supervisor state
---@param work_dir string
---@param plan table[]
---@return table[]? runs list of {id: string, behavior: string, raised: string?}
---@return string? err
local function fan_out(harness, sup, work_dir, plan)
    local supervisor = require('ai.harness.supervisor')
    local runs = {}
    for i, item in ipairs(plan) do
        local spec = {
            adapter = ADAPTER_NAME,
            workflow = 'gauntlet-task-01',
            goal = 'task-01 worker ' .. i .. ': ' .. item.purpose,
            workspace = work_dir,
            timeout_ms = RUN_TIMEOUT_MS,
            extensions = { gauntlet = { behavior = item.behavior } },
        }
        local ok_run, id_or_err = pcall(harness.run, spec)
        if ok_run and type(id_or_err) == 'string' then
            runs[#runs + 1] = { id = id_or_err, behavior = item.behavior }
        elseif not ok_run then
            local raised_id = FAKE.last_raised_run_id
            FAKE.last_raised_run_id = nil
            if type(raised_id) ~= 'string' then
                return nil,
                    'fan-out: start() raised but no run id recorded: ' .. tostring(id_or_err)
            end
            local reason = 'adapter start() raised: ' .. tostring(id_or_err)
            local ok_f, err_f = supervisor.finish(sup, raised_id, 'failed', reason)
            if not ok_f then
                return nil,
                    'fan-out: could not fail raised run ' .. raised_id .. ': ' .. tostring(err_f)
            end
            runs[#runs + 1] = {
                id = raised_id,
                behavior = item.behavior,
                raised = tostring(id_or_err),
            }
        else
            return nil, 'fan-out: run ' .. i .. ' failed to launch: ' .. tostring(id_or_err)
        end
    end
    return runs
end

---Poll with a bounded deadline until every run is terminal. In the default
---scenario the 'pending' run is cancelled mid-run after the first tick.
---@param harness table
---@param sup table supervisor state
---@param runs table[]
---@param scenario string
---@return string? err
local function poll_until_terminal(harness, sup, runs, scenario)
    local types = require('ai.harness.types')
    local supervisor = require('ai.harness.supervisor')
    local function all_terminal()
        for _, r in ipairs(runs) do
            local run = supervisor.get(sup, r.id)
            if run == nil or not types.is_terminal(run.state) then
                return false
            end
        end
        return true
    end
    local deadline_ns = types.now_ns() + POLL_TIMEOUT_MS * 1000000
    local cancelled = false
    while true do
        supervisor.tick(sup, types.now_ns())
        if scenario == 'default' and not cancelled then
            cancelled = true
            for _, r in ipairs(runs) do
                if r.behavior == 'pending' then
                    local ok_c, err_c = harness.cancel(r.id, 'gauntlet: driver mid-run cancel')
                    if not ok_c then
                        return 'fan-out: mid-run cancel failed for '
                            .. r.id
                            .. ': '
                            .. tostring(err_c)
                    end
                    r.cancelled_by_driver = true
                end
            end
        end
        if all_terminal() then
            return nil
        end
        if types.now_ns() >= deadline_ns then
            return 'fan-out: poll deadline ('
                .. POLL_TIMEOUT_MS
                .. ' ms) exceeded before all runs terminal'
        end
        vim.wait(POLL_STEP_MS)
    end
end

---Fan in: read terminal states/reasons from the sink's run.finished events
---and compute aggregate = pass iff all RUN_COUNT runs completed.
---@param sup table supervisor state
---@param runs table[]
---@return table agg {counts: table, finished_total: integer, lost: integer,
---                   aggregate_pass: boolean, lines: string[]}
local function fan_in(sup, runs)
    local supervisor = require('ai.harness.supervisor')
    local counts = { completed = 0, failed = 0, cancelled = 0, timed_out = 0, interrupted = 0 }
    local lines = {}
    local finished_total = 0
    local lost = 0
    for i, r in ipairs(runs) do
        local run = supervisor.get(sup, r.id)
        local state = run ~= nil and run.state or 'missing'
        local finished_state = '-'
        local finished_reason = '-'
        local finished_n = 0
        for _, ev in ipairs(sup.sink:events(r.id)) do
            if ev.kind == 'run.finished' then
                finished_n = finished_n + 1
                if type(ev.payload) == 'table' then
                    finished_state = tostring(ev.payload.state or '-')
                    finished_reason = tostring(ev.payload.reason or '-')
                end
            end
        end
        finished_total = finished_total + finished_n
        counts[state] = (counts[state] or 0) + 1
        if finished_n ~= 1 or state == 'missing' then
            lost = lost + 1
        end
        local extra = r.raised ~= nil and ' raised=yes' or ''
        lines[#lines + 1] = string.format(
            'run[%d] behavior=%s state=%s finished=%s reason=%s%s',
            i,
            r.behavior,
            state,
            finished_state,
            finished_reason,
            extra
        )
    end
    local aggregate_pass = finished_total == RUN_COUNT
        and lost == 0
        and counts.completed == RUN_COUNT
    return {
        counts = counts,
        finished_total = finished_total,
        lost = lost,
        aggregate_pass = aggregate_pass,
        lines = lines,
    }
end

---@return string outcome 'pass'|'fail'
---@return string? where set on fail
---@return string? how set on fail
---@return string[] evidence
local function run_attempt()
    local env, env_err = read_env()
    if env == nil then
        return 'fail', 'env', env_err, {}
    end
    vim.opt.runtimepath:append(env.diver_lua_dir)
    local harness = require('ai.harness')
    local ok_setup, setup_err = harness.setup({})
    if not ok_setup then
        return 'fail', 'setup', 'harness.setup failed: ' .. tostring(setup_err), {}
    end
    local registry = require('ai.harness.registry')
    local ok_reg, reg_err =
        registry.register_adapter(harness._state.registry, ADAPTER_NAME, fake_adapter())
    if not ok_reg then
        return 'fail', 'setup', 'fake adapter registration failed: ' .. tostring(reg_err), {}
    end
    local plan, plan_err = scenario_plan(env.scenario)
    if plan == nil then
        return 'fail', 'scenario', plan_err, {}
    end
    local sup = harness._state.supervisor
    local evidence = {
        'scenario='
            .. env.scenario
            .. ' adapter='
            .. ADAPTER_NAME
            .. ' (fake, runtime-registered test double)',
        'sink events before fan-out: ' .. sup.sink:count(),
    }
    local runs, fan_err = fan_out(harness, sup, env.work_dir, plan)
    if runs == nil then
        return 'fail', 'fan-out', fan_err, evidence
    end
    evidence[#evidence + 1] = 'fan-out: launched ' .. #runs .. '/' .. RUN_COUNT .. ' runs'
    local poll_err = poll_until_terminal(harness, sup, runs, env.scenario)
    if poll_err ~= nil then
        return 'fail', 'fan-out', poll_err, evidence
    end
    local agg = fan_in(sup, runs)
    for _, line in ipairs(agg.lines) do
        evidence[#evidence + 1] = line
    end
    local c = agg.counts
    local terminal = (c.completed or 0) + (c.failed or 0) + (c.cancelled or 0) + (c.timed_out or 0)
        + (c.interrupted or 0)
    evidence[#evidence + 1] = string.format(
        'fan-in: runs=%d terminal=%d/%d finished_events=%d lost=%d'
            .. ' completed=%d failed=%d cancelled=%d aggregate=%s',
        #runs,
        terminal,
        RUN_COUNT,
        agg.finished_total,
        agg.lost,
        c.completed or 0,
        c.failed or 0,
        c.cancelled or 0,
        agg.aggregate_pass and 'pass' or 'fail'
    )
    for _, r in ipairs(runs) do
        if r.raised ~= nil then
            evidence[#evidence + 1] = 'raise: adapter start() raised;'
                .. ' driver finished the run as failed via supervisor.finish'
            break
        end
    end
    local accounting_ok = #runs == RUN_COUNT
        and terminal == RUN_COUNT
        and agg.finished_total == RUN_COUNT
        and agg.lost == 0
    if not accounting_ok then
        return 'fail', 'fan-in', 'run accounting mismatch: a run was lost or non-terminal', evidence
    end
    evidence[#evidence + 1] = 'verdict: aggregation machinery correct;'
        .. ' aggregate rule = pass iff all 5 completed'
    return 'pass', nil, nil, evidence
end

---@param outcome string
---@param where string?
---@param how string?
---@param evidence string[]
local function emit(outcome, where, how, evidence)
    local bounded = {}
    for i, line in ipairs(evidence) do
        if i > EVIDENCE_LINES_MAX then
            break
        end
        bounded[#bounded + 1] = line
    end
    local verdict = { id = TASK_ID, outcome = outcome, evidence = bounded }
    if outcome == 'fail' then
        verdict.where = where or 'driver'
        verdict.how = how or 'unspecified driver failure'
    end
    -- print() writes to stderr under `nvim --headless -l`; the framework
    -- reads the verdict line from stdout, so write it explicitly.
    io.stdout:write(vim.json.encode(verdict) .. '\n')
    io.stdout:flush()
end

local function main()
    local results = { xpcall(run_attempt, debug.traceback) }
    if not results[1] then
        emit('fail', 'driver', 'unexpected Lua error: ' .. tostring(results[2]), {})
    else
        emit(results[2], results[3], results[4], results[5] or {})
    end
    os.exit(0)
end

main()
