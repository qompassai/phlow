-- task-122 driver: capability-based risk escalation acceptance probe
-- (diver Phase-2 Decision 1, Fix 3 detail).
--
-- The design: launch classifies risk from the adapter's pcall'd probe()
-- capabilities — `remote = true` -> risk 'network', else 'process' — and
-- consults policy.decide with the built request before starting the
-- adapter. A broken probe cannot crash classification: failures fall back
-- to 'network' (fail-closed).
--
-- Diver probed: c84352cc850d507df477706b9166b6541ebe9e1c (main; no Phase-2
-- branch exists).
--
-- Scenarios via GAUNTLET_SCENARIO (default "remote-true"):
--   remote-true      probe attests remote=true; launch builds no policy
--                    request (V, gap; acceptance: risk 'network')
--   remote-false     probe attests remote=false; launch builds no policy
--                    request (V, gap; acceptance: risk 'process')
--   probe-raises     probe() raises; launch proceeds with no fail-closed
--                    fallback (A, gap; acceptance: pcall -> 'network')
--   malformed-probe  malformed caps (remote='yes', non-table) and a
--                    probeless adapter: strict contract rejects them, but
--                    launch still classifies nothing (A, gap; acceptance:
--                    every malformed shape -> 'network')
--
-- Every scenario records the same precise gap — launch builds no policy
-- request at all — with scenario-specific evidence. The record IS the
-- Phase-2 acceptance artifact. The trust-boundary finding from task-118
-- (a lying probe attests remote=false while doing network I/O) is out of
-- scope here: classification trusts attestation by design; vetting is
-- adapter-vetting work, not a launch-path fix.
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

local function gap(how)
    return {
        id = 'task-122',
        outcome = 'fail',
        where = 'no-launch-policy-request',
        how = how,
        evidence = evidence,
    }
end

local function driver_fail(where, how)
    return { id = 'task-122', outcome = 'fail', where = where, how = how, evidence = evidence }
end

---Wire up the harness. Returns handles or (nil, err).
---Resolve DIVER_LUA_DIR to the runtimepath root and the lua/ dir beneath
---it. Accepts either the rtp root itself (holding lua/ai/harness/init.lua)
---or the lua/ dir directly (holding ai/harness/init.lua): the rtp entry
---must be the directory that *contains* lua/, or require() never fires.
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
        return parent, diver_lua_dir:gsub('/+$', '')
    end
    return nil, nil
end

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
        registry = st.registry,
        registry_mod = require('ai.harness.registry'),
        supervisor = require('ai.harness.supervisor'),
        adapter_mod = require('ai.harness.adapter'),
        policy_mod = require('ai.harness.policy'),
    }
end

---Register a stub adapter whose probe() is scripted and counted.
---Returns (name, spy) or (nil, err). The adapter completes its run at
---start so the full create -> launch -> adapter path is exercised.
local function register_spy_adapter(handles, name, probe_fn, spy)
    local adapter = { name = name }
    function adapter.probe()
        spy.probe_calls = spy.probe_calls + 1
        return probe_fn()
    end
    function adapter.start(run, sink)
        sink:append(run.id, 'model.completed', { outcome = 'completed' }, { source = name })
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

---Wrap policy.decide with a call-counting spy. Returns the counter table.
local function spy_on_decide(handles)
    local counter = { decide_calls = 0 }
    local orig = handles.policy_mod.decide
    handles.policy_mod.decide = function(...)
        counter.decide_calls = counter.decide_calls + 1
        return orig(...)
    end
    return counter
end

local function base_spec(adapter_name)
    return {
        workflow = 'gauntlet_probe_flow',
        goal = 'classify this run',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = adapter_name,
        timeout_ms = 30000,
    }
end

---Full probe capability table with strict booleans (the contract
---ai.harness.adapter.probe enforces per types.CAPABILITY_KEYS).
local function caps_with(remote)
    return {
        streaming = false,
        cancellation = true,
        resume = false,
        permissions = false,
        artifacts = false,
        remote = remote,
        tools = false,
    }
end

