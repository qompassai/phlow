-- task-128 driver: retry one-shot lifecycle (diver Phase-2 Decision 3,
-- event-driven supervision acceptance probe).
--
-- The design: retry_run schedules a one-shot at retry_at_ns ->
-- wake(sup, now, 'retry') -> re-queue and relaunch; cancel during
-- retry_wait cancels the timer.
--
-- Diver probed: c84352cc850d507df477706b9166b6541ebe9e1c (main; no Phase-2
-- branch exists).
--
-- Scenarios via GAUNTLET_SCENARIO (default "retry-one-shot-absent"):
--   retry-promotes-via-tick   retry_run sets a timestamp; only tick()
--                             promotes it — attempt+1 exactly once (V)
--   retry-ceiling-refusal     retry past RETRY_ATTEMPTS_MAX refuses cleanly
--                             with no state change (V)
--   retry-one-shot-absent     no one-shot scheduled at retry_at_ns — the
--                             gap record (A, gap)
--   cancel-during-retry-wait  cancel in retry_wait, then tick far past
--                             retry_at_ns: no phantom relaunch (A)
--
-- Prints exactly one JSON verdict line to stdout and always exits 0.
-- Writes nothing outside GAUNTLET_WORK_DIR. Never modifies the diver repo.

local DIVER_SHA = 'c84352cc850d507df477706b9166b6541ebe9e1c'
local EVIDENCE_MAX = 64
local RETRY_CYCLES_TO_CEILING = 3

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function gap(where, how)
    return {
        id = 'task-128',
        outcome = 'fail',
        where = where,
        how = how,
        evidence = evidence,
    }
end

local function scenario_pass(note)
    ev(note)
    return { id = 'task-128', outcome = 'pass', evidence = evidence }
end

local function driver_fail(where, how)
    return { id = 'task-128', outcome = 'fail', where = where, how = how, evidence = evidence }
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
        start_count = 0,
    }
end

---Stub adapter that stays running at start and counts invocations, so the
---driver can prove exactly-once relaunch (no phantom starts).
local function register_counting_adapter(handles, name)
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
        handles.start_count = handles.start_count + 1
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

---Count live uv handles by kind.
local function count_uv_handles()
    local counts = { total = 0, timers = 0, unknown = 0 }
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
        end
    end)
    if not walk_ok then
        return nil, 'vim.uv.walk failed: ' .. tostring(walk_err)
    end
    return counts
end

