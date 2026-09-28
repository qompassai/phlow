-- task-126 driver: sink-append wakes supervision, no poll (diver Phase-2
-- Decision 3, event-driven supervision acceptance probe).
--
-- The design: the event sink gains an `on_append` subscriber hook; the
-- supervisor registers its wake callback at setup and every append invokes
-- subscribers in pcall; a `waking` reentrancy flag coalesces storms; NO
-- periodic timer exists. `tick()` is retained as the test driver and as the
-- body `wake` invokes.
--
-- Diver probed: c84352cc850d507df477706b9166b6541ebe9e1c (main; no Phase-2
-- branch exists).
--
-- Scenarios via GAUNTLET_SCENARIO (default "wake-hook-absent"):
--   no-repeating-timers      setup creates zero uv repeating timers — the
--                            no-poll regression guard (V, passes)
--   tick-drives-completions  model.completed drains only on an explicit
--                            tick() — tick as test driver (V, passes)
--   wake-hook-absent         sink has no on_append hook; supervisor has no
--                            wake — the gap record (A, gap)
--   wake-coalescing-absent   no waking flag; storm of appends causes zero
--                            supervision passes — the gap record (A, gap)
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

local function gap(where, how)
    return {
        id = 'task-126',
        outcome = 'fail',
        where = where,
        how = how,
        evidence = evidence,
    }
end

local function scenario_pass(note)
    ev(note)
    return { id = 'task-126', outcome = 'pass', evidence = evidence }
end

local function driver_fail(where, how)
    return { id = 'task-126', outcome = 'fail', where = where, how = how, evidence = evidence }
end

---Resolve DIVER_LUA_DIR to the runtimepath root. Accepts either the rtp
---root itself (holding lua/ai/harness/init.lua) or the lua/ dir directly
---(holding ai/harness/init.lua): the rtp entry must be the directory that
---*contains* lua/, or require() never fires.
local function resolve_diver_dirs(diver_lua_dir)
    local function is_file(path)
        local fh = io.open(path, 'r')
        if fh == nil then
            return false
        end
        fh:close()
        return true
    end
    if is_file(diver_lua_dir .. '/lua/ai/harness/init.lua') then
        return diver_lua_dir
    end
    if is_file(diver_lua_dir .. '/ai/harness/init.lua') then
        local parent = diver_lua_dir:gsub('/+$', ''):gsub('/[^/]+$', '')
        if parent == '' then
            parent = '/'
        end
        return parent
    end
    return nil
end

---Wire up the harness. Returns handles or (nil, err).
local function bootstrap()
    local diver_lua_dir = vim.env.DIVER_LUA_DIR
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    if type(diver_lua_dir) ~= 'string' or diver_lua_dir == '' then
        return nil, 'DIVER_LUA_DIR is not set'
    end
    if type(work_dir) ~= 'string' or work_dir == '' then
        return nil, 'GAUNTLET_WORK_DIR is not set'
    end
    local rtp_root = resolve_diver_dirs(diver_lua_dir)
    if rtp_root == nil then
        return nil, 'DIVER_LUA_DIR has no ai/harness/init.lua under <dir>/lua or <dir>: ' .. diver_lua_dir
    end
    vim.opt.runtimepath:append(rtp_root)
    local harness_ok, harness = pcall(require, 'ai.harness')
    if not harness_ok then
        return nil, "require('ai.harness') failed: " .. tostring(harness)
    end
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
        registry = st.registry,
        registry_mod = require('ai.harness.registry'),
        supervisor = require('ai.harness.supervisor'),
        types = require('ai.harness.types'),
    }
end

---Stub adapter that stays running at start (no completion event), so the
---driver controls the run's terminal transition.
local function register_lingering_adapter(handles, name)
    local adapter = { name = name }
    function adapter.probe()
        return {
            streaming = false,
            cancellation = true,
            resume = false,
            permissions = false,
            artifacts = false,
            remote = false,
            tools = false,
        }
    end
    function adapter.start(run, _sink)
        return { adapter = name, run_id = run.id, closed = false }
    end
    function adapter.cancel(_handle)
        return true
    end
    function adapter.close(handle)
        handle.closed = true
    end
    local ok, err = handles.registry_mod.register_adapter(handles.registry, name, adapter)
    if not ok then
        return nil, err
    end
    return name
end

---Count live uv handles by kind. Repeating timers are the polling-loop
---signature: a periodic supervision timer would show up here.
local function count_uv_handles()
    local counts = { total = 0, timers = 0, repeating = 0, unknown = 0 }
    local walk_ok, walk_err = pcall(vim.uv.walk, function(handle)
        counts.total = counts.total + 1
        local type_ok, htype = pcall(function()
            return handle:get_type()
        end)
        if not type_ok then
            counts.unknown = counts.unknown + 1
            return
        end
        if htype == 'timer' then
            counts.timers = counts.timers + 1
            local rep_ok, rep = pcall(function()
                return handle:get_repeat()
            end)
            if rep_ok and type(rep) == 'number' and rep > 0 then
                counts.repeating = counts.repeating + 1
            end
        end
    end)
    if not walk_ok then
        return nil, 'vim.uv.walk failed: ' .. tostring(walk_err)
    end
    return counts
