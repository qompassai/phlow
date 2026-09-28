-- task-127 driver: deadline one-shots and handle hygiene (diver Phase-2
-- Decision 3, event-driven supervision acceptance probe).
--
-- The design: run create schedules a one-shot vim.uv timer at deadline_ns
-- -> wake(sup, now, 'deadline'); the handle is tracked in run._timers and
-- cancelled on terminal entry inside transition() (the airtight place —
-- every terminal path goes through it).
--
-- Diver probed: c84352cc850d507df477706b9166b6541ebe9e1c (main; no Phase-2
-- branch exists).
--
-- Scenarios via GAUNTLET_SCENARIO (default "deadline-one-shot-absent"):
--   deadline-fires-via-tick        deadline enforced only by an explicit
--                                  tick() comparison (V, passes)
--   no-one-shot-at-create          create schedules no uv timer; no
--                                  run._timers (V, passes)
--   deadline-one-shot-absent       no one-shot at deadline_ns — the gap
--                                  record (A, gap)
--   terminal-entry-no-timer-cleanup transition() cancels nothing; the
--                                  hygiene property holds vacuously — the
--                                  gap record (A, gap)
--
-- Prints exactly one JSON verdict line to stdout and always exits 0.
-- Writes nothing outside GAUNTLET_WORK_DIR. Never modifies the diver repo.

local DIVER_SHA = 'c84352cc850d507df477706b9166b6541ebe9e1c'
local EVIDENCE_MAX = 64
local WAIT_SLICE_MS = 20
local WAIT_SLICES_MAX = 250

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function gap(where, how)
    return {
        id = 'task-127',
        outcome = 'fail',
        where = where,
        how = how,
        evidence = evidence,
    }
end

local function scenario_pass(note)
    ev(note)
    return { id = 'task-127', outcome = 'pass', evidence = evidence }
end

local function driver_fail(where, how)
    return { id = 'task-127', outcome = 'fail', where = where, how = how, evidence = evidence }
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

---Count live uv handles by kind.
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
        end
    end)
    if not walk_ok then
        return nil, 'vim.uv.walk failed: ' .. tostring(walk_err)
    end
    return counts
end

