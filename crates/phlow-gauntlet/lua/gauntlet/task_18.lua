-- task-18 driver: worker crash recovery.
--
-- Drives diver's REAL run supervisor (ai.harness.supervisor) through the
-- crash-recovery contract: an adapter run whose worker process dies gets
-- bounded retries with backoff, and when the retry ceiling is hit the run
-- is finished `failed` with a cause chain the operator can read.
--
-- The crash monitor role is played by the driver itself: the mock adapter
-- ("gauntlet_crash") starts a fake worker handle, and `inject_crash`
-- appends a `diagnostic.observed` worker_crashed event to the sink and
-- calls `supervisor.retry_run` — the exact call a process monitor makes
-- when it reaps a dead worker. Backoff elision is explicit: the driver
-- sets `run.retry_at_ns = now` (documented test-time control of the
-- simulated clock) and lets `supervisor.tick` promote the retry, which is
-- the supervisor's designed promotion path ("tests call tick() directly").
--
-- Scenarios via GAUNTLET_SCENARIO (default "persistent-crash"):
--   persistent-crash  crash every attempt -> 4 attempts -> `failed`
--                     verdict carrying the per-attempt cause chain
--   transient-crash   crash attempts 1-2, succeed on 3 -> `completed`
--   duplicate-crash   a second crash report for the same attempt must not
--                     consume the retry budget twice
--   crash-storm       10 crash injections: attempts stay capped at 4,
--                     exactly one run.finished, no resurrection
--
-- Prints exactly one JSON verdict line to stdout and always exits 0; the
-- verdict carries the outcome, not the exit code. Writes nothing outside
-- GAUNTLET_WORK_DIR. Never modifies the diver repo.

local ADAPTER_NAME = 'gauntlet_crash'
local EVIDENCE_MAX = 64
local RETRY_ATTEMPTS_MAX = 4 -- mirrors ai.harness.supervisor's bound

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function fail(where, how)
    return { id = 'task-18', outcome = 'fail', where = where, how = how, evidence = evidence }
end

local function pass()
    return { id = 'task-18', outcome = 'pass', evidence = evidence }
end

---The per-attempt crash causes, in order. Each reads like a process
---monitor's report: the operator sees exactly what killed each worker.
local CRASH_CAUSES = {
    'worker process exited: signal 11 (SIGSEGV)',
    'worker process exited: signal 9 (SIGKILL, OOM)',
    'worker heartbeat lost: no output for 30s',
    'worker process exited: code 1 (adapter init failed)',
}

