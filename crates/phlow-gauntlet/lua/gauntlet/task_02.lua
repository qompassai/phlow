-- lua/gauntlet/task_02.lua
-- task-02 driver: budget exhaustion fails closed (Tiger Style)
-- Copyright (C) 2026 Qompass AI, All rights reserved
-- ----------------------------------------
-- Drives diver's ai.harness budget machinery headlessly and proves a
-- budget-exhausted run fails closed: tiny budget -> terminal `failed`
-- state with reason "budget exhausted", a `budget.exhausted` event on the
-- sink, a ledger showing the limit hit, and a harness verdict that is NOT
-- success. Never reports partial success.
--
-- Mock (declared): the run launches through a local `gauntlet_null`
-- adapter registered on a supervisor built from the public harness
-- modules (events/registry/supervisor/budget/verdict). It stands in for a
-- real protocol adapter (acp/a2a/mcp/phlow/rose/herd) because those need
-- live transports unavailable here, and the path under test --
-- supervisor.consume -> budget.exhausted -> tick -> finish('failed') --
-- does not depend on adapter behavior.
--
-- Prints exactly one JSON verdict line and always exits 0; the verdict
-- carries the outcome, not the exit code. Touches nothing outside
-- GAUNTLET_WORK_DIR (and writes no files at all).

local TASK_ID = 'task-02'
local EVIDENCE_MAX = 64
local TURN_LIMIT = 2
local WORKER_TURNS = 5
local TIMEOUT_MS = 60000

local evidence = {} ---@type string[]

---@param fmt string
local function note(fmt, ...)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = string.format(fmt, ...)
    end
end

---@param outcome 'pass'|'fail'
---@param where? string
---@param how? string
local function emit(outcome, where, how)
    local verdict = { id = TASK_ID, outcome = outcome, evidence = evidence }
    if where ~= nil then
        verdict.where = where
        verdict.how = how
    end
    -- NOTE: in `nvim --headless -l`, Lua print() writes to stderr while
    -- io.stdout:write() reaches stdout. The framework parses the verdict
    -- from stdout, so write there explicitly (verified 2026-09-28).
    io.stdout:write(vim.json.encode(verdict) .. '\n')
end

---Minimal test adapter: starts a run with an already-closed handle and
---does nothing else. Budget accounting lives in the supervisor, not here.
local null_adapter = {
    name = 'gauntlet_null',
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
        }
    end,
    start = function(run, sink)
        return { closed = true }, nil
    end,
    cancel = function(handle, reason)
        return true
    end,
    close = function(handle) end,
}

---Build a supervisor wired to the null adapter, from public harness
---modules only. Returns the supervisor, its sink, and an error slot.
---@return table? sup
---@return table? sink
---@return string? err
local function new_test_supervisor()
    local events_mod = require('ai.harness.events')
    local registry_mod = require('ai.harness.registry')
    local supervisor = require('ai.harness.supervisor')
    local sink = events_mod.new_sink()
    local reg = registry_mod.new()
    local ok, reg_err = registry_mod.register_adapter(reg, 'gauntlet_null', null_adapter)
    if not ok then
        return nil, nil, 'register gauntlet_null: ' .. tostring(reg_err)
    end
    local sup, sup_err = supervisor.new({
        registry = reg,
        sink = sink,
        default_timeout_ms = TIMEOUT_MS,
    })
    if sup == nil then
        return nil, nil, tostring(sup_err)
    end
    return sup, sink, nil
end

---Create and start a run with a 2-turn budget on the test supervisor.
---@param sup table
---@param work_dir string
---@return table? run
---@return string? where
---@return string? how
local function start_budget_run(sup, work_dir)
    local supervisor = require('ai.harness.supervisor')
    local run, err = supervisor.create(sup, {
        workflow = 'gauntlet-budget',
        goal = 'budget exhaustion fails closed',
        workspace = work_dir,
        adapter = 'gauntlet_null',
        budget = { turn = TURN_LIMIT },
    })
    if run == nil then
        return nil, 'create', 'run creation failed: ' .. tostring(err)
    end
    local ok, start_err = supervisor.start_run(sup, run.id, 'gauntlet_null')
    if not ok then
        return nil, 'start', 'start_run failed: ' .. tostring(start_err)
    end
    return run, nil, nil
end

