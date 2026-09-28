-- task-129 driver: approval-expiry one-shots (diver Phase-2 Decision 3,
-- event-driven supervision acceptance probe).
--
-- The design: an approval request schedules a one-shot at its expiry ->
-- wake(sup, now, 'approval') -> approval.sweep_expired; grant/deny cancels
-- the timer.
--
-- Diver probed: c84352cc850d507df477706b9166b6541ebe9e1c (main; no Phase-2
-- branch exists).
--
-- Scenarios via GAUNTLET_SCENARIO (default "approval-one-shot-absent"):
--   approval-expiry-via-tick            expiry swept only by an explicit
--                                       tick() (V, passes)
--   no-approval-timer                   request schedules no uv timer; the
--                                       approval entry carries no handle
--                                       (V, passes)
--   approval-one-shot-absent            no one-shot at approval expiry —
--                                       the gap record (A, gap)
--   grant-before-expiry-no-double-decision grant/deny races at T-eps:
--                                       tick-serialized, exactly one
--                                       decision, silent (A)
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
        id = 'task-129',
        outcome = 'fail',
        where = where,
        how = how,
        evidence = evidence,
    }
end

local function scenario_pass(note)
    ev(note)
    return { id = 'task-129', outcome = 'pass', evidence = evidence }
end

local function driver_fail(where, how)
    return { id = 'task-129', outcome = 'fail', where = where, how = how, evidence = evidence }
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
        approval = require('ai.harness.approval'),
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
        workflow = 'gauntlet_approval_flow',
        goal = 'probe approval expiry one-shot',
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

local function request_approval(handles, run_id, timeout_ms)
    local id, err = handles.approval.request(
        handles.sup.approvals,
        run_id,
        { tool = 'shell.exec', risk = 'process', summary = 'gauntlet probe' },
        { timeout_ms = timeout_ms }
    )
    if id == nil then
        return nil, 'approval.request failed: ' .. tostring(err)
    end
    return id
end

---approval-expiry-via-tick (V): an overdue approval is swept only by an
---explicit tick(); nothing fires at expiry. Characterization of today's
---sweep path — and of what it does NOT do to the run. Extended: two
---approvals on different runs with staggered expiries prove independent
---expiry and decision behavior with no cross-talk.
local function scenario_approval_expiry_via_tick(handles)
    local name, reg_err = register_lingering_adapter(handles, 'gauntlet_linger_129')
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local run_a, start_err_a = start_run(handles, name)
    if run_a == nil then
        return driver_fail('run-a', start_err_a)
    end
    local run_b, start_err_b = start_run(handles, name)
    if run_b == nil then
        return driver_fail('run-b', start_err_b)
    end
    local approval_a, req_err_a = request_approval(handles, run_a, 150)
    if approval_a == nil then
        return driver_fail('request-a', req_err_a)
    end
    local approval_b, req_err_b = request_approval(handles, run_b, 60000)
    if approval_b == nil then
        return driver_fail('request-b', req_err_b)
    end
    local ap_a = handles.approval.get(handles.sup.approvals, approval_a)
    ev('approval A (run A) timeout_ms=150; approval B (run B) timeout_ms=60000')
    if not wait_until_ns(ap_a.deadline_ns) then
        return driver_fail('wait', 'wall clock never passed approval A deadline_ns within the bounded wait')
    end
    ev('wall clock passed approval A expiry (B far in the future)')
    ap_a = handles.approval.get(handles.sup.approvals, approval_a)
    local ap_b = handles.approval.get(handles.sup.approvals, approval_b)
    if ap_a.state ~= 'pending' or ap_b.state ~= 'pending' then
        return driver_fail(
            'auto-sweep',
            'approval left pending WITHOUT any tick() — an expiry timer exists, contradicting the probed gap'
        )
    end
    ev('both approvals still pending with no tick() — nothing fired at expiry')
    local acted = handles.supervisor.tick(handles.sup, handles.types.now_ns())
    ev('explicit tick() acted=' .. acted)
    ap_a = handles.approval.get(handles.sup.approvals, approval_a)
    ap_b = handles.approval.get(handles.sup.approvals, approval_b)
    if ap_a.state ~= 'expired' then
        return driver_fail('sweep-a', 'tick() did not sweep overdue approval A: ' .. tostring(ap_a.state))
    end
    if ap_b.state ~= 'pending' then
        return driver_fail(
            'cross-talk',
            'tick() swept approval B before its expiry: ' .. tostring(ap_b.state) .. ' — cross-talk between runs'
        )
    end
    ev("approval A swept to 'expired'; approval B still 'pending' — independent expiry, no cross-talk")
    local ra = handles.supervisor.get(handles.sup, run_a)
    local rb = handles.supervisor.get(handles.sup, run_b)
    if ra.state ~= 'running' or rb.state ~= 'running' then
        return driver_fail(
            'run-touched',
            'sweep changed a run state: A=' .. tostring(ra.state) .. ' B=' .. tostring(rb.state)
        )
    end
    ev("both runs untouched by the sweep (still 'running')")
    -- Decide B while A stays expired: decisions are per-approval too.
    local dok, derr = handles.approval.decide(handles.sup.approvals, approval_b, 'approved', 'gauntlet')
    if not dok then
        return driver_fail('decide-b', 'decide failed: ' .. tostring(derr))
    end
    handles.supervisor.tick(handles.sup, handles.types.now_ns())
    ap_a = handles.approval.get(handles.sup.approvals, approval_a)
    ap_b = handles.approval.get(handles.sup.approvals, approval_b)
    if ap_b.state ~= 'approved' then
        return driver_fail('double-decision', 'grant of B was overwritten by a later sweep: ' .. tostring(ap_b.state))
    end
    if ap_a.state ~= 'expired' then
        return driver_fail('cross-talk-2', 'deciding B changed approval A: ' .. tostring(ap_a.state))
    end
    ev("B granted and stays 'approved' across a later tick; A stays 'expired' — independent decisions")
    ev('note: expiry-means-denied is documented (approval.lua) but no run transition or outcome event follows')
    ev('source: supervisor.lua M.tick approval sweep (line 472), approval.sweep_expired (approval.lua:132)')
    return scenario_pass('approval expiry swept only by explicit tick(); the run itself is untouched — per-run approvals independent, no cross-talk')