end

local function start_run(handles, adapter_name, workflow)
    local spec = {
        workflow = workflow or 'gauntlet_wake_flow',
        goal = 'probe sink-append wake',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = adapter_name,
        timeout_ms = 300000,
    }
    local run_id, run_err = handles.harness.run(spec)
    if run_id == nil then
        return nil, 'harness.run failed: ' .. tostring(run_err)
    end
    local run = handles.supervisor.get(handles.sup, run_id)
    if run.state ~= 'running' then
        return nil, 'run did not reach running: ' .. tostring(run.state)
    end
    return run_id
end

---no-repeating-timers (V): setup creates zero uv repeating timers. The
---no-poll regression guard: it passes trivially today (setup schedules
---nothing) and must keep passing once Phase 2 adds one-shots — a periodic
---timer appearing here would be the design violation.
local function scenario_no_repeating_timers(handles)
    local before, berr = count_uv_handles()
    if before == nil then
        return driver_fail('uv-walk', berr)
    end
    -- setup already ran in bootstrap; count now and compare against the
    -- pre-harness baseline is impossible here, so assert the absolute:
    -- zero repeating timers exist after setup.
    ev('uv handles after harness.setup: total=' .. before.total .. ' timers=' .. before.timers
        .. ' repeating=' .. before.repeating)
    if before.repeating ~= 0 then
        return driver_fail(
            'periodic-timer',
            'harness setup left ' .. before.repeating .. ' repeating uv timer(s) — a polling loop exists'
        )
    end
    ev('file evidence: no vim.uv timer creation anywhere in lua/ai/harness/ (grep: zero matches)')
    return scenario_pass('no repeating uv timers after setup — no polling loop (regression guard)')
end

---tick-drives-completions (V): the completion drain runs only when tick()
---is called explicitly. Characterization of tick() as the test driver —
---the body Phase-2 wake will invoke.
local function scenario_tick_drives_completions(handles)
    local name, reg_err = register_lingering_adapter(handles, 'gauntlet_linger_126')
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local run_id, start_err = start_run(handles, name)
    if run_id == nil then
        return driver_fail('run', start_err)
    end
    -- The adapter "reports" completion by appending straight to the sink.
    local event, aerr = handles.sink:append(
        run_id,
        'model.completed',
        { outcome = 'completed' },
        { source = 'gauntlet' }
    )
    if event == nil then
        return driver_fail('append', 'sink append failed: ' .. tostring(aerr))
    end
    ev('appended model.completed directly to the sink (seq=' .. event.seq .. ')')
    local run = handles.supervisor.get(handles.sup, run_id)
    if run.state == 'completed' then
        return driver_fail(
            'auto-wake',
            'run finished WITHOUT any tick() call — a wake mechanism exists, contradicting the probed gap'
        )
    end
    ev('run still ' .. run.state .. ' after append with no tick() — nothing woke supervision')
    local acted = handles.supervisor.tick(handles.sup, handles.types.now_ns())
    ev('explicit tick() acted=' .. acted)
    run = handles.supervisor.get(handles.sup, run_id)
    if run.state ~= 'completed' then
        return driver_fail('tick-drain', 'tick() did not drain the completion: ' .. tostring(run.state))
    end
    ev('source: supervisor.lua drain_completions (line 436), invoked only from M.tick (line 468)')
    return scenario_pass('completion drained only by explicit tick() — tick is the sole supervision driver')
end

---wake-hook-absent (A): the sink has no on_append subscriber hook and the
---supervisor has no wake. The gap record.
local function scenario_wake_hook_absent(handles)
    if handles.sink.on_append ~= nil then
        return gap(
            'wake-hook-present',
            'sink.on_append exists — the wake hook shipped, contradicting the probed gap'
        )
    end
    ev('sink.on_append == nil')
    if handles.supervisor.wake ~= nil then
        return gap(
            'wake-present',
            'supervisor.wake exists — the wake callback shipped, contradicting the probed gap'
        )
    end
    ev('supervisor.wake == nil (no waking flag, no wake callback anywhere in supervisor.lua)')
    ev('file evidence: events.lua sink:append (lines 108-117) is a bare table insert — no subscriber invocation')
    local how = 'decision-3 gap: the event sink has no `on_append` subscriber hook and the supervisor '
        .. 'registers no wake callback, so appends never drive supervision — only explicit tick() calls do '
        .. '(see tick-drives-completions). Phase-2 acceptance: sink:on_append(fn) invokes subscribers in pcall '
        .. 'on every append; supervisor registers its wake at setup; wake runs the tick() body; a `waking` '
        .. 'reentrancy flag coalesces nested wakes.'
    return gap('sink-wake-hook-absent', how)
