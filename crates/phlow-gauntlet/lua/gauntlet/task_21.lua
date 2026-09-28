-- task-21 driver: saga-coordinator seam recon for diver's ai.harness.
--
-- The design asks for a 4-step saga (reserve -> charge -> provision ->
-- notify) with reverse-order compensation, driven by "real harness/workflow
-- code". Recon finds that seam ABSENT:
--
--   * registry.lua exposes register_workflow / get_workflow — a NAMING
--     layer (name -> { adapter = ... } binding). There is no list_workflows,
--     no run_workflow, no execute_workflow, and nothing anywhere in the
--     harness executes a multi-step definition.
--   * init.lua exposes setup / run / cancel / resume / version. harness.run
--     hands the spec straight to supervisor.create, which never consults
--     the workflow registry.
--   * types.validate_run_spec requires spec.workflow to be a non-empty
--     string — a label attached to the run, not a program to execute.
--
-- Building a coordinator in this driver would invent the seam the design
-- forbids inventing ("the saga coordinator must be real harness/workflow
-- code"), so every scenario documents the absence with file evidence and
-- reports outcome='fail', where='seam': the designed capability has no
-- seam to drive. The integration tests assert the evidence, not a pass.
--
-- Scenarios via GAUNTLET_SCENARIO (default "default"):
--   default       naming layer accepts a saga-shaped def; spec label validated
--   naming-layer  harness.run treats workflow as a label: one run, no steps
--   no-executor   executor entry points are nil + registry source scan
--   inert-def     saga def with steps+compensations registers; nothing executes
--
-- Prints exactly one JSON verdict line to stdout and always exits 0.
-- Writes nothing outside GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function seam_absent(how)
    return { id = 'task-21', outcome = 'fail', where = 'seam', how = how, evidence = evidence }
end

local function driver_fail(where, how)
    return { id = 'task-21', outcome = 'fail', where = where, how = how, evidence = evidence }
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
        registry_mod = require('ai.harness.registry'),
        registry = st.registry,
        supervisor = require('ai.harness.supervisor'),
        types = require('ai.harness.types'),
    }
end

---The saga-shaped workflow definition the design describes. Steps and
---compensations are data; nothing in the harness will execute them.
local function saga_def()
    return {
        adapter = 'gauntlet_saga',
        steps = { 'reserve', 'charge', 'provision', 'notify' },
        compensations = { 'deprovision', 'refund', 'unreserve' },
    }
end

---default (V): the naming layer works — a saga-shaped def registers and
---reads back; the run spec validates `workflow` as a label.
local function scenario_default(handles)
    local ok, err = handles.registry_mod.register_workflow(
        handles.registry,
        'gauntlet_saga',
        saga_def()
    )
    if not ok then
        return driver_fail('register', 'register_workflow rejected the saga def: ' .. tostring(err))
    end
    ev('register_workflow accepted the saga-shaped def (naming layer works)')
    local def = handles.registry_mod.get_workflow(handles.registry, 'gauntlet_saga')
    if def == nil then
        return driver_fail('register', 'get_workflow did not return the registered def')
    end
    ev('get_workflow returned the def: steps=' .. table.concat(def.steps, ','))
    local spec_ok, spec_err = handles.types.validate_run_spec({
        workflow = 'gauntlet_saga',
        goal = 'saga probe',
        workspace = vim.env.GAUNTLET_WORK_DIR,
    })
    if not spec_ok then
        return driver_fail('spec', 'validate_run_spec rejected the saga label: ' .. tostring(spec_err))
    end
    ev('validate_run_spec accepts workflow as a non-empty string label (types.lua)')
    local bad_ok, bad_err = handles.types.validate_run_spec({
        goal = 'saga probe',
        workspace = vim.env.GAUNTLET_WORK_DIR,
    })
    if bad_ok then
        return driver_fail('spec', 'validate_run_spec accepted a spec with no workflow label')
    end
    ev('validate_run_spec rejects a missing workflow label: ' .. tostring(bad_err))
    return seam_absent(
        'seam absent: diver ai.harness has a workflow naming layer (register_workflow/get_workflow) '
            .. 'but no workflow runner or saga coordinator; spec.workflow is a label only '
            .. '(registry.lua, init.lua, types.lua) — the design forbids inventing the coordinator'
    )