end

---no-approval-timer (V): requesting an approval schedules no uv timer and
---the approval entry carries no timer handle.
local function scenario_no_approval_timer(handles)
    local name, reg_err = register_lingering_adapter(handles, 'gauntlet_linger_129b')
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local run_id, start_err = start_run(handles, name)
    if run_id == nil then
        return driver_fail('run', start_err)
    end
    local before, berr = count_uv_handles()
    if before == nil then
        return driver_fail('uv-walk', berr)
    end
    local approval_id, req_err = request_approval(handles, run_id, 60000)
    if approval_id == nil then
        return driver_fail('request', req_err)
    end
    local after, aerr = count_uv_handles()
    if after == nil then
        return driver_fail('uv-walk', aerr)
    end
    ev('uv timers before request=' .. before.timers .. ' after request=' .. after.timers)
    if after.timers ~= before.timers then
        return driver_fail(
            'timer-at-request',
            'approval.request scheduled a uv timer — an expiry one-shot exists, contradicting the probed gap'
        )
    end
    local approval = handles.approval.get(handles.sup.approvals, approval_id)
    if approval.timer ~= nil or approval._timer ~= nil then
        return driver_fail('timer-field', 'approval entry carries a timer handle — expiry tracking shipped')
    end
    ev('approval entry has no timer handle; uv timer delta across request is zero')
    ev('file evidence: approval.lua M.request (lines 40-85) schedules nothing')
    return scenario_pass('approval request schedules no one-shot and tracks no handle')
end

---approval-one-shot-absent (A): no one-shot fires at approval expiry. The
---gap record — including the exact hook location the design asks for.
local function scenario_approval_one_shot_absent(handles)
    ev('file evidence: approval.lua M.request (lines 40-85) — the hook point where Phase-2 schedules timers; schedules nothing today')
    ev('file evidence: supervisor.lua M.tick (line 472) — expiry handled by timestamp sweep only')
    ev('file evidence: approval.lua M.decide (lines 88-107) — a silent state change; no outcome event, no timer to cancel')
    ev('file evidence: zero vim.uv timer creations anywhere in lua/ai/harness/')
    local how = 'decision-3 gap: an approval request schedules no one-shot vim.uv timer at its expiry — '
        .. 'expiry is noticed only when someone calls tick(). Phase-2 acceptance: M.request schedules a '
        .. "one-shot at approval expiry -> wake(sup, now, 'approval') -> approval.sweep_expired; grant/deny "
        .. 'cancels the timer; grant at T-eps finds nothing to sweep (no double-decision, exactly one '
        .. "approval-outcome event); two pending approvals on different runs expire independently (run A's "
        .. "expiry never sweeps run B's approval)."
    return gap('approval-one-shot-absent', how)
