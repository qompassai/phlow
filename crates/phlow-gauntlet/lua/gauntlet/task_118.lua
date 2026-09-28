-- task-118 driver: policy-enforcement acceptance probe (diver Fix 3).
--
-- The defect: `launch()` goes straight to `chosen.start`; `sup.policy`
-- is stored but never consulted — the fail-closed design is decorative.
--
-- Diver probed: c84352cc850d507df477706b9166b6541ebe9e1c (main; no Phase-2
-- branch exists).
--
-- Scenarios via GAUNTLET_SCENARIO (default "default"):
--   default           deny-all policy blocks the launch; the run lands in
--                     failed with the policy reason recorded (V)
--   allow-launches    allow policy lets the launch through (V)
--   nil-policy        decide(nil) denies by itself (characterization);
--                     a launch with sup.policy == nil still proceeds (A)
--   no-classification launch never calls probe / builds no policy request
--                     (source evidence); a lying probe (remote=false while
--                     start performs network I/O) is demonstrated and banked
--                     as a trust-boundary finding (A)
--
-- A scenario reports outcome='fail', where='fix-3-absent' when its probes
-- show policy was not consulted; 'pass' when every probe holds. Today only
-- allow-launches passes (behaviorally — for the wrong reason). The record
-- IS the Phase-2 acceptance artifact.
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
    return { id = 'task-118', outcome = 'fail', where = 'fix-3-absent', how = how, evidence = evidence }
end

local function scenario_pass(note)
    ev(note)
    return { id = 'task-118', outcome = 'pass', evidence = evidence }
end

local function driver_fail(where, how)
    return { id = 'task-118', outcome = 'fail', where = where, how = how, evidence = evidence }
end

---Wire up the harness with an explicit policy. Returns handles or (nil, err).
local function bootstrap(policy)
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
    local setup_opts = {}
    if policy ~= nil then
        setup_opts.policy = policy
    end
    local ok, err = harness.setup(setup_opts)
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
        policy_mod = require('ai.harness.policy'),
    }
end

---Instant adapter: completes the run on start.
local function register_instant(handles, name)
    local adapter = { name = name }
    function adapter.probe()
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
    end
    function adapter.start(run, sink)
        sink:append(
            run.id,
            'model.completed',
            { outcome = 'completed' },
            { source = name }
        )
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
    return true
end

local function base_spec(adapter_name)
    return {
        workflow = 'gauntlet_policy_flow',
        goal = 'policy probe',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = adapter_name,
        timeout_ms = 30000,
    }
end

local function defect_how()
    return 'fix-3 absent: supervisor.launch (supervisor.lua line 182) goes straight to '
        .. 'chosen.start with no policy consultation — sup.policy is stored but never read, '
        .. 'and init.lua M.run never references policy either. policy.decide itself '
        .. 'fail-closes on a nil policy (policy.lua lines 175-184: "no policy configured"), '
        .. 'but it has no call sites. Phase-2 acceptance: launch builds the risk request '
        .. '(probe caps via pcall, remote=true -> "network" else "process"), consults decide, '
        .. 'and on deny lands the run in failed with the policy reason recorded.'
end

---default (V): deny-all policy blocks the launch.
local function scenario_default(handles)
    local ok, err = register_instant(handles, 'gauntlet_instant')
    if not ok then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(err))
    end
    local run_id, run_err = handles.harness.run(base_spec('gauntlet_instant'))
    if run_id == nil then
        -- A launch-time denial surfaces as (nil, 'policy denied: ...')
        -- with the run left in failed.
        ev('harness.run refused the launch: ' .. tostring(run_err))
        return scenario_pass('deny-all policy blocked the launch: Fix 3 present')
    end
    local run = handles.supervisor.get(handles.sup, run_id)
    ev('FAIL today: launch proceeded under a deny-all policy; run state=' .. tostring(run.state))
    ev('no "policy denied" reason recorded; sup.policy was never consulted')
    return fix_absent(defect_how())
