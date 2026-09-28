-- task-130 driver: idle-loop quietness — the phone-battery acceptance
-- test (diver Phase-2 Decision 3, event-driven supervision).
--
-- The design justification for "no poll" over 250ms/1s: a polling
-- supervisor wakes the event loop forever; an event-driven one sleeps.
--
-- Diver probed: c84352cc850d507df477706b9166b6541ebe9e1c (main; no Phase-2
-- branch exists).
--
-- Scenarios via GAUNTLET_SCENARIO (default "deadline-handle-absent"):
--   idle-window-zero-activity      10s idle: zero new uv handles, no
--                                  background ticking, no sink events (V)
--   settled-runs-baseline-handles  100 settled runs: handle count back at
--                                  the pre-setup baseline (V)
--   deadline-handle-absent          24h deadline -> zero pending uv
--                                  handles — the gap record (A, gap)
--   timer-teardown-absent           no VimLeavePre/teardown closing timer
--                                  handles — the gap record (A, gap)
--
-- Prints exactly one JSON verdict line to stdout and always exits 0.
-- Writes nothing outside GAUNTLET_WORK_DIR. Never modifies the diver repo.

local DIVER_SHA = 'c84352cc850d507df477706b9166b6541ebe9e1c'
local EVIDENCE_MAX = 64
local IDLE_WINDOW_MS = 10000
local IDLE_WARMUP_MS = 1000
local SETTLED_RUNS = 100
local FAR_FUTURE_TIMEOUT_MS = 86400000

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function gap(where, how)
    return {
        id = 'task-130',
        outcome = 'fail',
        where = where,
        how = how,
        evidence = evidence,
    }
end

local function scenario_pass(note)
    ev(note)
    return { id = 'task-130', outcome = 'pass', evidence = evidence }
end

local function driver_fail(where, how)
    return { id = 'task-130', outcome = 'fail', where = where, how = how, evidence = evidence }
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
        return diver_lua_dir, diver_lua_dir .. '/lua'
    end
    if is_file(diver_lua_dir .. '/ai/harness/init.lua') then
        local parent = diver_lua_dir:gsub('/+$', ''):gsub('/[^/]+$', '')
        if parent == '' then
            parent = '/'
        end
        return parent, diver_lua_dir
    end
    return nil, nil
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
    local rtp_root, harness_dir = resolve_diver_dirs(diver_lua_dir)
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
        harness_dir = harness_dir .. '/ai/harness',
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

---Count live uv handles by kind. A polling supervisor would hold a
---repeating timer here forever.
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

local function fmt_counts(c)
    return 'total=' .. c.total .. ' timers=' .. c.timers .. ' repeating=' .. c.repeating
end

