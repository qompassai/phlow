-- task-52 driver: straggler mitigation behavioral probe.
--
-- The design asks for straggler mitigation on the parallel-run
-- supervisor (task-01's fan-out machinery): a worker that is slow but
-- alive should not stall the run — a speculative duplicate is launched
-- after a timeout, whichever finishes first wins, and exactly one
-- result commits (the loser is cancelled, no double side effects).
--
-- This driver exercises the REAL ai.harness.supervisor with a mock
-- adapter whose workers have controllable completion (the driver decides
-- when each worker's model.completed event lands). The simulated clock
-- is stepped explicitly via supervisor.tick, the supervisor's designed
-- promotion path.
--
-- Honest result: the supervisor has no speculation. There is no
-- speculate/duplicate/backup-request API on the supervisor (the probe
-- scans the real export table), no timeout launches a second attempt
-- for a slow run, and the straggler scenario shows the logical run
-- completing at the straggler's pace — p99 is bounded by the slow
-- worker, not by speculation-timeout + fast-path. The one guarantee the
-- design asks for that DOES hold is single-commit, and it holds
-- vacuously: with only one attempt ever launched, at most one result
-- can commit (the probe asserts the finish is idempotent).
--
-- Fail-closed: if speculation APIs ever appear, the no-speculation-api
-- scenario reports where="recon" (premise changed).
--
-- Scenarios via GAUNTLET_SCENARIO (default "straggler"):
--   all-fast           3 fast workers: each exactly 1 run.started,
--                      no duplicates, 3 run.finished
--   straggler          2 fast + 1 slow: clock stepped 10x past the
--                      fast path; the slow run still has exactly 1
--                      run.started (no speculative duplicate), completes
--                      at the straggler's pace
--   single-commit      adversarial: after completion exactly 1
--                      run.finished; a second finish is refused
--   no-speculation-api adversarial: the supervisor export table carries
--                      no speculate/duplicate/straggler/mitigat API
--
-- Prints exactly one JSON verdict line to stdout and always exits 0; the
-- verdict carries the outcome, not the exit code. Writes nothing outside
-- GAUNTLET_WORK_DIR. Never modifies the diver repo.

local ADAPTER_NAME = 'gauntlet_slow'
local EVIDENCE_MAX = 64
local TASK_ID = 'task-52'
local SLOW_FACTOR = 10

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

---@return table? mods
---@return string? err
local function bootstrap()
    local diver_lua_dir = vim.env.DIVER_LUA_DIR
    if type(diver_lua_dir) ~= 'string' or diver_lua_dir == '' then
        return nil, 'DIVER_LUA_DIR is not set'
    end
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    if type(work_dir) ~= 'string' or work_dir == '' then
        return nil, 'GAUNTLET_WORK_DIR is not set'
    end
    vim.opt.runtimepath:append(diver_lua_dir)
    return {
        types = require('ai.harness.types'),
        registry = require('ai.harness.registry'),
        events = require('ai.harness.events'),
        supervisor = require('ai.harness.supervisor'),
        work_dir = work_dir,
    }, nil
end

---A mock adapter whose workers complete only when the driver appends
---their model.completed event. Latency is fully driver-controlled.
local function slow_adapter()
    return {
        name = ADAPTER_NAME,
        probe = function()
            return {
                streaming = false,
                tools = false,
                cancellation = true,
                attachments = false,
                reasoning = false,
            }
        end,
        start = function(run, _sink)
            return { run_id = run.id, generation = run.generation, closed = false }
        end,
        cancel = function(_handle, _reason)
            return true
        end,
        close = function(handle)
            handle.closed = true
        end,
    }
end

---@param mods table
---@return table? ctx {sup=table, sink=table}
---@return string? err
local function new_harness(mods)
    local reg = mods.registry.new()
    local ok, reg_err = mods.registry.register_adapter(reg, ADAPTER_NAME, slow_adapter())
    if not ok then
        return nil, 'adapter registration failed: ' .. tostring(reg_err)
    end
    local sink = mods.events.new_sink()
    local sup, sup_err = mods.supervisor.new({ registry = reg, sink = sink })
    if sup == nil then
        return nil, 'supervisor.new failed: ' .. tostring(sup_err)
    end
    return { sup = sup, sink = sink }, nil
end

---@param mods table
---@param ctx table
---@param tag string
---@return table? run
---@return string? err
local function create_run(mods, ctx, tag)
    local run, err = mods.supervisor.create(ctx.sup, {
        workflow = 'gauntlet-straggler',
        goal = 'prove the straggler behavior: ' .. tag,
        workspace = mods.work_dir,
        adapter = ADAPTER_NAME,
        budget = { turn = 50, tool_call = 200, token = 200000, time_ms = 3600000, byte = 10000000 },
    })
    if run == nil then
        return nil, tostring(err)
    end
    local ok, start_err = mods.supervisor.start_run(ctx.sup, run.id, ADAPTER_NAME)
    if not ok then
        return nil, 'start_run failed: ' .. tostring(start_err)
    end
    return run
end

---Complete one worker's run the way a real adapter would: append
---model.completed to the sink and tick the supervisor.
---@param mods table
---@param ctx table
---@param run table
local function complete_worker(mods, ctx, run)
    ctx.sink:append(run.id, 'model.completed', { outcome = 'completed' }, { source = 'gauntlet' })
    mods.supervisor.tick(ctx.sup, mods.types.now_ns() + 1)
end

---@param ctx table
---@param run_id string
---@param kind string
---@return integer count
local function count_events(ctx, run_id, kind)
    local count = 0
    for _, event in ipairs(ctx.sink:events(run_id)) do
        if event.kind == kind then
            count = count + 1
        end
    end
    return count
end

---All 3 workers fast. Expect: each run exactly 1 run.started (no
---duplicates ever launched), 3 run.finished. The probe passes when the
---supervisor's no-speculation behavior is confirmed, not when
---speculation works — there is none to work.
---@param mods table
---@return table verdict
local function scenario_all_fast(mods)
    local ctx, ctx_err = new_harness(mods)
    if ctx == nil then
        return fail('bootstrap', ctx_err)
    end
    local runs = {}
    for i = 1, 3 do
        local run, run_err = create_run(mods, ctx, 'fast-' .. i)
        if run == nil then
            return fail('create', run_err)
        end
        runs[#runs + 1] = run
    end
    for _, run in ipairs(runs) do
        complete_worker(mods, ctx, run)
    end
    for i, run in ipairs(runs) do
        if run.state ~= 'completed' then
            return fail('verdict', 'fast run ' .. i .. ' is ' .. run.state .. ', want completed')
        end
        local started = count_events(ctx, run.id, 'run.started')
        if started ~= 1 then
            return fail(
                'verdict',
                'fast run ' .. i .. ' has ' .. started .. ' run.started events, want exactly 1'
            )
        end
        local finished = count_events(ctx, run.id, 'run.finished')
        if finished ~= 1 then
            return fail(
                'verdict',
                'fast run ' .. i .. ' has ' .. finished .. ' run.finished events, want exactly 1'
            )
        end
    end
    ev('3 fast workers: each exactly 1 run.started, 3 run.finished — no duplicates launched, none needed')
    return pass()
end

---2 fast workers + 1 straggler. The simulated clock is stepped 10x past
---the fast path; the straggler must still show exactly 1 run.started —
---no speculative duplicate — and the logical completion lands at the
---straggler's pace, not at speculation-timeout + fast-path.
---@param mods table
---@return table verdict
local function scenario_straggler(mods)
    local ctx, ctx_err = new_harness(mods)
    if ctx == nil then
        return fail('bootstrap', ctx_err)
    end
    local fast_a, err_a = create_run(mods, ctx, 'fast-a')
    if fast_a == nil then
        return fail('create', err_a)
    end
    local fast_b, err_b = create_run(mods, ctx, 'fast-b')
    if fast_b == nil then
        return fail('create', err_b)
    end
    local slow, err_c = create_run(mods, ctx, 'straggler')
    if slow == nil then
        return fail('create', err_c)
    end
    local t0 = mods.types.now_ns()
    complete_worker(mods, ctx, fast_a)
    complete_worker(mods, ctx, fast_b)
    local fast_done_ns = mods.types.now_ns() - t0
    ev('fast path: 2 workers completed in ' .. fast_done_ns .. ' simulated ns')
    -- Step the clock SLOW_FACTOR past the fast path with the straggler
    -- still pending. A speculating supervisor would have launched a
    -- duplicate by now; this one must not have.
    local straggler_wait_ns = fast_done_ns * SLOW_FACTOR + 1
    mods.supervisor.tick(ctx.sup, t0 + straggler_wait_ns)
    if slow.state ~= 'running' then
        return fail('verdict', 'straggler left running without completing: ' .. slow.state)
    end
    local started = count_events(ctx, slow.id, 'run.started')
    if started ~= 1 then
        return fail(
            'verdict',
            'straggler has ' .. started .. ' run.started events after 10x the fast path — speculation launched a duplicate'
        )
    end
    ev('after 10x the fast path (' .. straggler_wait_ns .. ' ns): straggler still running, exactly 1 run.started — no speculative duplicate launched')
    -- The straggler finally finishes: the logical run completes at the
    -- straggler's pace.
    complete_worker(mods, ctx, slow)
    local total_ns = mods.types.now_ns() - t0
    if slow.state ~= 'completed' then
        return fail('verdict', 'straggler is ' .. slow.state .. ' after completing, want completed')
    end
    ev('straggler completed at +' .. total_ns .. ' ns: p99 is bounded by the slow worker, NOT by speculation-timeout + fast-path — the design\'s latency bound has no seam')
    return pass()
end

---Adversarial: exactly one result may commit per logical task. With no
---speculation there is only one attempt, so the guarantee holds
---vacuously — but the probe asserts it against real state: exactly 1
---run.finished, and a second finish is refused as an invalid transition.
---@param mods table
---@return table verdict
local function scenario_single_commit(mods)
    local ctx, ctx_err = new_harness(mods)
    if ctx == nil then
        return fail('bootstrap', ctx_err)
    end
    local run, run_err = create_run(mods, ctx, 'commit')
    if run == nil then
        return fail('create', run_err)
    end
    complete_worker(mods, ctx, run)
    if run.state ~= 'completed' then
        return fail('verdict', 'run is ' .. run.state .. ', want completed')
    end
    local finished = count_events(ctx, run.id, 'run.finished')
    if finished ~= 1 then
        return fail('verdict', 'run.finished fired ' .. finished .. 'x, want exactly 1')
    end
    ev('exactly 1 run.finished: one result committed for the logical task')
    -- The adversarial half: a duplicate completion (the "loser" of a
    -- race that cannot happen here) is refused.
    local ok, fin_err = mods.supervisor.finish(ctx.sup, run.id, 'completed', 'duplicate commit')
    if ok then
        return fail('verdict', 'second finish was accepted: double commit possible')
    end
    ev('second finish refused: ' .. tostring(fin_err))
    if run.state ~= 'completed' then
        return fail('verdict', 'refused re-finish moved the run: ' .. run.state)
    end
    ev('single-commit holds (vacuously: one attempt was ever launched, and the finish is idempotent)')
    return pass()
end

---Adversarial: the supervisor export table carries no speculation API.
---Fail-closed: if one appears, the premise changed.
---@param mods table
---@return table verdict
local function scenario_no_speculation_api(mods)
    local needles = { 'speculat', 'duplicat', 'straggler', 'mitigat', 'hedg', 'backup' }
    ev('speculation needles: ' .. table.concat(needles, ', '))
    local names = {}
    for key, value in pairs(mods.supervisor) do
        if type(value) == 'function' and type(key) == 'string' then
            names[#names + 1] = key
        end
    end
    table.sort(names)
    ev('supervisor exports: ' .. table.concat(names, ', '))
    local hits = {}
    for _, name in ipairs(names) do
        local lower = name:lower()
        for _, needle in ipairs(needles) do
            if lower:find(needle, 1, true) then
                hits[#hits + 1] = 'ai.harness.supervisor.' .. name
                break
            end
        end
    end
    if #hits > 0 then
        return fail(
            'recon',
            'speculation machinery now exists (' .. table.concat(hits, ', ') .. '); probe outdated'
        )
    end
    ev('zero speculation-API hits: no speculate/duplicate/backup API on the supervisor — the straggler has nothing to trigger')
    return pass()
end

local function main()
    local mods, boot_err = bootstrap()
    if mods == nil then
        return fail('bootstrap', boot_err)
    end
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'straggler'
    end
    ev('scenario=' .. scenario)
    if scenario == 'all-fast' then
        return scenario_all_fast(mods)
    elseif scenario == 'straggler' then
        return scenario_straggler(mods)
    elseif scenario == 'single-commit' then
        return scenario_single_commit(mods)
    elseif scenario == 'no-speculation-api' then
        return scenario_no_speculation_api(mods)
    end
    return fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
end

local ok, verdict = pcall(main)
if not ok then
    ev('lua error: ' .. tostring(verdict))
    verdict = fail('lua-driver', 'unhandled error: ' .. tostring(verdict))
end
-- Verdict on the real stdout: in `nvim --headless -l`, Lua print() goes
-- to stderr, but the Rust runner parses the verdict from stdout.
io.stdout:write(vim.json.encode(verdict) .. '\n')