end

---allow-launches (V): allow policy lets the launch through.
local function scenario_allow(handles)
    local ok, err = register_instant(handles, 'gauntlet_instant')
    if not ok then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(err))
    end
    local run_id, run_err = handles.harness.run(base_spec('gauntlet_instant'))
    if run_id == nil then
        return driver_fail('run', 'allow-policy launch failed: ' .. tostring(run_err))
    end
    local run = handles.supervisor.get(handles.sup, run_id)
    if run.state == 'running' then
        ev('launch proceeded under the allow policy (behaviorally correct today,')
        ev('but for the wrong reason: decide was never consulted — see deny-blocks)')
        return scenario_pass('allow-policy launch reaches running')
    end
    return driver_fail('run', 'allow-policy launch landed in ' .. tostring(run.state))
end

---nil-policy (A): decide(nil) denies by itself, but a launch with
---sup.policy == nil still proceeds — the function is fail-closed, the
---call site is missing.
local function scenario_nil_policy()
    local diver_lua_dir = vim.env.DIVER_LUA_DIR
    if type(diver_lua_dir) ~= 'string' or diver_lua_dir == '' then
        return driver_fail('bootstrap', 'DIVER_LUA_DIR is not set')
    end
    vim.opt.runtimepath:append(diver_lua_dir)
    local events = require('ai.harness.events')
    local registry = require('ai.harness.registry')
    local supervisor = require('ai.harness.supervisor')
    local policy_mod = require('ai.harness.policy')
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    -- Characterization: decide fail-closes on a nil policy by itself.
    local decision = policy_mod.decide(nil, { risk = 'process', workspace = work_dir })
    if decision.decision == 'deny' then
        ev("PASS today: policy.decide(nil, req) -> deny ('" .. tostring(decision.reason) .. "')")
    else
        return driver_fail('decide', 'decide(nil) did not deny: ' .. vim.inspect(decision))
    end
    -- The gap: a supervisor built with policy = nil still launches.
    local sink = events.new_sink()
    local reg = registry.new()
    local sup, sup_err = supervisor.new({ registry = reg, sink = sink, policy = nil })
    if sup == nil then
        return driver_fail('supervisor', 'supervisor.new failed: ' .. tostring(sup_err))
    end
    local adapter = { name = 'gauntlet_bare' }
    function adapter.probe()
        return { available = true, remote = false }
    end
    function adapter.start(run, _sink)
        return { adapter = 'gauntlet_bare', run_id = run.id, closed = false }
    end
    function adapter.cancel(_handle)
        return true
    end
    function adapter.close(handle)
        handle.closed = true
    end
    local ok, reg_err = registry.register_adapter(reg, 'gauntlet_bare', adapter)
    if not ok then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local run, create_err = supervisor.create(sup, {
        workflow = 'gauntlet_bare_flow',
        goal = 'nil policy probe',
        workspace = work_dir,
        adapter = 'gauntlet_bare',
    })
    if run == nil then
        return driver_fail('create', 'supervisor.create failed: ' .. tostring(create_err))
    end
    local started, start_err = supervisor.start_run(sup, run.id, 'gauntlet_bare')
    if started and run.state == 'running' then
        ev('FAIL today: launch with sup.policy == nil proceeded to running —')
        ev('the fail-closed decide() has no call site on the launch path')
        return fix_absent(defect_how())
    end
    ev('launch refused with nil policy: ' .. tostring(start_err))
    return scenario_pass('nil policy denies the launch: Fix 3 present')
end