end

---naming-layer (V): harness.run treats `workflow` as a label — exactly one
---normal run is created; no steps execute; the registry is never consulted.
local function scenario_naming_layer(handles)
    local ok, err = handles.registry_mod.register_workflow(
        handles.registry,
        'gauntlet_saga',
        saga_def()
    )
    if not ok then
        return driver_fail('register', 'register_workflow failed: ' .. tostring(err))
    end
    -- A fake adapter that completes immediately; the run below must behave
    -- like any ordinary single run, proving `workflow` selected no program.
    local instant = { name = 'gauntlet_instant' }
    function instant.probe()
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
    function instant.start(run, sink)
        sink:append(run.id, 'model.completed', { outcome = 'completed' }, { source = 'gauntlet_instant' })
        return { adapter = 'gauntlet_instant', run_id = run.id, closed = false }
    end
    function instant.cancel(_handle)
        return true
    end
    function instant.close(handle)
        handle.closed = true
    end
    local reg_ok, reg_err =
        handles.registry_mod.register_adapter(handles.registry, 'gauntlet_instant', instant)
    if not reg_ok then
        return driver_fail('register', 'fake adapter registration failed: ' .. tostring(reg_err))
    end
    local run_id, run_err = handles.harness.run({
        workflow = 'gauntlet_saga',
        goal = 'prove workflow is a label',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = 'gauntlet_instant',
        timeout_ms = 30000,
    })
    if run_id == nil then
        return driver_fail('run', 'harness.run failed: ' .. tostring(run_err))
    end
    ev('harness.run with workflow=gauntlet_saga created run ' .. run_id)
    local runs = handles.supervisor.list(handles.sup)
    if #runs ~= 1 then
        return driver_fail('run', 'expected exactly 1 run, saw ' .. #runs)
    end
    ev('exactly one run exists: the workflow name selected no multi-step program')
    local run = handles.supervisor.get(handles.sup, run_id)
    if run.workflow ~= 'gauntlet_saga' then
        return driver_fail('run', 'run.workflow label not stored: ' .. tostring(run.workflow))
    end
    ev("run.workflow stored as the label 'gauntlet_saga' (supervisor.lua: run.workflow = spec.workflow)")
    if #run.children ~= 0 then
        return driver_fail('run', 'run unexpectedly owns children')
    end
    ev('run owns no children and no step executions: label-only confirmed')
    return seam_absent(
        'seam absent: harness.run never consults the workflow registry (init.lua hands the spec '
            .. 'to supervisor.create); spec.workflow is stored as a label, never executed'
    )
end

---no-executor (A): probe the real modules for any executor entry point;
---scan registry.lua source for workflow-related definitions.
local function scenario_no_executor(handles)
    local probes = {
        { mod = handles.harness, name = 'run_workflow' },
        { mod = handles.harness, name = 'execute_workflow' },
        { mod = handles.harness, name = 'saga' },
        { mod = handles.registry_mod, name = 'run_workflow' },
        { mod = handles.registry_mod, name = 'execute_workflow' },
        { mod = handles.registry_mod, name = 'list_workflows' },
        { mod = handles.supervisor, name = 'run_workflow' },
    }
    for _, probe in ipairs(probes) do
        if probe.mod[probe.name] ~= nil then
            return driver_fail(
                'executor-probe',
                'unexpected executor entry point present: ' .. probe.name
            )
        end
        ev('absent: ' .. probe.name)
    end
    ev('no executor entry point on harness / registry / supervisor modules')
    -- Source-level evidence: enumerate every workflow-related definition in
    -- the real registry.lua and show the set is naming-only. Neovim's
    -- `require` resolves through the runtimepath (not package.path), so
    -- build the source path from DIVER_LUA_DIR directly.
    local reg_path = vim.env.DIVER_LUA_DIR .. '/ai/harness/registry.lua'
    local fh, open_err = io.open(reg_path, 'r')
    if fh == nil then
        return driver_fail('source-scan', 'could not read registry.lua: ' .. tostring(open_err))
    end
    local names = {}
    for line in fh:lines() do
        local name = line:match('^function M%.([%w_]*[Ww]orkflow[%w_]*)%(')
        if name ~= nil then
            names[#names + 1] = name
        end
    end
    fh:close()
    table.sort(names)
    ev('registry.lua workflow-related definitions: ' .. table.concat(names, ', '))
    local want = { 'get_workflow', 'register_workflow' }
    if #names ~= #want then
        return driver_fail(
            'source-scan',
            'unexpected workflow definition set in registry.lua: ' .. table.concat(names, ', ')
        )
    end
    for i, name in ipairs(want) do
        if names[i] ~= name then
            return driver_fail('source-scan', 'workflow definition mismatch at index ' .. i)
        end
    end
    ev('file evidence: ' .. reg_path)
    ev('the complete workflow surface is {register_workflow, get_workflow}: naming only, no runner')
    return seam_absent(
        'seam absent: no run_workflow/execute_workflow/saga entry point on any harness module; '
            .. 'registry.lua defines only register_workflow + get_workflow (file evidence above)'
    )
end

---inert-def (A): a saga def carrying steps AND compensations registers
---cleanly — and then nothing happens. No runs, no step invocations, no
---compensation invocations. Silent acceptance without execution is the
---dangerous case, and it is exactly what the naming layer does.
local function scenario_inert_def(handles)
    local step_calls = {}
    local def = saga_def()
    def.on_step = function(step)
        step_calls[#step_calls + 1] = step
    end
    local ok, err =
        handles.registry_mod.register_workflow(handles.registry, 'gauntlet_saga_inert', def)
    if not ok then
        return driver_fail('register', 'register_workflow rejected the inert def: ' .. tostring(err))
    end
    ev('register_workflow accepted a def carrying steps + compensations + on_step hook')
    local runs = handles.supervisor.list(handles.sup)
    if #runs ~= 0 then
        return driver_fail('inert', 'runs appeared without harness.run: ' .. #runs)
    end
    ev('supervisor holds 0 runs: registering a saga def executes nothing')
    if #step_calls ~= 0 then
        return driver_fail('inert', 'step hook fired without a runner')
    end
    ev('on_step hook never fired: no executor exists to call it')
    -- The registry cannot reject step-shaped defs: it validates only
    -- def.adapter, so a def that LOOKS like a program is stored silently.
    local ok2, err2 = handles.registry_mod.register_workflow(
        handles.registry,
        'gauntlet_saga_nosteps',
        { adapter = 'gauntlet_saga', steps = { 'reserve' } }
    )
    if not ok2 then
        return driver_fail('register', 'second def rejected: ' .. tostring(err2))
    end
    ev('registry validates only def.adapter: step-shaped defs are stored without execution semantics')
    return seam_absent(
        'seam absent: saga-shaped workflow defs are inert data in the registry; nothing in '
            .. 'ai.harness executes steps or compensations (no runner, no coordinator)'
    )
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'default'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return driver_fail('bootstrap', boot_err)
    end
    ev('scenario=' .. scenario)
    if scenario == 'default' then
        return scenario_default(handles)
    elseif scenario == 'naming-layer' then
        return scenario_naming_layer(handles)
    elseif scenario == 'no-executor' then
        return scenario_no_executor(handles)
    elseif scenario == 'inert-def' then
        return scenario_inert_def(handles)
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