local function start_run(handles, adapter_name)
    local spec = {
        workflow = 'gauntlet_retry_flow',
        goal = 'probe retry one-shot',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = adapter_name,
        timeout_ms = 600000,
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

---retry-promotes-via-tick (V): retry_run sets retry_at_ns and attempt+1;
---only an explicit tick() at/after retry_at_ns promotes — relaunch happens
---exactly once, attempt increments exactly once.
local function scenario_retry_promotes_via_tick(handles)
    local name, reg_err = register_counting_adapter(handles, 'gauntlet_retry_128')
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local run_id, start_err = start_run(handles, name)
    if run_id == nil then
        return driver_fail('run', start_err)
    end
    ev('run live, adapter start count=' .. handles.start_count)
    local rok, rerr = handles.supervisor.retry_run(handles.sup, run_id, 'transient boom')
    if not rok then
        return driver_fail('retry', 'retry_run failed: ' .. tostring(rerr))
    end
    local run = handles.supervisor.get(handles.sup, run_id)
    if run.state ~= 'retry_wait' then
        return driver_fail('retry-state', 'expected retry_wait, got ' .. tostring(run.state))
    end
    if run.attempt ~= 2 then
        return driver_fail('retry-attempt', 'expected attempt 2, got ' .. tostring(run.attempt))
    end
    if type(run.retry_at_ns) ~= 'number' then
        return driver_fail('retry-at', 'retry_at_ns is not a timestamp')
    end
    ev('retry_run ok: state=retry_wait attempt=2 retry_at_ns set (a timestamp, not a timer)')
    -- Tick before due: nothing promotes.
    handles.supervisor.tick(handles.sup, run.retry_at_ns - 1000000)
    run = handles.supervisor.get(handles.sup, run_id)
    if run.state ~= 'retry_wait' then
        return driver_fail('early-promote', 'tick before retry_at_ns promoted the run: ' .. run.state)
    end
    ev('tick before retry_at_ns: still retry_wait — no early promotion')
    -- Tick after due: exactly one promotion and one relaunch.
    handles.supervisor.tick(handles.sup, run.retry_at_ns + 1000000)
    run = handles.supervisor.get(handles.sup, run_id)
    if run.state ~= 'running' then
        return driver_fail('promote', 'tick after retry_at_ns did not relaunch: ' .. tostring(run.state))
    end
    if run.attempt ~= 2 then
        return driver_fail('attempt-double', 'attempt incremented twice: ' .. tostring(run.attempt))
    end
    if handles.start_count ~= 2 then
        return driver_fail('relaunch-count', 'adapter start count=' .. handles.start_count .. ', want 2')
    end
    ev('tick at retry_at_ns: re-queued and relaunched exactly once, attempt still 2')
    -- Clock jump: retry_at_ns explicitly in the past. A fresh run goes to
    -- retry_wait, then the driver backdates the timestamp 60s (simulating a
    -- clock jump — no timer exists to misfire, so the probe is about the
    -- tick() comparison). Promotion must be immediate on the next tick,
    -- exactly once, with no spin or double launch on the tick after.
    local run_id2, start_err2 = start_run(handles, name)
    if run_id2 == nil then
        return driver_fail('run2', start_err2)
    end
    local rok2, rerr2 = handles.supervisor.retry_run(handles.sup, run_id2, 'transient boom 2')
    if not rok2 then
        return driver_fail('retry2', 'retry_run failed: ' .. tostring(rerr2))
    end
    local run2 = handles.supervisor.get(handles.sup, run_id2)
    if run2.state ~= 'retry_wait' or run2.attempt ~= 2 then
        return driver_fail('retry2-state', 'expected retry_wait/attempt 2, got ' .. tostring(run2.state) .. '/' .. tostring(run2.attempt))
    end
    run2.retry_at_ns = handles.types.now_ns() - 60000000000
    ev('simulated clock jump: run2.retry_at_ns backdated 60s into the past')
    local count_before = handles.start_count
    handles.supervisor.tick(handles.sup, handles.types.now_ns())
    run2 = handles.supervisor.get(handles.sup, run_id2)
    if run2.state ~= 'running' then
        return driver_fail('past-due-promote', 'tick did not promote the past-due retry: ' .. tostring(run2.state))
    end
    if run2.attempt ~= 2 then
        return driver_fail('past-due-attempt', 'past-due promotion changed attempt: ' .. tostring(run2.attempt))
    end
    if handles.start_count ~= count_before + 1 then
        return driver_fail(
            'past-due-relaunch',
            'past-due promotion launched ' .. (handles.start_count - count_before) .. ' times (want exactly 1)'
        )
    end
    ev('past-due retry_at_ns: immediate promotion on tick, exactly one relaunch, attempt still 2')
    handles.supervisor.tick(handles.sup, handles.types.now_ns() + 1000000000)
    run2 = handles.supervisor.get(handles.sup, run_id2)
    if handles.start_count ~= count_before + 1 then
        return driver_fail(
            'past-due-spin',
            'tick after past-due promotion relaunched again: start count=' .. handles.start_count .. ' (spin/double launch)'
        )
    end
    if run2.state ~= 'running' then
        return driver_fail('past-due-state', 'run2 left running after second tick: ' .. tostring(run2.state))
    end
    ev('second tick: no relaunch — no spin, no double launch')
    ev('source: supervisor.lua M.retry_run (lines 340-360), M.tick retry promotion (line 486)')
    return scenario_pass('retry promotes only via explicit tick(); attempt+1 and relaunch exactly once')
end

---retry-ceiling-refusal (V): retry past RETRY_ATTEMPTS_MAX refuses cleanly
---before any mutation — no state change, no timer scheduled.
local function scenario_retry_ceiling_refusal(handles)
    local name, reg_err = register_counting_adapter(handles, 'gauntlet_retry_128b')
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local run_id, start_err = start_run(handles, name)
    if run_id == nil then
        return driver_fail('run', start_err)
    end
    for _ = 1, RETRY_CYCLES_TO_CEILING do
        local rok, rerr = handles.supervisor.retry_run(handles.sup, run_id, 'transient')
        if not rok then
            return driver_fail('retry-cycle', 'expected retry_run ok, got: ' .. tostring(rerr))
        end
        local run = handles.supervisor.get(handles.sup, run_id)
        handles.supervisor.tick(handles.sup, run.retry_at_ns + 1000000)
    end
    local run = handles.supervisor.get(handles.sup, run_id)
    ev('after 3 retry cycles: attempt=' .. run.attempt .. ' state=' .. run.state)
    if run.attempt ~= 4 or run.state ~= 'running' then
        return driver_fail('ceiling-setup', 'unexpected state before ceiling: ' .. run.state)
    end
    local h_before, herr = count_uv_handles()
    if h_before == nil then
        return driver_fail('uv-walk', herr)
    end
    local retry_at_before = run.retry_at_ns
    local rok, rerr = handles.supervisor.retry_run(handles.sup, run_id, 'one too many')
    if rok then
        return driver_fail('ceiling', 'retry_run past the ceiling SUCCEEDED')
    end
    if type(rerr) ~= 'string' or not rerr:find('retry attempt ceiling exceeded', 1, true) then
        return driver_fail('ceiling-err', "refusal is not the ceiling error: " .. vim.inspect(rerr))
    end
    ev("4th retry_run refused: '" .. rerr .. "'")
    run = handles.supervisor.get(handles.sup, run_id)
    if run.attempt ~= 4 or run.state ~= 'running' then
        return driver_fail('ceiling-mutation', 'refused retry mutated the run: ' .. run.state)
    end
    if run.retry_at_ns ~= retry_at_before then
        return driver_fail('ceiling-mutation', 'refused retry changed retry_at_ns')
    end
    local h_after, herr2 = count_uv_handles()
    if h_after == nil then
        return driver_fail('uv-walk', herr2)
    end
    if h_after.timers ~= h_before.timers then
        return driver_fail('ceiling-timer', 'refused retry scheduled a uv timer')
    end
    ev('run untouched (attempt=4, running), retry_at_ns unchanged, no timer scheduled')
    return scenario_pass('retry past the ceiling refuses cleanly with no state change')
end

---retry-one-shot-absent (A): retry_run schedules no one-shot at
---retry_at_ns. The gap record.
local function scenario_retry_one_shot_absent(handles)
    ev('file evidence: supervisor.lua M.retry_run (lines 340-360) — sets retry_at_ns timestamp only')
    ev('file evidence: supervisor.lua M.tick (line 486) — retry promotion is a timestamp comparison only')
    ev('file evidence: zero vim.uv timer creations anywhere in lua/ai/harness/')
    local how = 'decision-3 gap: retry_run schedules no one-shot vim.uv timer at retry_at_ns — the retry '
        .. 'is promoted only when someone calls tick(). Phase-2 acceptance: retry_run schedules a one-shot '
        .. "at retry_at_ns -> wake(sup, now, 'retry') -> re-queue and relaunch, attempt incremented exactly "
        .. 'once; retry_at_ns already in the past fires immediately and exactly once (no spin, no '
        .. 'double-launch).'
    return gap('retry-one-shot-absent', how)
end

---cancel-during-retry-wait (A): cancel in retry_wait, then tick far past
---retry_at_ns — the stale timestamp must never cause a phantom relaunch.
local function scenario_cancel_during_retry_wait(handles)
    local name, reg_err = register_counting_adapter(handles, 'gauntlet_retry_128c')
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local run_id, start_err = start_run(handles, name)
    if run_id == nil then
        return driver_fail('run', start_err)
    end
    local rok, rerr = handles.supervisor.retry_run(handles.sup, run_id, 'transient')
    if not rok then
        return driver_fail('retry', 'retry_run failed: ' .. tostring(rerr))
    end
    local run = handles.supervisor.get(handles.sup, run_id)
    local retry_at = run.retry_at_ns
    ev('run in retry_wait (attempt=2), adapter start count=' .. handles.start_count)
    local cok, cerr = handles.supervisor.cancel(handles.sup, run_id, 'operator cancel')
    if not cok then
        return driver_fail('cancel', 'cancel in retry_wait failed: ' .. tostring(cerr))
    end
    ev('cancelled during retry_wait')
    -- Far past the retry time: the stale retry_at_ns must not relaunch.
    handles.supervisor.tick(handles.sup, retry_at + 3600000000000)
    run = handles.supervisor.get(handles.sup, run_id)
    if run.state ~= 'cancelled' then
        return driver_fail('phantom', 'tick past retry_at_ns moved a cancelled run: ' .. run.state)
    end
    if handles.start_count ~= 1 then
        return driver_fail(
            'phantom-relaunch',
            'adapter start count=' .. handles.start_count .. ' — phantom relaunch after cancel'
        )
    end
    ev('tick far past retry_at_ns: run stays cancelled, adapter start count still 1 — no phantom relaunch')
    ev('source: supervisor.lua M.tick (line 476) skips terminal runs, so the stale timestamp is inert')
    return scenario_pass('cancel during retry_wait kills the retry — no phantom relaunch')
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'retry-one-shot-absent'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return driver_fail('bootstrap', boot_err)
    end
    ev('diver probed: ' .. DIVER_SHA .. ' (main; no Phase-2 branch exists)')
    ev('scenario=' .. scenario)
    if scenario == 'retry-promotes-via-tick' then
        return scenario_retry_promotes_via_tick(handles)
    elseif scenario == 'retry-ceiling-refusal' then
        return scenario_retry_ceiling_refusal(handles)
    elseif scenario == 'retry-one-shot-absent' then
        return scenario_retry_one_shot_absent(handles)
    elseif scenario == 'cancel-during-retry-wait' then
        return scenario_cancel_during_retry_wait(handles)
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