---idle-window-zero-activity (V): with zero live runs and zero pending
---timers, a 10-second observation window shows zero wakeups — no new
---handles, no background ticking, no sink events. The phone-battery test.
---
---Measurement note: `nvim --headless -l` itself materializes one
---repeating uv timer (repeat=200, the script runner's own loop) the
---first time the event loop idles — verified with no harness loaded.
---The scenario warms up (1s) so that infrastructure timer is inside the
---baseline census; the acceptance is then zero DELTA across the window.
local function scenario_idle_window_zero_activity(handles)
    vim.wait(IDLE_WARMUP_MS)
    local h0, err0 = count_uv_handles()
    if h0 == nil then
        return driver_fail('uv-walk', err0)
    end
    local tick_ns_0 = handles.sup.last_tick_ns
    local events_0 = handles.sink:count()
    ev('idle baseline (after 1s warm-up; the one repeating timer is nvim -l infra, not the harness): '
        .. fmt_counts(h0)
        .. ' last_tick_ns='
        .. tick_ns_0
        .. ' sink='
        .. events_0)
    vim.wait(IDLE_WINDOW_MS)
    local h1, err1 = count_uv_handles()
    if h1 == nil then
        return driver_fail('uv-walk', err1)
    end
    local tick_ns_1 = handles.sup.last_tick_ns
    local events_1 = handles.sink:count()
    ev('after 10s idle: ' .. fmt_counts(h1) .. ' last_tick_ns delta=' .. (tick_ns_1 - tick_ns_0)
        .. ' sink delta=' .. (events_1 - events_0))
    if h1.total ~= h0.total or h1.timers ~= h0.timers or h1.repeating ~= h0.repeating then
        return driver_fail('idle-handles', 'uv handle set changed during the idle window')
    end
    if tick_ns_1 ~= tick_ns_0 then
        return driver_fail('idle-tick', 'sup.last_tick_ns advanced with no tick() call — background supervision?')
    end
    if events_1 ~= events_0 then
        return driver_fail('idle-events', 'sink gained events with no activity')
    end
    ev('zero wakeups over the 10s window: the loop sleeps — no poll exists to keep it awake')
    return scenario_pass('idle loop is quiet: zero new handles, zero background ticks, zero events over 10s')
end

---settled-runs-baseline-handles (V): settled runs leave no residue — the
---handle count returns to the pre-setup baseline.
local function scenario_settled_runs_baseline(handles)
    local name, reg_err = register_lingering_adapter(handles, 'gauntlet_linger_130')
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local h0, err0 = count_uv_handles()
    if h0 == nil then
        return driver_fail('uv-walk', err0)
    end
    for i = 1, SETTLED_RUNS do
        local spec = {
            workflow = 'gauntlet_settle_flow',
            goal = 'settle run ' .. i,
            workspace = vim.env.GAUNTLET_WORK_DIR,
            adapter = name,
            timeout_ms = 600000,
        }
        local run_id, run_err = handles.harness.run(spec)
        if run_id == nil then
            return driver_fail('run', 'run ' .. i .. ': ' .. tostring(run_err))
        end
        local fok, ferr = handles.supervisor.finish(handles.sup, run_id, 'completed')
        if not fok then
            return driver_fail('finish', 'run ' .. i .. ': ' .. tostring(ferr))
        end
    end
    ev(SETTLED_RUNS .. ' runs created and settled (completed)')
    local h1, err1 = count_uv_handles()
    if h1 == nil then
        return driver_fail('uv-walk', err1)
    end
    ev('handles before: ' .. fmt_counts(h0) .. ' after: ' .. fmt_counts(h1))
    if h1.total ~= h0.total or h1.timers ~= h0.timers then
        return driver_fail(
            'handle-residue',
            'settled runs left uv handle residue: ' .. fmt_counts(h0) .. ' -> ' .. fmt_counts(h1)
        )
    end
    return scenario_pass('100 settled runs leave zero uv handle residue — count back at baseline')
end

---deadline-handle-absent (A): a run with a far-future deadline holds zero
---pending uv handles. The gap record for the one-handle acceptance.
local function scenario_deadline_handle_absent(handles)
    local name, reg_err = register_lingering_adapter(handles, 'gauntlet_linger_130b')
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local spec = {
        workflow = 'gauntlet_far_deadline_flow',
        goal = 'far future deadline',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = name,
        timeout_ms = FAR_FUTURE_TIMEOUT_MS,
    }
    local run_id, run_err = handles.harness.run(spec)
    if run_id == nil then
        return driver_fail('run', 'harness.run failed: ' .. tostring(run_err))
    end
    local h, herr = count_uv_handles()
    if h == nil then
        return driver_fail('uv-walk', herr)
    end
    ev('run live with timeout_ms=86400000 (24h); uv timers pending: ' .. h.timers)
    if h.timers ~= 0 then
        return gap(
            'deadline-handle-present',
            'a pending uv timer exists for the 24h deadline — the one-shot shipped, contradicting the probed gap'
        )
    end
    local how = 'decision-3 gap: a run with a far-future deadline (24h) holds zero pending uv handles — '
        .. 'the deadline is a bare timestamp, not a scheduled wake. Phase-2 acceptance: exactly one pending '
        .. 'uv handle per live deadline, zero wakeups during an idle observation window (the loop still '
        .. 'sleeps between one-shots).'
    return gap('deadline-handle-absent', how)
end

---timer-teardown-absent (A): no VimLeavePre/teardown in the harness closes
---timer handles — because none are tracked. The gap record. Gathered by
---reading the harness sources at runtime (read-only scan).
local function scenario_timer_teardown_absent(handles)
    local found_teardown = {}
    local found_timer = {}
    local files = 0
    local dir_ok, dir_err = pcall(function()
        for fname, ftype in vim.fs.dir(handles.harness_dir) do
            if ftype == 'file' and fname:sub(-4) == '.lua' then
                files = files + 1
                local fh = io.open(handles.harness_dir .. '/' .. fname, 'r')
                if fh ~= nil then
                    local content = fh:read('*a')
                    fh:close()
                    if content:find('VimLeavePre', 1, true) then
                        found_teardown[#found_teardown + 1] = fname
                    end
                    if content:find('new_timer', 1, true) then
                        found_timer[#found_timer + 1] = fname
                    end
                end
            end
        end
    end)
    if not dir_ok then
        return driver_fail('scan', 'harness source scan failed: ' .. tostring(dir_err))
    end
    ev('scanned ' .. files .. ' harness sources in ' .. handles.harness_dir)
    ev("'VimLeavePre' matches: " .. #found_teardown .. "; 'new_timer' matches: " .. #found_timer)
    if #found_teardown > 0 or #found_timer > 0 then
        return gap(
            'timer-teardown-present',
            'timer teardown surface exists — the one-shot lifecycle shipped, contradicting the probed gap'
        )
    end
    local how = 'decision-3 gap: no VimLeavePre autocmd or teardown path in lua/ai/harness/ closes uv '
        .. 'timer handles — none are created or tracked, so there is nothing to tear down. Phase-2 '
        .. 'acceptance: VimLeavePre (or the harness teardown entry) closes every tracked handle, teardown '
        .. 'emits no errors, and the post-teardown handle count equals the pre-setup baseline.'
    return gap('timer-teardown-absent', how)
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'deadline-handle-absent'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return driver_fail('bootstrap', boot_err)
    end
    ev('diver probed: ' .. DIVER_SHA .. ' (main; no Phase-2 branch exists)')
    ev('scenario=' .. scenario)
    if scenario == 'idle-window-zero-activity' then
        return scenario_idle_window_zero_activity(handles)
    elseif scenario == 'settled-runs-baseline-handles' then
        return scenario_settled_runs_baseline(handles)
    elseif scenario == 'deadline-handle-absent' then
        return scenario_deadline_handle_absent(handles)
    elseif scenario == 'timer-teardown-absent' then
        return scenario_timer_teardown_absent(handles)
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