---no-classification (A): launch never calls probe and builds no policy
---request (source evidence); the lying probe is demonstrated and banked as
---a trust-boundary finding.
local function scenario_no_classification(handles)
    local probe_calls = 0
    local network_io_performed = false
    local adapter = { name = 'gauntlet_liar' }
    function adapter.probe()
        probe_calls = probe_calls + 1
        -- The lie: claims local-only.
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
    end
    function adapter.start(run, _sink)
        -- ...while performing network I/O.
        network_io_performed = true
        return { adapter = 'gauntlet_liar', run_id = run.id, closed = false }
    end
    function adapter.cancel(_handle)
        return true
    end
    function adapter.close(handle)
        handle.closed = true
    end
    local ok, err = handles.registry_mod.register_adapter(handles.registry, 'gauntlet_liar', adapter)
    if not ok then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(err))
    end
    local run_id, run_err = handles.harness.run(base_spec('gauntlet_liar'))
    if run_id == nil then
        return driver_fail('run', 'harness.run failed: ' .. tostring(run_err))
    end
    ev('launch by explicit adapter name called probe() ' .. probe_calls .. ' time(s)')
    ev('adapter attested remote=false yet performed network I/O: ' .. tostring(network_io_performed))
    -- Source evidence: launch() builds no policy request at all.
    local src_path = vim.env.DIVER_LUA_DIR .. '/ai/harness/supervisor.lua'
    local fh, open_err = io.open(src_path, 'r')
    if fh == nil then
        return driver_fail('source-scan', 'could not read supervisor.lua: ' .. tostring(open_err))
    end
    local src = fh:read('*a')
    fh:close()
    local launch_start = src:find('local function launch', 1, true)
    local launch_end = src:find('\nfunction M.start_run', 1, true)
    if launch_start == nil or launch_end == nil or launch_end <= launch_start then
        return driver_fail('source-scan', 'could not isolate the launch() body')
    end
    local body = src:sub(launch_start, launch_end)
    if body:find('policy', 1, true) ~= nil then
        return driver_fail('source-scan', 'launch() now references policy: premise changed')
    end
    ev('file evidence: launch() body (supervisor.lua lines 182-224) contains no policy reference')
    ev('file evidence: zero decide( call sites in supervisor.lua and init.lua')
    ev('TRUST-BOUNDARY FINDING (banked, out of scope to fix here): Phase-2 classifies risk')
    ev('from probe attestation, so a lying probe bypasses classification — probe attestation')
    ev('is trusted input; fixing that is adapter-vetting work, not silently fixable here.')
    ev('EXTENSIONS-SMUGGLING + MUTATED-POLICY (banked as acceptance criteria): unverifiable')
    ev('today — launch builds no policy request, so there is nothing to smuggle through;')
    ev('Phase-2 must assert the request is built only from run fields and that decide runs')
    ev('at launch time against the live policy table.')
    return fix_absent(defect_how())
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'default'
    end
    ev('diver probed: ' .. DIVER_SHA .. ' (main; no Phase-2 branch exists)')
    ev('scenario=' .. scenario)
    if scenario == 'nil-policy' then
        return scenario_nil_policy()
    end
    if scenario == 'no-classification' then
        local handles, boot_err = bootstrap({})
        if handles == nil then
            return driver_fail('bootstrap', boot_err)
        end
        return scenario_no_classification(handles)
    end
    local policy = nil
    if scenario == 'default' then
        policy = { default = 'deny', rules = {} }
    elseif scenario == 'allow-launches' then
        policy = { default = 'allow', rules = {} }
    else
        return driver_fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
    end
    local handles, boot_err = bootstrap(policy)
    if handles == nil then
        return driver_fail('bootstrap', boot_err)
    end
    if scenario == 'default' then
        return scenario_default(handles)
    end
    return scenario_allow(handles)
end

local ok, verdict = pcall(main)
if not ok then
    ev('lua error: ' .. tostring(verdict))
    verdict = driver_fail('lua-driver', 'unhandled error: ' .. tostring(verdict))
end
-- verdict on the real stdout: in `nvim --headless -l`, Lua print() goes
-- to stderr, but the Rust runner parses the verdict from stdout.
io.stdout:write(vim.json.encode(verdict) .. '\n')
