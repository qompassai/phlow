-- task-120 driver: M.run failure-path legality probe (diver Fix 6).
--
-- The defect: `init.lua M.run` calls `supervisor.finish(..., 'failed')`
-- for ANY failure. But if the failure happened BEFORE the created→queued
-- transition, finish attempts created→failed, which is ILLEGAL
-- (types.TRANSITIONS.created permits only queued/cancelled) — the run
-- emits a spurious diagnostic.invalid_transition and the intended failure
-- outcome is lost.
--
-- Diver probed: c84352cc850d507df477706b9166b6541ebe9e1c (main; no Phase-2
-- branch exists).
--
-- Scenarios via GAUNTLET_SCENARIO (default "default"):
--   default             unknown adapter: start_run fails AFTER the run is
--                       queued; finish queued->failed is legal and the
--                       reason is preserved (V)
--   queued-failure      instant adapter whose start errors after the run
--                       is queued: the failure reason is preserved (V)
--   pre-queued-failure  an adapter-name lookup that fails before the
--                       created->queued transition: no diagnostic is
--                       emitted and the run stays created (A)
--   not-set-up         init.lua M.run before setup returns the clear
--                       "not set up" error; an adapter start that RAISES
--                       stays contained (A)
--
-- A scenario reports outcome='fail', where='fix-6-absent' when its probes
-- show the illegal-transition path (spurious diagnostic, lost reason);
-- 'pass' when every probe holds. Today the unknown-adapter path passes
-- (queued->failed is legal); the pre-queued path is where the defect
-- bites. The record IS the Phase-2 acceptance artifact.
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
    return { id = 'task-120', outcome = 'fail', where = 'fix-6-absent', how = how, evidence = evidence }
end

local function scenario_pass(note)
    ev(note)
    return { id = 'task-120', outcome = 'pass', evidence = evidence }
end

local function driver_fail(where, how)
    return { id = 'task-120', outcome = 'fail', where = where, how = how, evidence = evidence }
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
    vim.opt.runtimepath:append(diver_lua_dir)
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
        registry = st.registry,
        registry_mod = require('ai.harness.registry'),
        supervisor = require('ai.harness.supervisor'),
        types = require('ai.harness.types'),
    }
end

local function base_spec(adapter_name)
    return {
        workflow = 'gauntlet_fail_flow',
        goal = 'failure-path probe',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = adapter_name,
        timeout_ms = 30000,
    }
end

