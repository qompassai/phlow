-- task-125 driver: run selection TOCTOU for cancel/resume acceptance probe
-- (diver Phase-2 Decision 2).
--
-- The design: `:HarnessCancel` / `:HarnessResume` with no arg offer
-- vim.ui.select over eligible runs (live runs for cancel; terminal-but-
-- not-completed for resume). The TOCTOU core: the selected run may go
-- terminal between listing and acting — the act must then fail cleanly
-- ('run is already terminal'), with no corruption and no error event.
-- Selection is by run id, never by workflow name.
--
-- Diver probed: c84352cc850d507df477706b9166b6541ebe9e1c (main; no Phase-2
-- branch exists).
--
-- Scenarios via GAUNTLET_SCENARIO (default "cancel-picker-missing"):
--   cancel-picker-missing  :HarnessCancel does not exist; pin the live-run
--                          picker contract (V, gap)
--   resume-picker-missing  :HarnessResume does not exist; pin the
--                          resumable-run picker contract (V, gap)
--   cancel-after-terminal  run goes terminal between "list" and "cancel":
--                          cancel returns 'run is already terminal'
--                          cleanly, no corruption, no new events (A)
--   cancel-unknown-run     cancel of a never-existing id: clean
--                          'unknown run' error (A)
--
-- The picker commands do not exist today, so the first two scenarios
-- record the exact gap; that record IS the Phase-2 acceptance artifact.
-- The TOCTOU core and the unknown-run path exercise supervisor.cancel
-- directly and characterize today (both pass).
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
        id = 'task-125',
        outcome = 'fail',
        where = 'command-module-absent',
        how = how,
        evidence = evidence,
    }
end

local function scenario_pass(note)
    ev(note)
    return { id = 'task-125', outcome = 'pass', evidence = evidence }
end

local function driver_fail(where, how)
    return { id = 'task-125', outcome = 'fail', where = where, how = how, evidence = evidence }
end

---Wire up the harness. Returns handles or (nil, err).
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

---cancel-picker-missing (V): :HarnessCancel does not exist; pin the
---live-run picker contract.
local function scenario_cancel_picker(handles)
    if vim.fn.exists(':HarnessCancel') ~= 0 then
        return gap(':HarnessCancel exists — picker spec testable (Phase-2 shipped)')
    end
    ev('vim.fn.exists(":HarnessCancel") == 0')
    ev('file evidence: lua/ai/harness/ has no command module (no :HarnessCancel registration)')
    local how = 'decision-2 gap: `:HarnessCancel` with no arg must offer vim.ui.select over the LIVE runs '
        .. '(created/queued/running/waiting_*) — never terminal ones — and act on the SELECTED RUN ID. '
        .. 'Unverifiable today (no command module). Phase-2 acceptance: picker lists exactly the live '
        .. 'runs; two runs with identical workflow names are disambiguated by id and the wrong run is '
        .. 'never acted on.'
    return gap(how)
end

---resume-picker-missing (V): :HarnessResume does not exist; pin the
---resumable-run picker contract.
local function scenario_resume_picker(handles)
    if vim.fn.exists(':HarnessResume') ~= 0 then
        return gap(':HarnessResume exists — picker spec testable (Phase-2 shipped)')
    end
    ev('vim.fn.exists(":HarnessResume") == 0')
    ev('file evidence: lua/ai/harness/ has no command module (no :HarnessResume registration)')
    local how = 'decision-2 gap: `:HarnessResume` with no arg must offer vim.ui.select over RESUMABLE runs '
        .. '(failed/cancelled/timed_out/interrupted) — never completed, never running — and resume by '
        .. 'run id. Unverifiable today (no command module). Phase-2 acceptance: picker lists exactly the '
        .. 'resumable set; completed and running runs are excluded.'
    return gap(how)
end