---Launch one run against the named stub adapter and report whether the
---launch path classified anything: probe-call count and decide-call
---count. Returns a gap verdict naming the scenario's acceptance criterion.
local function launch_without_classification(handles, adapter_name, spy, acceptance)
    local decide_counter = spy_on_decide(handles)
    local run_id, run_err = handles.harness.run(base_spec(adapter_name))
    if run_id == nil then
        return driver_fail('run', 'harness.run failed: ' .. tostring(run_err))
    end
    local run = handles.supervisor.get(handles.sup, run_id)
    ev('adapter probe() calls during launch: ' .. tostring(spy.probe_calls))
    ev('policy.decide calls during launch: ' .. tostring(decide_counter.decide_calls))
    ev('run state after launch: ' .. tostring(run and run.state or 'nil'))
    if spy.probe_calls ~= 0 then
        return driver_fail(
            'classification',
            'launch called probe() ' .. spy.probe_calls .. 'x — expected no classification today'
        )
    end
    if decide_counter.decide_calls ~= 0 then
        return driver_fail(
            'classification',
            'policy.decide was consulted ' .. decide_counter.decide_calls .. 'x — expected no consultation today'
        )
    end
    ev('GAP today: launch (supervisor.lua line 182) builds no policy request at all')
    ev('file evidence: launch body has no probe() call, no risk request, no policy.decide call')
    ev('file evidence: negotiation calls adapter.probe only when NO adapter name is given')
    local how = 'fix-3 detail absent: ' .. acceptance .. ' Today launch resolves the adapter by name '
        .. 'and calls chosen.start directly — no capabilities are classified, no request is built, '
        .. 'decide is never consulted. Phase-2 acceptance: launch pcall()s probe(), classifies '
        .. "remote=true -> risk 'network' else 'process', consults decide at launch time, and a deny "
        .. 'lands the run in failed with the policy reason.'
    return gap(how)
end

---remote-true (V): remote=true attestation builds no 'network' request.
local function scenario_remote_true(handles)
    local spy = { probe_calls = 0 }
    local name, err = register_spy_adapter(handles, 'gauntlet_remote', function()
        return caps_with(true)
    end, spy)
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(err))
    end
    return launch_without_classification(
        handles,
        name,
        spy,
        "Phase-2 classifies remote=true -> request.risk='network'."
    )
end

---remote-false (V): remote=false attestation builds no 'process' request.
local function scenario_remote_false(handles)
    local spy = { probe_calls = 0 }
    local name, err = register_spy_adapter(handles, 'gauntlet_local', function()
        return caps_with(false)
    end, spy)
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(err))
    end
    return launch_without_classification(
        handles,
        name,
        spy,
        "Phase-2 classifies remote=false -> request.risk='process'."
    )
end

---probe-raises (A): a raising probe() gets no fail-closed 'network'
---classification — launch proceeds with no classification at all.
local function scenario_probe_raises(handles)
    local spy = { probe_calls = 0 }
    local name, err = register_spy_adapter(handles, 'gauntlet_broken_probe', function()
        error('probe exploded')
    end, spy)
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(err))
    end
    return launch_without_classification(
        handles,
        name,
        spy,
        'Phase-2 wraps probe() in pcall so a raising probe falls back to risk '
            .. "'network' (fail-closed). Today launch never calls probe() on the named adapter, so a "
            .. 'broken probe is invisible AND unclassified. Phase-2 acceptance: probe raising -> '
            .. "'network', no crash, decide consulted."
    )
end