---Run the harness verdict machinery: success iff the run completed.
---@param run table
---@return table? verdict
---@return string? err
local function verdict_for(run)
    local verdict_mod = require('ai.harness.verdict')
    return verdict_mod.evaluate(run, {
        {
            kind = 'custom',
            check = function(r)
                return r.state == 'completed'
            end,
        },
    })
end

---Scenario "default": 5 turn-consumes against a 2-turn budget must end in
---terminal `failed` with a `budget.exhausted` event and a non-success
---verdict.
---@param sup table
---@param sink table
---@param work_dir string
---@return boolean ok
---@return string? where
---@return string? how
local function scenario_default(sup, sink, work_dir)
    local supervisor = require('ai.harness.supervisor')
    local types = require('ai.harness.types')
    local budget_mod = require('ai.harness.budget')

    local run, where, how = start_budget_run(sup, work_dir)
    if run == nil then
        return false, where, how
    end
    note('spec.budget={turn=%d}; simulated worker needs %d turns', TURN_LIMIT, WORKER_TURNS)
    note('run started: state=%s', run.state)

    for i = 1, WORKER_TURNS do
        local cok, cerr = supervisor.consume(sup, run.id, 'turn', 1)
        if cok then
            note('consume %d/%d turn: ok', i, WORKER_TURNS)
        else
            note('consume %d/%d turn: REJECTED (%s)', i, WORKER_TURNS, tostring(cerr))
        end
    end

    -- Drive supervision so the budget.exhausted event settles the run.
    supervisor.tick(sup, types.now_ns())
    note('after tick: state=%s', run.state)

    local saw_exhausted = false
    for _, event in ipairs(sink:events(run.id)) do
        if event.kind == 'budget.exhausted' then
            saw_exhausted = true
            note('event budget.exhausted: kind=%s seq=%d', tostring(event.payload.kind), event.seq)
        elseif event.kind == 'run.finished' then
            note(
                'event run.finished: state=%s reason=%s',
                tostring(event.payload.state),
                tostring(event.payload.reason)
            )
        end
    end

    local snap = budget_mod.snapshot(run.budget)
    note(
        'ledger: limits.turn=%s used.turn=%s',
        tostring(snap.limits.turn),
        tostring(snap.used.turn)
    )
    note('terminal: types.is_terminal(%q)=%s', run.state, tostring(types.is_terminal(run.state)))

    local verdict, verr = verdict_for(run)
    if verdict == nil then
        return false, 'verdict', 'verdict.evaluate failed: ' .. tostring(verr)
    end
    note('verdict pass=%s (run never completed)', tostring(verdict.pass))

    if run.state ~= 'failed' then
        return false, 'state', 'expected terminal state "failed", got ' .. tostring(run.state)
    end
    if not saw_exhausted then
        return false, 'event', 'no budget.exhausted event on the sink'
    end
    if verdict.pass then
        return false, 'verdict', 'verdict reported success on a budget-exhausted run'
    end
    if snap.used.turn ~= TURN_LIMIT then
        return false, 'ledger', 'ledger mismatch: used.turn=' .. tostring(snap.used.turn)
    end
    note('PASS: exhausted run terminated failed; verdict is NOT success; no partial success')
    return true
end

---Scenario "zero-budget": a spec with a zero budget must be rejected at
---creation, cleanly, with no run started. Exercises the narrow public
---harness.run() path.
---@param work_dir string
---@return boolean ok
---@return string? where
---@return string? how
local function scenario_zero_budget(work_dir)
    local harness = require('ai.harness')
    local ok, serr = harness.setup({})
    if not ok then
        return false, 'setup', 'harness.setup failed: ' .. tostring(serr)
    end
    local run_id, err = harness.run({
        workflow = 'gauntlet-budget',
        goal = 'zero budget rejected at creation',
        workspace = work_dir,
        adapter = 'gauntlet_null',
        budget = { turn = 0 },
    })
    note('harness.run(budget={turn=0}) -> run_id=%s err=%q', tostring(run_id), tostring(err))
    if run_id ~= nil then
        return false, 'create', 'zero-budget spec was accepted; expected clean rejection'
    end
    if err == nil or not tostring(err):find('budget', 1, true) then
        return false, 'create', 'rejection did not name the budget problem: ' .. tostring(err)
    end
    note('PASS: zero budget rejected cleanly at creation; no run was started')
    return true
end