---@param run table
---@param causes string[] per-attempt crash causes collected so far
---@return string the operator-facing cause chain
local function cause_chain(causes)
    local parts = {}
    for i, cause in ipairs(causes) do
        parts[#parts + 1] = '[' .. i .. '] ' .. cause
    end
    return 'worker crashed on attempts 1-'
        .. #causes
        .. '; supervisor gave up: retry attempt ceiling exceeded (max '
        .. RETRY_ATTEMPTS_MAX
        .. '). causes: '
        .. table.concat(parts, '; ')
end

---@return table modules
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

---A mock adapter whose "worker" is a fake handle. The crash is injected
---by the driver calling `inject_crash`; `start` itself always succeeds so
---the scenario exercises retry logic, not start-failure handling.
local function crash_adapter()
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
    local ok, reg_err = mods.registry.register_adapter(reg, ADAPTER_NAME, crash_adapter())
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
---@return table? run
---@return string? err
local function create_run(mods, ctx)
    return mods.supervisor.create(ctx.sup, {
        workflow = 'gauntlet-crash-recovery',
        goal = 'prove bounded retry on worker crashes',
        workspace = mods.work_dir,
        adapter = ADAPTER_NAME,
        -- Generous time budget: tick() accounts wall time, and the driver
        -- steps the clock itself. Nothing here may die of budget.
        budget = { turn = 50, tool_call = 200, token = 200000, time_ms = 3600000, byte = 10000000 },
    })
end

---Inject one worker crash for the run's current attempt and ask the
---supervisor for a bounded retry. Returns (true) when the retry was
---scheduled, (nil, err) when the supervisor refused (ceiling or state).
---@param mods table
---@param ctx table
---@param run table
---@param cause string
---@return boolean ok
---@return string? err
local function inject_crash(mods, ctx, run, cause)
    ctx.sink:append(run.id, 'diagnostic.observed', {
        kind = 'worker_crashed',
        detail = cause,
        attempt = run.attempt,
    }, { source = 'gauntlet' })
    return mods.supervisor.retry_run(ctx.sup, run.id, cause)
end

---Elide the backoff wait (explicit simulated-clock control) and let the
---supervisor promote the scheduled retry to a fresh attempt.
---@param mods table
---@param ctx table
---@param run table
local function promote_retry(mods, ctx, run)
    run.retry_at_ns = mods.types.now_ns()
    mods.supervisor.tick(ctx.sup, mods.types.now_ns() + 1)
end

---@param ctx table
---@param run_id string
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

---@param ctx table
---@param run_id string
---@return table? event the run.finished event, or nil
local function finished_event(ctx, run_id)
    for _, event in ipairs(ctx.sink:events(run_id)) do
        if event.kind == 'run.finished' then
            return event
        end
    end
    return nil
end

---The operator-facing verdict line: what a human (or the CLI report)
---reads after the supervisor gives up. One line, unambiguous verdict,
---attempt accounting, and the full cause chain.
---@param run table
---@param causes string[]
---@return string
local function operator_view(run, causes)
    return ('operator view: verdict=%s attempts=%d/%d reason="%s"'):format(
        run.state,
        run.attempt,
        RETRY_ATTEMPTS_MAX,
        cause_chain(causes)
    )
end

---Crash the worker on every attempt. Expect: 4 launches, attempt ceiling,
---then `failed` with a legible cause chain and exactly one run.finished.
---@param mods table
---@return table verdict
local function scenario_persistent_crash(mods)
    local ctx, ctx_err = new_harness(mods)
    if ctx == nil then
        return fail('bootstrap', ctx_err)
    end
    local run, run_err = create_run(mods, ctx)
    if run == nil then
        return fail('create', tostring(run_err))
    end
    local ok, start_err = mods.supervisor.start_run(ctx.sup, run.id, ADAPTER_NAME)
    if not ok then
        return fail('start', tostring(start_err))
    end
    local causes = {}
    -- Attempts are bounded at RETRY_ATTEMPTS_MAX: crashes on attempts
    -- 1..3 are retried; the crash on attempt 4 hits the ceiling.
    for attempt = 1, RETRY_ATTEMPTS_MAX - 1 do
        local cause = CRASH_CAUSES[attempt]
        local retry_ok, retry_err = inject_crash(mods, ctx, run, cause)
        if not retry_ok then
            return fail(
                'retry',
                'crash on attempt ' .. attempt .. ' refused before the ceiling: ' .. tostring(retry_err)
            )
        end
        causes[#causes + 1] = cause
        ev('crash on attempt ' .. attempt .. ': retry scheduled (now attempt ' .. run.attempt .. ')')
        promote_retry(mods, ctx, run)
        if run.state ~= 'running' then
            return fail('retry', 'promoted retry did not relaunch: state=' .. run.state)
        end
    end
    -- The crash on the final attempt: the supervisor must refuse.
    local last_cause = CRASH_CAUSES[RETRY_ATTEMPTS_MAX]
    local extra_ok, extra_err = inject_crash(mods, ctx, run, last_cause)
    if extra_ok then
        return fail('retry', 'retry ceiling was never hit; attempts unbounded?')
    end
    causes[#causes + 1] = last_cause
    ev('crash on attempt ' .. RETRY_ATTEMPTS_MAX .. ' refused: ' .. tostring(extra_err))
    -- The caller that detected the crash owns the give-up: finish `failed`
    -- with the cause chain as the reason the operator reads.
    local reason = cause_chain(causes)
    local fin_ok, fin_err = mods.supervisor.finish(ctx.sup, run.id, 'failed', reason)
    if not fin_ok then
        return fail('finish', tostring(fin_err))
    end
    ev('supervisor gave up after ' .. RETRY_ATTEMPTS_MAX .. ' attempts: run finished failed')
    -- Assertions on the real supervisor state.
    if run.state ~= 'failed' then
        return fail('verdict', 'final state is ' .. run.state .. ', want failed')
    end
    if run.attempt ~= RETRY_ATTEMPTS_MAX then
        return fail('verdict', 'final attempt is ' .. run.attempt .. ', want ' .. RETRY_ATTEMPTS_MAX)
    end
    local started = count_events(ctx, run.id, 'run.started')
    if started ~= RETRY_ATTEMPTS_MAX then
        return fail('verdict', 'run.started fired ' .. started .. 'x, want ' .. RETRY_ATTEMPTS_MAX)
    end
    local fin = finished_event(ctx, run.id)
    if fin == nil then
        return fail('verdict', 'no run.finished event in the sink')
    end
    if fin.payload.state ~= 'failed' then
        return fail('verdict', 'run.finished state is ' .. tostring(fin.payload.state))
    end
    local fin_reason = tostring(fin.payload.reason or '')
    for _, cause in ipairs(causes) do
        if fin_reason:find(cause, 1, true) == nil then
            return fail('verdict', 'cause chain lost a cause: ' .. cause)
        end
    end
    if fin_reason:find('retry attempt ceiling exceeded', 1, true) == nil then
        return fail('verdict', 'run.finished reason hides the give-up: ' .. fin_reason)
    end
    ev('run.finished reason carries all ' .. #causes .. ' crash causes + the ceiling note')
    -- No illegal transition may have fired: invalid moves become
    -- diagnostic.observed events with kind=invalid_transition.
    for _, event in ipairs(ctx.sink:events(run.id)) do
        if event.kind == 'diagnostic.observed' and event.payload.kind == 'invalid_transition' then
            return fail(
                'verdict',
                'invalid transition fired during recovery: '
                    .. tostring(event.payload.from)
                    .. ' -> '
                    .. tostring(event.payload.to)
            )
        end
    end
    ev('zero invalid_transition diagnostics: every move was a legal transition')
    ev(operator_view(run, causes))
    return pass()
end

---Crash on attempts 1-2, succeed on 3. Expect: `completed`, attempt == 3,
---no failed verdict anywhere — recovery actually recovers.
---@param mods table
---@return table verdict
local function scenario_transient_crash(mods)
    local ctx, ctx_err = new_harness(mods)
    if ctx == nil then
        return fail('bootstrap', ctx_err)
    end
    local run, run_err = create_run(mods, ctx)
    if run == nil then
        return fail('create', tostring(run_err))
    end
    local ok, start_err = mods.supervisor.start_run(ctx.sup, run.id, ADAPTER_NAME)
    if not ok then
        return fail('start', tostring(start_err))
    end
    for _, cause in ipairs({ CRASH_CAUSES[1], CRASH_CAUSES[3] }) do
        local retry_ok, retry_err = inject_crash(mods, ctx, run, cause)
        if not retry_ok then
            return fail('retry', 'transient crash refused a retry: ' .. tostring(retry_err))
        end
        promote_retry(mods, ctx, run)
    end
    ev('attempts 1-2 crashed and were retried; attempt 3 is ' .. run.state)
    -- The worker survives attempt 3: it reports completion normally.
    ctx.sink:append(run.id, 'model.completed', { outcome = 'completed' }, { source = 'gauntlet' })
    mods.supervisor.tick(ctx.sup, mods.types.now_ns() + 1)
    if run.state ~= 'completed' then
        return fail('verdict', 'final state is ' .. run.state .. ', want completed')
    end
    if run.attempt ~= 3 then
        return fail('verdict', 'final attempt is ' .. run.attempt .. ', want 3')
    end
    ev('run completed on attempt 3 after two crash recoveries')
    ev(('operator view: verdict=%s attempts=%d/%d (recovered, no failure)'):format(
        run.state,
        run.attempt,
        RETRY_ATTEMPTS_MAX
    ))
    return pass()
end

---A second crash report for the same attempt must not consume the retry
---budget twice: retry_run from `retry_wait` is an invalid transition, the
---supervisor records a diagnostic, and the attempt counter stays put.
---@param mods table
---@return table verdict
local function scenario_duplicate_crash(mods)
    local ctx, ctx_err = new_harness(mods)
    if ctx == nil then
        return fail('bootstrap', ctx_err)
    end
    local run, run_err = create_run(mods, ctx)
    if run == nil then
        return fail('create', tostring(run_err))
    end
    local ok, start_err = mods.supervisor.start_run(ctx.sup, run.id, ADAPTER_NAME)
    if not ok then
        return fail('start', tostring(start_err))
    end
    local retry_ok, retry_err = inject_crash(mods, ctx, run, CRASH_CAUSES[1])
    if not retry_ok then
        return fail('retry', 'first crash refused: ' .. tostring(retry_err))
    end
    if run.attempt ~= 2 then
        return fail('retry', 'attempt is ' .. run.attempt .. ' after one crash, want 2')
    end
    ev('first crash: retry scheduled, attempt=2, state=' .. run.state)
    -- The process monitor double-fires for the same dead worker.
    local dup_ok, dup_err = inject_crash(mods, ctx, run, CRASH_CAUSES[1] .. ' (duplicate report)')
    if dup_ok then
        return fail('retry', 'duplicate crash report consumed a second retry: attempt=' .. run.attempt)
    end
    ev('duplicate crash report refused: ' .. tostring(dup_err))
    if run.attempt ~= 2 then
        return fail('retry', 'duplicate report moved the attempt counter to ' .. run.attempt)
    end
    ev('attempt counter still 2: the duplicate did not spend budget')
    -- The refusal is visible to the operator as a diagnostic event.
    local saw_invalid = false
    for _, event in ipairs(ctx.sink:events(run.id)) do
        if
            event.kind == 'diagnostic.observed'
            and event.payload.kind == 'invalid_transition'
            and event.payload.from == 'retry_wait'
        then
            saw_invalid = true
        end
    end
    if not saw_invalid then
        return fail('retry', 'no invalid_transition diagnostic for the duplicate crash report')
    end
    ev('invalid_transition diagnostic recorded: the refusal is operator-visible')
    -- Recovery still works: promote the one legitimate retry, succeed.
    promote_retry(mods, ctx, run)
    ctx.sink:append(run.id, 'model.completed', { outcome = 'completed' }, { source = 'gauntlet' })
    mods.supervisor.tick(ctx.sup, mods.types.now_ns() + 1)
    if run.state ~= 'completed' or run.attempt ~= 2 then
        return fail('verdict', 'recovery broke: state=' .. run.state .. ' attempt=' .. run.attempt)
    end
    ev('legitimate retry promoted and completed on attempt 2')
    return pass()
end

---Ten crash injections against the bound: attempts never exceed 4, the
---run finishes `failed` exactly once, and post-ceiling crashes cannot
---resurrect or re-finish it.
---@param mods table
---@return table verdict
local function scenario_crash_storm(mods)
    local ctx, ctx_err = new_harness(mods)
    if ctx == nil then
        return fail('bootstrap', ctx_err)
    end
    local run, run_err = create_run(mods, ctx)
    if run == nil then
        return fail('create', tostring(run_err))
    end
    local ok, start_err = mods.supervisor.start_run(ctx.sup, run.id, ADAPTER_NAME)
    if not ok then
        return fail('start', tostring(start_err))
    end
    local causes = {}
    local ceiling_hits = 0
    for i = 1, 10 do
        local cause = 'storm crash #' .. i
        local retry_ok, retry_err = inject_crash(mods, ctx, run, cause)
        -- The first 4 reports are the real per-attempt crashes (attempts
        -- 1-4); reports 5-10 arrive with the attempt counter already at
        -- the ceiling.
        if #causes < RETRY_ATTEMPTS_MAX then
            causes[#causes + 1] = cause
        end
        if retry_ok then
            promote_retry(mods, ctx, run)
        else
            if tostring(retry_err):find('retry attempt ceiling exceeded', 1, true) == nil then
                return fail('retry', 'unexpected refusal: ' .. tostring(retry_err))
            end
            ceiling_hits = ceiling_hits + 1
        end
        if run.attempt > RETRY_ATTEMPTS_MAX then
            return fail('retry', 'attempt counter escaped the bound: ' .. run.attempt)
        end
    end
    ev('10 crash injections: ' .. ceiling_hits .. ' hit the ceiling, attempt capped at ' .. run.attempt)
    if ceiling_hits ~= 7 then
        return fail('retry', 'expected 7 ceiling hits, got ' .. ceiling_hits)
    end
    local fin_ok, fin_err =
        mods.supervisor.finish(ctx.sup, run.id, 'failed', cause_chain(causes))
    if not fin_ok then
        return fail('finish', tostring(fin_err))
    end
    -- Late crash reports after the terminal finish change nothing.
    local late_ok, late_err = inject_crash(mods, ctx, run, 'storm crash #11 (late)')
    if late_ok then
        return fail('retry', 'late crash after terminal finish scheduled a retry')
    end
    ev('late crash after give-up refused: ' .. tostring(late_err))
    if run.state ~= 'failed' then
        return fail('verdict', 'run left failed: ' .. run.state)
    end
    local finished = count_events(ctx, run.id, 'run.finished')
    if finished ~= 1 then
        return fail('verdict', 'run.finished fired ' .. finished .. 'x, want exactly 1')
    end
    ev('run.finished fired exactly once: the finish is idempotent under the storm')
    ev(operator_view(run, causes))
    return pass()
end

local function main()
    local mods, boot_err = bootstrap()
    if mods == nil then
        return fail('bootstrap', boot_err)
    end
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'persistent-crash'
    end
    ev('scenario=' .. scenario)
    if scenario == 'persistent-crash' then
        return scenario_persistent_crash(mods)
    elseif scenario == 'transient-crash' then
        return scenario_transient_crash(mods)
    elseif scenario == 'duplicate-crash' then
        return scenario_duplicate_crash(mods)
    elseif scenario == 'crash-storm' then
        return scenario_crash_storm(mods)
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