end

---grant-before-expiry-no-double-decision (A): human-speed decisions racing
---machine-speed expiry are tick-serialized today — grant at T-eps and deny
---then expiry both resolve cleanly with exactly one decision and silence
---otherwise.
local function scenario_grant_before_expiry(handles)
    local name, reg_err = register_lingering_adapter(handles, 'gauntlet_linger_129c')
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local run_id, start_err = start_run(handles, name)
    if run_id == nil then
        return driver_fail('run', start_err)
    end
    -- Race 1: grant just before expiry.
    local id1, err1 = request_approval(handles, run_id, 300)
    if id1 == nil then
        return driver_fail('request', err1)
    end
    local events_before = handles.sink:count()
    local dok, derr = handles.approval.decide(handles.sup.approvals, id1, 'approved', 'gauntlet')
    if not dok then
        return driver_fail('decide', 'decide failed: ' .. tostring(derr))
    end
    local a1 = handles.approval.get(handles.sup.approvals, id1)
    if not wait_until_ns(a1.deadline_ns) then
        return driver_fail('wait', 'wall clock never passed expiry within the bounded wait')
    end
    handles.supervisor.tick(handles.sup, handles.types.now_ns())
    a1 = handles.approval.get(handles.sup.approvals, id1)
    if a1.state ~= 'approved' then
        return driver_fail('double-decision', 'grant at T-eps was overwritten by the sweep: ' .. a1.state)
    end
    ev("grant at T-eps: sweep finds nothing pending; state stays 'approved' — no double-decision")
    -- Race 2: deny, then expiry passes — no phantom wake from a timer.
    local id2, err2 = request_approval(handles, run_id, 150)
    if id2 == nil then
        return driver_fail('request', err2)
    end
    local dok2, derr2 = handles.approval.decide(handles.sup.approvals, id2, 'denied', 'gauntlet')
    if not dok2 then
        return driver_fail('decide', 'deny failed: ' .. tostring(derr2))
    end
    local a2 = handles.approval.get(handles.sup.approvals, id2)
    if not wait_until_ns(a2.deadline_ns) then
        return driver_fail('wait', 'wall clock never passed expiry within the bounded wait')
    end
    handles.supervisor.tick(handles.sup, handles.types.now_ns())
    a2 = handles.approval.get(handles.sup.approvals, id2)
    if a2.state ~= 'denied' then
        return driver_fail('deny-overwrite', 'deny was overwritten after expiry: ' .. a2.state)
    end
    local events_after = handles.sink:count()
    if events_after ~= events_before then
        return driver_fail(
            'silent',
            'decide/sweep appended ' .. (events_after - events_before) .. ' sink events — decisions are not silent'
        )
    end
    ev("deny then expiry: state stays 'denied'; decide and sweep append zero sink events (silent)")
    ev('note: the races are tick-serialized today — no timer exists to race against; the hazard is banked, not live')
    return scenario_pass('grant/deny at T-eps resolve cleanly: no double-decision, no phantom expiry, silent')
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'approval-one-shot-absent'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return driver_fail('bootstrap', boot_err)
    end
    ev('diver probed: ' .. DIVER_SHA .. ' (main; no Phase-2 branch exists)')
    ev('scenario=' .. scenario)
    if scenario == 'approval-expiry-via-tick' then
        return scenario_approval_expiry_via_tick(handles)
    elseif scenario == 'no-approval-timer' then
        return scenario_no_approval_timer(handles)
    elseif scenario == 'approval-one-shot-absent' then
        return scenario_approval_one_shot_absent(handles)
    elseif scenario == 'grant-before-expiry-no-double-decision' then
        return scenario_grant_before_expiry(handles)
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