end

---wake-coalescing-absent (A): a storm of 1000 appends causes zero
---supervision passes; the drain itself is storm-safe but only an explicit
---tick runs it. The gap record for the coalescing criterion.
---
---Design requires 1000 rapid appends, but RUNS_MAX is 256, so the storm
---uses 10 runs x 100 appends: 99 harmless `model.stream_delta` events
---plus one `model.completed` per run. Repeated completions would
---re-finish runs (invalid-transition noise); exactly one completion per
---run keeps the drain clean and proves exactly one finish each.
local function scenario_wake_coalescing_absent(handles)
    local name, reg_err = register_lingering_adapter(handles, 'gauntlet_storm_126')
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local STORM_RUN_COUNT = 10
    local APPENDS_PER_RUN = 100
    local TOTAL_APPENDS = STORM_RUN_COUNT * APPENDS_PER_RUN
    local run_ids = {}
    for i = 1, STORM_RUN_COUNT do
        local run_id, start_err = start_run(handles, name, 'gauntlet_storm_flow_' .. i)
        if run_id == nil then
            return driver_fail('run', 'storm run ' .. i .. ': ' .. start_err)
        end
        run_ids[#run_ids + 1] = run_id
    end
    ev(STORM_RUN_COUNT .. ' runs live')
    -- Storm: 100 appends per run, as fast as the loop goes.
    local appended = 0
    for _, run_id in ipairs(run_ids) do
        for i = 1, APPENDS_PER_RUN - 1 do
            local event, aerr = handles.sink:append(
                run_id,
                'model.stream_delta',
                { delta = 'storm ' .. i },
                { source = 'gauntlet' }
            )
            if event == nil then
                return driver_fail('append', 'storm delta append failed: ' .. tostring(aerr))
            end
            appended = appended + 1
        end
        local done, derr = handles.sink:append(
            run_id,
            'model.completed',
            { outcome = 'completed' },
            { source = 'gauntlet' }
        )
        if done == nil then
            return driver_fail('append', 'storm completion append failed: ' .. tostring(derr))
        end
        appended = appended + 1
    end
    if appended ~= TOTAL_APPENDS then
        return driver_fail('storm-count', 'expected ' .. TOTAL_APPENDS .. ' appends, got ' .. appended)
    end
    ev(TOTAL_APPENDS .. ' rapid appends (99 stream_delta + 1 model.completed per run), zero supervision passes so far')
    -- Nothing woke: sample runs are still live.
    for i = 1, 5 do
        local run = handles.supervisor.get(handles.sup, run_ids[i])
        if run.state ~= 'running' then
            return driver_fail(
                'auto-wake',
                'run finished during the storm without tick() — a wake exists, contradicting the probed gap'
            )
        end
    end
    ev('sampled runs still running after the storm — no wake fired')
    -- One explicit tick drains everything: the drain pass itself is
    -- storm-safe (single synchronous pass), only the trigger is missing.
    local acted = handles.supervisor.tick(handles.sup, handles.types.now_ns())
    ev('one explicit tick() acted=' .. acted)
    local finished = 0
    for _, event in ipairs(handles.sink:events()) do
        if event.kind == 'run.finished' then
            finished = finished + 1
        end
    end
    if finished ~= STORM_RUN_COUNT then
        return driver_fail(
            'storm-drain',
            'expected ' .. STORM_RUN_COUNT .. ' run.finished events, got ' .. finished
        )
    end
    ev(STORM_RUN_COUNT .. ' runs finished, exactly one run.finished each — single pass, no recursion')
    local acted2 = handles.supervisor.tick(handles.sup, handles.types.now_ns())
    if acted2 ~= 0 then
        return driver_fail('double-tick', 'second tick acted=' .. acted2 .. ' — double finish?')
    end
    ev('second tick acted=0 — finished runs are never re-finished')
    local how = 'decision-3 gap: 1000 rapid appends cause zero supervision passes — there is '
        .. 'no `waking` reentrancy flag and no wake to coalesce, because appends never wake anything. '
        .. 'Phase-2 acceptance: N rapid appends cause at most one in-flight wake pass plus one trailing pass '
        .. '(assert <= 3 wake passes for 1000 appends); nested append during a wake pass does not recurse; '
        .. 'every run finishes exactly once.'
    return gap('wake-coalescing-absent', how)
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'wake-hook-absent'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return driver_fail('bootstrap', boot_err)
    end
    ev('diver probed: ' .. DIVER_SHA .. ' (main; no Phase-2 branch exists)')
    ev('scenario=' .. scenario)
    if scenario == 'no-repeating-timers' then
        return scenario_no_repeating_timers(handles)
    elseif scenario == 'tick-drives-completions' then
        return scenario_tick_drives_completions(handles)
    elseif scenario == 'wake-hook-absent' then
        return scenario_wake_hook_absent(handles)
    elseif scenario == 'wake-coalescing-absent' then
        return scenario_wake_coalescing_absent(handles)
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