---malformed-probe (A): malformed capability shapes are rejected by the
---probe contract (strict booleans), but launch classifies nothing either
---way; a probeless adapter cannot even register.
local function scenario_malformed(handles)
    local adapter_mod = handles.adapter_mod
    local function shell(name, probe_fn)
        return {
            name = name,
            probe = probe_fn,
            start = function()
                return nil, 'unused'
            end,
            cancel = function()
                return true
            end,
            close = function() end,
        }
    end

    -- remote='yes' (truthy non-boolean): the contract rejects it.
    local bad_truthy = shell('gauntlet_truthy', function()
        local c = caps_with(false)
        c.remote = 'yes'
        return c
    end)
    local _, truthy_err = adapter_mod.probe(bad_truthy)
    if truthy_err == nil or not truthy_err:find('must be a boolean', 1, true) then
        return driver_fail(
            'contract',
            'M.probe accepted remote=\'yes\': ' .. vim.inspect(truthy_err)
        )
    end
    ev("contract: M.probe rejects remote='yes' ('" .. truthy_err .. "') — strict boolean, not truthiness")

    -- non-table caps: the contract rejects them.
    local bad_shape = shell('gauntlet_shape', function()
        return 'not a table'
    end)
    local _, shape_err = adapter_mod.probe(bad_shape)
    if shape_err == nil or not shape_err:find('must return a table', 1, true) then
        return driver_fail('contract', 'M.probe accepted non-table caps: ' .. vim.inspect(shape_err))
    end
    ev("contract: M.probe rejects non-table caps ('" .. shape_err .. "')")

    -- remote key missing (nil): the contract rejects it.
    local bad_nil = shell('gauntlet_nilremote', function()
        local c = caps_with(false)
        c.remote = nil
        return c
    end)
    local _, nil_err = adapter_mod.probe(bad_nil)
    if nil_err == nil or not nil_err:find('must be a boolean', 1, true) then
        return driver_fail('contract', 'M.probe accepted remote=nil: ' .. vim.inspect(nil_err))
    end
    ev("contract: M.probe rejects remote=nil ('" .. nil_err .. "')")

    -- no probe function at all: registration itself rejects.
    local probeless = { name = 'gauntlet_noprobe' }
    function probeless.start()
        return nil, 'unused'
    end
    function probeless.cancel()
        return true
    end
    function probeless.close() end
    local ok, reg_err =
        handles.registry_mod.register_adapter(handles.registry, 'gauntlet_noprobe', probeless)
    if ok then
        return driver_fail('contract', 'registry accepted an adapter with no probe function')
    end
    ev("contract: register_adapter rejects probeless adapter ('" .. tostring(reg_err) .. "')")

    -- And launch classifies none of these shapes: a valid-probe adapter
    -- launches with zero classification, as in the other scenarios.
    local spy = { probe_calls = 0 }
    local name, lerr = register_spy_adapter(handles, 'gauntlet_malformed_launch', function()
        return caps_with(false)
    end, spy)
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(lerr))
    end
    local decide_counter = spy_on_decide(handles)
    local run_id, run_err = handles.harness.run(base_spec(name))
    if run_id == nil then
        return driver_fail('run', 'harness.run failed: ' .. tostring(run_err))
    end
    ev('adapter probe() calls during launch: ' .. tostring(spy.probe_calls))
    ev('policy.decide calls during launch: ' .. tostring(decide_counter.decide_calls))
    if spy.probe_calls ~= 0 or decide_counter.decide_calls ~= 0 then
        return driver_fail('classification', 'launch classified something — expected no classification today')
    end
    ev('GAP today: launch builds no policy request for any probe shape — malformed or not')
    ev('file evidence: supervisor.lua launch (line 182); adapter.lua M.probe (strict contract, unused by launch)')
    local how = 'fix-3 detail absent: Phase-2 maps EVERY malformed shape (probe raising, remote=nil, '
        .. "non-table caps, missing probe fn, truthy non-boolean) -> risk 'network' (fail-closed) via "
        .. 'pcall + strict boolean check. Today the strict contract exists (adapter.lua M.probe) but '
        .. 'launch never invokes it and builds no request. Phase-2 acceptance: all four malformed '
        .. "shapes classify 'network'; decide consulted at launch."
    return gap(how)
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'remote-true'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return driver_fail('bootstrap', boot_err)
    end
    ev('diver probed: ' .. DIVER_SHA .. ' (main; no Phase-2 branch exists)')
    ev('scenario=' .. scenario)
    if scenario == 'remote-true' then
        return scenario_remote_true(handles)
    elseif scenario == 'remote-false' then
        return scenario_remote_false(handles)
    elseif scenario == 'probe-raises' then
        return scenario_probe_raises(handles)
    elseif scenario == 'malformed-probe' then
        return scenario_malformed(handles)
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