local function start_run(handles, adapter_name, timeout_ms)
    local spec = {
        workflow = 'gauntlet_deadline_flow',
        goal = 'probe deadline one-shot',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = adapter_name,
        timeout_ms = timeout_ms,
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

---Wait (bounded) until the wall clock passes target_ns. Returns true when
---the deadline passed, false on timeout of the wait itself.
local function wait_until_ns(target_ns)
    for _ = 1, WAIT_SLICES_MAX do
        if vim.uv.hrtime() >= target_ns then
            return true
        end
        vim.wait(WAIT_SLICE_MS)
    end
    return false
end

---deadline-fires-via-tick (V): the deadline is a timestamp compared inside
---tick(); nothing fires it. Characterization of today's enforcement path.
local function scenario_deadline_fires_via_tick(handles)
    local name, reg_err = register_lingering_adapter(handles, 'gauntlet_linger_127')
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local run_id, start_err = start_run(handles, name, 150)
    if run_id == nil then
        return driver_fail('run', start_err)
    end
    local run = handles.supervisor.get(handles.sup, run_id)
    ev('run.deadline_ns set at create (timeout_ms=150)')
    if not wait_until_ns(run.deadline_ns) then
        return driver_fail('wait', 'wall clock never passed deadline_ns within the bounded wait')
    end
    ev('wall clock passed deadline_ns')
    run = handles.supervisor.get(handles.sup, run_id)
    if run.state ~= 'running' then
        return driver_fail(
            'auto-timeout',
            'run left running WITHOUT any tick() — a deadline timer exists, contradicting the probed gap'
        )
    end
    ev('run still running past its deadline with no tick() — no timer fired')
    local acted = handles.supervisor.tick(handles.sup, handles.types.now_ns())
    ev('explicit tick() acted=' .. acted)
    run = handles.supervisor.get(handles.sup, run_id)
    if run.state ~= 'timed_out' then
        return driver_fail('tick-deadline', 'tick() did not time out the run: ' .. tostring(run.state))
    end
    local reason = nil
    for _, event in ipairs(handles.sink:events(run_id)) do
        if event.kind == 'run.finished' then
            reason = event.payload.reason
        end
    end
    ev("run.finished reason='" .. tostring(reason) .. "'")
    -- Defensive second fire: tick again far past the deadline. transition()
    -- is the single choke point for every terminal path, so the terminal run
    -- is skipped and no second run.finished may be emitted.
    local finished_count = 0
    for _, event in ipairs(handles.sink:events(run_id)) do
        if event.kind == 'run.finished' then
            finished_count = finished_count + 1
        end
    end
    handles.supervisor.tick(handles.sup, handles.types.now_ns() + 60000000000)
    local finished_after = 0
    for _, event in ipairs(handles.sink:events(run_id)) do
        if event.kind == 'run.finished' then
            finished_after = finished_after + 1
        end
    end
    if finished_count ~= 1 or finished_after ~= 1 then
        return driver_fail(
            'second-fire',
            'second tick past deadline changed the run.finished count: '
                .. finished_count
                .. ' -> '
                .. finished_after
                .. ' (want exactly 1)'
        )
    end
    ev('second tick far past deadline: still exactly one run.finished — finish is idempotent')
    -- Deadline while waiting_approval: nothing in the harness transitions a
    -- run into waiting_approval today (the state exists only in types.lua),
    -- so the driver sets it directly to simulate the Phase-2 precondition,
    -- then probes the tick() deadline path against it. The deadline must be
    -- absolute, not paused, for that state.
    local run_id2, start_err2 = start_run(handles, name, 150)
    if run_id2 == nil then
        return driver_fail('run2', start_err2)
    end
    local run2 = handles.supervisor.get(handles.sup, run_id2)
    run2.state = 'waiting_approval'
    ev("simulated precondition: run2.state set to 'waiting_approval' directly (no harness path exists today)")
    if not wait_until_ns(run2.deadline_ns) then
        return driver_fail('wait2', 'wall clock never passed run2 deadline_ns within the bounded wait')
    end
    ev('wall clock passed run2 deadline_ns while waiting_approval')
    run2 = handles.supervisor.get(handles.sup, run_id2)
    if run2.state ~= 'waiting_approval' then
        return driver_fail(
            'auto-timeout-approval',
            'run left waiting_approval WITHOUT any tick() — a deadline timer exists, contradicting the probed gap'
        )
    end
    ev("run still waiting_approval past its deadline with no tick() — no timer fired")
    handles.supervisor.tick(handles.sup, handles.types.now_ns())
    run2 = handles.supervisor.get(handles.sup, run_id2)
    if run2.state ~= 'timed_out' then
        return driver_fail(
            'approval-deadline',
            'tick() did not time out the waiting_approval run: ' .. tostring(run2.state)
        )
    end
    local finished2 = 0
    for _, event in ipairs(handles.sink:events(run_id2)) do
        if event.kind == 'run.finished' then
            finished2 = finished2 + 1
        end
    end
    handles.supervisor.tick(handles.sup, handles.types.now_ns() + 60000000000)
    local finished2_after = 0
    for _, event in ipairs(handles.sink:events(run_id2)) do
        if event.kind == 'run.finished' then
            finished2_after = finished2_after + 1
        end
    end
    if finished2 ~= 1 or finished2_after ~= 1 then
        return driver_fail(
            'approval-second-fire',
            'waiting_approval run.finished count: ' .. finished2 .. ' -> ' .. finished2_after .. ' (want exactly 1)'
        )
    end
    ev("tick() times out the waiting_approval run — the deadline is absolute, not paused")
    ev('second tick: still exactly one run.finished for the waiting_approval run')
    ev('source: supervisor.lua M.tick deadline check (line 482: now_ns >= run.deadline_ns)')
    return scenario_pass('deadline enforced only by explicit tick() comparison — no one-shot timer')
end

---no-one-shot-at-create (V): creating a run schedules no uv timer and
---tracks no per-run timer table.
local function scenario_no_one_shot_at_create(handles)
    local name, reg_err = register_lingering_adapter(handles, 'gauntlet_linger_127b')
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local before, berr = count_uv_handles()
    if before == nil then
        return driver_fail('uv-walk', berr)
    end
    local run_id, start_err = start_run(handles, name, 60000)
    if run_id == nil then
        return driver_fail('run', start_err)
    end
    local after, aerr = count_uv_handles()
    if after == nil then
        return driver_fail('uv-walk', aerr)
    end
    ev('uv timers before create=' .. before.timers .. ' after create=' .. after.timers)
    if after.timers ~= before.timers then
        return driver_fail(
            'timer-at-create',
            'create scheduled a uv timer — a deadline one-shot exists, contradicting the probed gap'
        )
    end
    local run = handles.supervisor.get(handles.sup, run_id)
    if run._timers ~= nil then
        return driver_fail('timers-table', 'run._timers exists — per-run timer tracking shipped')
    end
    ev('run._timers == nil; uv timer delta across create is zero')
    ev('file evidence: supervisor.lua M.create (lines 116-172) contains no vim.uv timer')
    return scenario_pass('create schedules no one-shot and tracks no timer handles')
end

---deadline-one-shot-absent (A): no one-shot fires at deadline_ns. The gap
---record.
local function scenario_deadline_one_shot_absent(handles)
    ev('file evidence: supervisor.lua M.create (lines 116-172) — no vim.uv timer scheduled')
    ev('file evidence: supervisor.lua M.tick (line 482) — deadline is a timestamp comparison only')
    ev('file evidence: zero vim.uv timer creations anywhere in lua/ai/harness/')
    local how = 'decision-3 gap: run create schedules no one-shot vim.uv timer at deadline_ns — the '
        .. 'deadline is enforced only when someone calls tick(). Phase-2 acceptance: create schedules a '
        .. "one-shot at deadline_ns -> wake(sup, now, 'deadline'); the handle is tracked in run._timers; "
        .. 'cancelled on terminal entry inside transition().'
    return gap('deadline-one-shot-absent', how)
end

---terminal-entry-no-timer-cleanup (A): transition() has no timer
---cancellation on terminal entry — the hygiene property holds vacuously
---today, and the airtight choke point has nothing to protect. The gap
---record.
local function scenario_terminal_entry_no_timer_cleanup(handles)
    local name, reg_err = register_lingering_adapter(handles, 'gauntlet_linger_127c')
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local run_id, start_err = start_run(handles, name, 5000)
    if run_id == nil then
        return driver_fail('run', start_err)
    end
    -- Finish before the deadline: with a real one-shot this must cancel it.
    local fok, ferr = handles.supervisor.finish(handles.sup, run_id, 'completed')
    if not fok then
        return driver_fail('finish', 'supervisor.finish failed: ' .. tostring(ferr))
    end
    local h1, herr = count_uv_handles()
    if h1 == nil then
        return driver_fail('uv-walk', herr)
    end
    local run = handles.supervisor.get(handles.sup, run_id)
    local acted = handles.supervisor.tick(handles.sup, run.deadline_ns + 1000000000)
    run = handles.supervisor.get(handles.sup, run_id)
    if run.state ~= 'completed' then
        return driver_fail('post-terminal', 'tick past deadline moved a finished run: ' .. run.state)
    end
    local h2, herr2 = count_uv_handles()
    if h2 == nil then
        return driver_fail('uv-walk', herr2)
    end
    ev('finished run + tick far past deadline: state stays completed, acted=' .. acted)
    ev('uv timers: ' .. h1.timers .. ' before terminal, ' .. h2.timers .. ' after — zero both ways')
    ev('file evidence: supervisor.lua transition() (lines 86-112) — no _timers cancellation on terminal entry')
    local how = 'decision-3 gap: transition() — the airtight choke point every terminal path goes through — '
        .. 'cancels no timer handles, because no run._timers table exists and no one-shots are scheduled. '
        .. 'The "zero live handles after terminal" property holds vacuously today. Phase-2 acceptance: every '
        .. 'one-shot handle is tracked in run._timers and cancelled inside transition() on terminal entry; '
        .. 'a leaked handle keeps the loop alive, so tests must assert zero live uv handles after run '
        .. 'completion (the handle-leak assertion).'
    return gap('timer-handle-hygiene-absent', how)
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'deadline-one-shot-absent'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return driver_fail('bootstrap', boot_err)
    end
    ev('diver probed: ' .. DIVER_SHA .. ' (main; no Phase-2 branch exists)')
    ev('scenario=' .. scenario)
    if scenario == 'deadline-fires-via-tick' then
        return scenario_deadline_fires_via_tick(handles)
    elseif scenario == 'no-one-shot-at-create' then
        return scenario_no_one_shot_at_create(handles)
    elseif scenario == 'deadline-one-shot-absent' then
        return scenario_deadline_one_shot_absent(handles)
    elseif scenario == 'terminal-entry-no-timer-cleanup' then
        return scenario_terminal_entry_no_timer_cleanup(handles)
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