local function find_run_id_by_diagnostic(handles, wanted_fragment)
    -- The failure-path defect surfaces as a diagnostic event; find the run
    -- that owns the most recent invalid_transition diagnostic. The real
    -- event kind is 'diagnostic.observed' with payload.kind =
    -- 'invalid_transition' (supervisor.lua transition()).
    local matches = {}
    for run_id, _ in pairs(handles.sup.runs) do
        for _, event in ipairs(handles.sink:events(run_id)) do
            if event.kind == 'diagnostic.observed' then
                local payload = event.payload or {}
                if payload.kind == 'invalid_transition' then
                    local from = tostring(payload.from or '')
                    if wanted_fragment == nil or from:find(wanted_fragment, 1, true) ~= nil then
                        matches[#matches + 1] = run_id
                    end
                end
            end
        end
    end
    return matches
end

local function defect_how()
    return 'fix-6 absent: init.lua M.run calls supervisor.finish(..., "failed") for ANY '
        .. 'failure (init.lua line 83), including failures that happened BEFORE the '
        .. 'created->queued transition inside start_run. finish then attempts the illegal '
        .. 'created->failed transition (types.TRANSITIONS.created permits only '
        .. 'queued/cancelled), emitting a spurious diagnostic.invalid_transition and '
        .. 'losing the intended failure outcome. Phase-2 acceptance: finish is legal '
        .. 'when the failure happened after the run was queued (reason preserved); no '
        .. 'diagnostic is emitted and no illegal transition attempted when the failure '
        .. 'predates the queued transition.'
end

---default (V): unknown adapter — start_run fails after the run is queued;
---queued->failed is legal and the reason is preserved.
local function scenario_default(handles)
    local run_id, run_err = handles.harness.run(base_spec('no_such_adapter_xyz'))
    ev('harness.run with an unknown adapter -> run_id=' .. tostring(run_id) .. ' err=' .. tostring(run_err))
    -- The run was created, so the harness holds it; find the created run.
    local created = nil
    for id, run in pairs(handles.sup.runs) do
        if run.adapter == 'no_such_adapter_xyz' then
            created = run
            run_id = id
            break
        end
    end
    if created == nil then
        return driver_fail('probe', 'no run record for the unknown-adapter spec')
    end
    ev('run state=' .. tostring(created.state))
    local diag = find_run_id_by_diagnostic(handles, nil)
    local spurious = #diag
    if created.state == 'failed' and spurious == 0 then
        return scenario_pass('queued->failed is legal: failure reason preserved, no spurious diagnostic')
    end
    ev('FAIL today: expected failed with no diagnostic; got state=' .. tostring(created.state)
        .. ' spurious-diagnostics=' .. spurious)
    return fix_absent(defect_how())
end

---queued-failure (V): adapter start errors after the run is queued; the
---failure reason is preserved.
local function scenario_queued_failure(handles)
    local adapter = { name = 'gauntlet_blowup' }
    function adapter.probe()
        return { available = true, remote = false }
    end
    function adapter.start(_run, _sink)
        return nil, 'adapter start blew up'
    end
    function adapter.cancel(_handle)
        return true
    end
    function adapter.close(_handle) end
    local ok, reg_err = handles.registry_mod.register_adapter(handles.registry, 'gauntlet_blowup', adapter)
    if not ok then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local run_id, run_err = handles.harness.run(base_spec('gauntlet_blowup'))
    ev('harness.run with a failing adapter -> run_id=' .. tostring(run_id) .. ' err=' .. tostring(run_err))
    -- launch() itself transitions the run to failed when start returns
    -- nil, so M.run reports (nil, err); find the run by adapter name.
    local found_id, run = nil, nil
    for id, candidate in pairs(handles.sup.runs) do
        if candidate.adapter == 'gauntlet_blowup' then
            found_id, run = id, candidate
            break
        end
    end
    if run == nil then
        return driver_fail('run', 'no run record for the failing-adapter spec')
    end
    run_id = found_id
    local has_reason = false
    for _, event in ipairs(handles.sink:events(run_id)) do
        if event.kind == 'run.finished' and type(event.payload) == 'table' then
            if type(event.payload.reason) == 'string' and event.payload.reason ~= '' then
                has_reason = true
                ev('run.finished reason: ' .. event.payload.reason)
            end
        end
    end
    if run.state == 'failed' and has_reason then
        return scenario_pass('start-return failure after queued preserved the failure reason')
    end
    ev('FAIL today: expected failed with a preserved reason; got state=' .. tostring(run.state))
    return fix_absent(defect_how())
end

---pre-queued-failure (A): a failure that predates the created->queued
---transition must not emit a diagnostic and must leave the run in created.
local function scenario_pre_queued(handles)
    -- Force the failure before the queued transition by making launch's
    -- adapter lookup fail: register nothing named gauntlet_missing and call
    -- supervisor.create + a direct transition check.
    local run, create_err = handles.supervisor.create(handles.sup, base_spec('gauntlet_missing'))
    if run == nil then
        return driver_fail('create', 'supervisor.create failed: ' .. tostring(create_err))
    end
    local run_id = run.id
    ev('run created: state=' .. tostring(run.state))
    -- Simulate a pre-queued failure exactly the way M.run does today:
    -- call finish as if the start had failed, while the run is still created.
    local ok, finish_err = handles.supervisor.finish(handles.sup, run_id, 'failed', 'pre-queued failure probe')
    ev('finish(created -> failed) -> ok=' .. tostring(ok) .. ' err=' .. tostring(finish_err))
    local after = handles.supervisor.get(handles.sup, run_id)
    local diag = find_run_id_by_diagnostic(handles, nil)
    local spurious = #diag
    ev('run state now=' .. tostring(after.state) .. ' spurious-diagnostics=' .. spurious)
    if after.state == 'created' and spurious == 0 then
        return scenario_pass('pre-queued failure is legal: run stays created, no diagnostic emitted')
    end
    ev('FAIL today: finish attempted the illegal created->failed transition —')
    ev('spurious diagnostic.invalid_transition emitted and the intended outcome is lost.')
    return fix_absent(defect_how())
end

---not-set-up (A): M.run before setup returns the clear error; an adapter
---start that RAISES stays contained (no driver crash, harness intact).
local function scenario_not_set_up()
    local diver_lua_dir = vim.env.DIVER_LUA_DIR
    if type(diver_lua_dir) ~= 'string' or diver_lua_dir == '' then
        return driver_fail('bootstrap', 'DIVER_LUA_DIR is not set')
    end
    vim.opt.runtimepath:append(diver_lua_dir)
    local harness = require('ai.harness')
    -- Do NOT call setup: M.run must refuse with the clear error.
    local run_id, err = harness.run({
        workflow = 'gauntlet_nosetup_flow',
        goal = 'not set up probe',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = 'gauntlet_never',
    })
    if run_id ~= nil then
        return driver_fail('not-set-up', 'M.run succeeded without setup')
    end
    ev('M.run before setup -> ' .. tostring(err))
    local clear = type(err) == 'string' and err:find('not set up', 1, true) ~= nil
    if not clear then
        return driver_fail('not-set-up', 'unclear error before setup: ' .. tostring(err))
    end
    -- Now set up and register a start that RAISES: launch calls
    -- chosen.start with no pcall (supervisor.lua launch, lines 182-224),
    -- so today the raise propagates out of harness.run instead of being
    -- contained. The driver catches it with pcall and records the gap.
    local setup_ok, setup_err = harness.setup({})
    if not setup_ok then
        return driver_fail('setup', 'harness.setup failed: ' .. tostring(setup_err))
    end
    local st = harness._state
    local registry_mod = require('ai.harness.registry')
    local supervisor = require('ai.harness.supervisor')
    local adapter = { name = 'gauntlet_raiser' }
    function adapter.probe()
        return { available = true, remote = false }
    end
    function adapter.start(_run, _sink)
        error('adapter start exploded')
    end
    function adapter.cancel(_handle)
        return true
    end
    function adapter.close(_handle) end
    local reg_ok, reg_err = registry_mod.register_adapter(st.registry, 'gauntlet_raiser', adapter)
    if not reg_ok then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local run_ok, run_id2, err2 = pcall(harness.run, {
        workflow = 'gauntlet_raise_flow',
        goal = 'raising adapter probe',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = 'gauntlet_raiser',
    })
    if not run_ok then
        ev('FAIL today: the raising adapter start propagated out of harness.run uncaught:')
        ev(tostring(run_id2))
        return fix_absent(
            'fix-6 absent (containment half): launch calls chosen.start without pcall '
                .. '(supervisor.lua launch, lines 182-224), so a raising adapter start '
                .. 'propagates out of harness.run instead of landing the run in failed. '
                .. 'Phase-2 acceptance: M.run contains start errors and finishes the run '
                .. 'legally with the reason preserved.'
        )
    end
    ev('harness.run with a raising adapter -> run_id=' .. tostring(run_id2) .. ' err=' .. tostring(err2))
    if run_id2 == nil then
        return scenario_pass('not-set-up gives the clear error; the raising start stayed contained')
    end
    local run = supervisor.get(st.supervisor, run_id2)
    ev('run state after raising start: ' .. tostring(run.state))
    if run.state == 'failed' then
        return scenario_pass('not-set-up gives the clear error; the raising start stayed contained (run failed)')
    end
    return driver_fail('contained', 'raising start left the run in ' .. tostring(run.state))
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'default'
    end
    if scenario == 'not-set-up' then
        return scenario_not_set_up()
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return driver_fail('bootstrap', boot_err)
    end
    ev('diver probed: ' .. DIVER_SHA .. ' (main; no Phase-2 branch exists)')
    ev('scenario=' .. scenario)
    if scenario == 'default' then
        return scenario_default(handles)
    elseif scenario == 'queued-failure' then
        return scenario_queued_failure(handles)
    elseif scenario == 'pre-queued-failure' then
        return scenario_pre_queued(handles)
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