---cancel-after-terminal (A): the TOCTOU core. The run goes terminal
---between "listing" and "acting" — cancel must return 'run is already
---terminal' cleanly: no corruption, no new events.
local function scenario_cancel_after_terminal(handles)
    local name, reg_err = register_lingering_adapter(handles, 'gauntlet_linger')
    if name == nil then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local spec = {
        workflow = 'gauntlet_toctou_flow',
        goal = 'go terminal before cancel',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = name,
        timeout_ms = 30000,
    }
    local run_id, run_err = handles.harness.run(spec)
    if run_id == nil then
        return driver_fail('run', 'harness.run failed: ' .. tostring(run_err))
    end
    local run = handles.supervisor.get(handles.sup, run_id)
    if run.state ~= 'running' then
        return driver_fail('state', 'run did not reach running: ' .. tostring(run.state))
    end
    ev('run is live (state=running) — this is what the picker would list')

    -- Between listing and acting, the run goes terminal (finished by the
    -- driver, standing in for the adapter completing on its own).
    local events_before = #(handles.sink:events())
    local fok, ferr = handles.supervisor.finish(handles.sup, run_id, 'completed')
    if not fok then
        return driver_fail('finish', 'supervisor.finish failed: ' .. tostring(ferr))
    end
    local events_after_finish = #(handles.sink:events())
    ev('run finished between list and act (state=' .. handles.supervisor.get(handles.sup, run_id).state .. ')')

    -- Now act: cancel must fail cleanly.
    local cok, cerr = handles.supervisor.cancel(handles.sup, run_id)
    if cok then
        return driver_fail('toctou', 'cancel of a terminal run SUCCEEDED — expected a clean refusal')
    end
    if type(cerr) ~= 'string' or not cerr:find('already terminal', 1, true) then
        return driver_fail('toctou', "cancel error is not 'already terminal': " .. vim.inspect(cerr))
    end
    ev("cancel returned clean refusal: '" .. cerr .. "'")
    local run_after = handles.supervisor.get(handles.sup, run_id)
    if run_after.state ~= 'completed' then
        return driver_fail('toctou', 'cancel corrupted the run state: ' .. tostring(run_after.state))
    end
    ev('run state unchanged (completed) — no corruption')
    local events_after_cancel = #(handles.sink:events())
    if events_after_cancel ~= events_after_finish then
        return driver_fail(
            'toctou',
            'cancel appended ' .. (events_after_cancel - events_after_finish) .. ' events on refusal'
        )
    end
    ev('sink events unchanged by the refused cancel (' .. events_after_finish .. ' total; '
        .. events_before .. ' before finish) — no error event')
    ev('source: supervisor.lua M.cancel — unknown-run and already-terminal guards before any mutation')
    return scenario_pass("TOCTOU: cancel after terminal fails cleanly ('already terminal'), no corruption")
end

---cancel-unknown-run (A): cancel of a never-existing id is a clean
---'unknown run' error — no panic, no state touched.
local function scenario_cancel_unknown(handles)
    local runs_before = #handles.supervisor.list(handles.sup)
    local ok, err = handles.supervisor.cancel(handles.sup, 'run-that-never-existed')
    if ok then
        return driver_fail('unknown', 'cancel of an unknown run id SUCCEEDED')
    end
    if type(err) ~= 'string' or not err:find('unknown run', 1, true) then
        return driver_fail('unknown', "cancel error is not 'unknown run': " .. vim.inspect(err))
    end
    ev("cancel('run-that-never-existed') -> clean refusal: '" .. err .. "'")
    local runs_after = #handles.supervisor.list(handles.sup)
    if runs_after ~= runs_before then
        return driver_fail('unknown', 'cancel of unknown run changed the run list')
    end
    ev('run list unchanged — no state touched')
    ev('source: supervisor.lua M.cancel — `if run == nil then return nil, \'unknown run: ...\'`')
    return scenario_pass("cancel of unknown run id fails cleanly ('unknown run')")
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'cancel-picker-missing'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return driver_fail('bootstrap', boot_err)
    end
    ev('diver probed: ' .. DIVER_SHA .. ' (main; no Phase-2 branch exists)')
    ev('scenario=' .. scenario)
    if scenario == 'cancel-picker-missing' then
        return scenario_cancel_picker(handles)
    elseif scenario == 'resume-picker-missing' then
        return scenario_resume_picker(handles)
    elseif scenario == 'cancel-after-terminal' then
        return scenario_cancel_after_terminal(handles)
    elseif scenario == 'cancel-unknown-run' then
        return scenario_cancel_unknown(handles)
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