---Scenario "consume-after-exhaustion": consuming past exhaustion is
---rejected; the run stays exhausted and never resurrects as success.
---@param sup table
---@param sink table
---@param work_dir string
---@return boolean ok
---@return string? where
---@return string? how
local function scenario_consume_after_exhaustion(sup, sink, work_dir)
    local supervisor = require('ai.harness.supervisor')
    local types = require('ai.harness.types')
    local budget_mod = require('ai.harness.budget')

    local run, where, how = start_budget_run(sup, work_dir)
    if run == nil then
        return false, where, how
    end
    for i = 1, TURN_LIMIT + 1 do
        local cok, cerr = supervisor.consume(sup, run.id, 'turn', 1)
        local status = cok and 'ok' or string.format('REJECTED (%s)', tostring(cerr))
        note('consume %d: %s', i, status)
    end
    supervisor.tick(sup, types.now_ns())
    note('state after exhaustion tick: %s', run.state)
    if run.state ~= 'failed' then
        return false, 'state', 'expected failed after exhaustion, got ' .. tostring(run.state)
    end

    for i = 1, 2 do
        local cok, cerr = supervisor.consume(sup, run.id, 'turn', 1)
        local status = cok and 'ok (UNEXPECTED)' or string.format('REJECTED (%s)', tostring(cerr))
        note('post-exhaustion consume %d: %s', i, status)
        if cok then
            return false, 'consume', 'consumption succeeded after exhaustion'
        end
    end
    supervisor.tick(sup, types.now_ns())
    note('state after extra tick: %s', run.state)
    if run.state ~= 'failed' then
        return false, 'state', 'run left failed state: ' .. tostring(run.state)
    end
    local snap = budget_mod.snapshot(run.budget)
    note(
        'ledger: limits.turn=%s used.turn=%s',
        tostring(snap.limits.turn),
        tostring(snap.used.turn)
    )
    local completed = false
    for _, event in ipairs(sink:events(run.id)) do
        if event.kind == 'run.finished' and event.payload.state == 'completed' then
            completed = true
        end
    end
    if completed then
        return false, 'event', 'a run.finished/completed event appeared after exhaustion'
    end
    note('PASS: post-exhaustion consumption rejected; run stays failed, never resurrects')
    return true
end

local function main()
    local diver_lua_dir = vim.env.DIVER_LUA_DIR
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    local scenario = vim.env.GAUNTLET_SCENARIO or 'default'
    if type(diver_lua_dir) ~= 'string' or diver_lua_dir == '' then
        emit('fail', 'env', 'DIVER_LUA_DIR missing or empty')
        return
    end
    if type(work_dir) ~= 'string' or work_dir == '' then
        emit('fail', 'env', 'GAUNTLET_WORK_DIR missing or empty')
        return
    end
    vim.opt.runtimepath:append(diver_lua_dir)
    local harness = require('ai.harness')
    local ok, serr = harness.setup({})
    if not ok then
        emit('fail', 'setup', 'harness.setup failed: ' .. tostring(serr))
        return
    end
    note('task-02 scenario=%s', tostring(scenario))

    local dispatch_ok, dispatch_where, dispatch_how
    if scenario == 'default' then
        local sup, sink, err = new_test_supervisor()
        if sup == nil then
            emit('fail', 'supervisor', tostring(err))
            return
        end
        dispatch_ok, dispatch_where, dispatch_how = scenario_default(sup, sink, work_dir)
    elseif scenario == 'zero-budget' then
        dispatch_ok, dispatch_where, dispatch_how = scenario_zero_budget(work_dir)
    elseif scenario == 'consume-after-exhaustion' then
        local sup, sink, err = new_test_supervisor()
        if sup == nil then
            emit('fail', 'supervisor', tostring(err))
            return
        end
        dispatch_ok, dispatch_where, dispatch_how =
            scenario_consume_after_exhaustion(sup, sink, work_dir)
    else
        emit('fail', 'scenario', 'unknown GAUNTLET_SCENARIO: ' .. tostring(scenario))
        return
    end
    if dispatch_ok then
        emit('pass')
    else
        emit('fail', dispatch_where, dispatch_how)
    end
end

local ok, err = xpcall(main, debug.traceback)
if not ok then
    note('driver raised: %s', tostring(err))
    emit('fail', 'driver', 'unhandled driver error: ' .. tostring(err))
end
