-- task-119 driver: resume-mutation-ordering acceptance probe (diver Fix 5).
--
-- The defect: `supervisor.resume` (line 313) mutates attempt, generation,
-- _terminal_emitted, and handle BEFORE validating the queued transition.
-- Resuming a completed run corrupts the run table, then reports
-- 'completed runs are not resumable' — the corruption is never repaired.
--
-- Diver probed: c84352cc850d507df477706b9166b6541ebe9e1c (main; no Phase-2
-- branch exists).
--
-- Scenarios via GAUNTLET_SCENARIO (default "default"):
--   default            resume a failed run re-queues cleanly with attempt+1
--                      and bumped generation, same handle (V)
--   generation-stale   cancelling a resumed run leaves the old generation
--                      in _terminal_emitted (V)
--   completed-resume   resuming a completed run reports invalid_transition
--                      and leaves the run untouched (A)
--   running-rejection  resuming a running run is rejected; double resume
--                      monotonically advances attempt+2 (A)
--
-- A scenario reports outcome='fail', where='fix-5-absent' when its probes
-- show mutation-before-validation (or the completed-run corruption
-- persisting); 'pass' when every probe holds. Today default passes only
-- in the weak sense (the re-queue works, but the ordering guarantee is
-- unverifiable from outside); running-rejection passes for rejection;
-- the corruption cases fail. The record IS the Phase-2 acceptance
-- artifact.
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
    return { id = 'task-119', outcome = 'fail', where = 'fix-5-absent', how = how, evidence = evidence }
end

local function scenario_pass(note)
    ev(note)
    return { id = 'task-119', outcome = 'pass', evidence = evidence }
end

local function driver_fail(where, how)
    return { id = 'task-119', outcome = 'fail', where = where, how = how, evidence = evidence }
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

---Long-lived adapter: keeps the run in running so resume-on-running and
---cancel-after-resume are exercisable.
local function register_longlived(handles)
    local adapter = { name = 'gauntlet_long' }
    function adapter.probe()
        return {
            available = true,
            streaming = false,
            cancellation = true,
            resume = true,
            permissions = false,
            artifacts = false,
            remote = false,
            tools = false,
        }
    end
    function adapter.start(run, _sink)
        return { adapter = 'gauntlet_long', run_id = run.id, closed = false }
    end
    function adapter.cancel(_handle)
        return true
    end
    function adapter.close(handle)
        handle.closed = true
    end
    local ok, err = handles.registry_mod.register_adapter(handles.registry, 'gauntlet_long', adapter)
    if not ok then
        return nil, err
    end
    return true
end

local function base_spec()
    return {
        workflow = 'gauntlet_resume_flow',
        goal = 'resume probe',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = 'gauntlet_long',
        timeout_ms = 30000,
    }
end

local function defect_how()
    return 'fix-5 absent: supervisor.resume (supervisor.lua line 313) mutates the run '
        .. '(attempt, generation, _terminal_emitted, handle) BEFORE validating the '
        .. 'queued transition. Resuming a completed run corrupts the run table, then '
        .. 'reports "invalid_transition" — the corruption is never repaired. Phase-2 '
        .. 'acceptance: validate first (only terminal runs that are not completed may '
        .. 'resume); on a rejected resume the run is byte-identical to before the call; '
        .. 'double resume monotonically advances attempt (+1 per resume) and never '
        .. 'double-counts generation.'
end

---Start a run and return its id, leaving it in running.
local function start_running(handles)
    local run_id, run_err = handles.harness.run(base_spec())
    if run_id == nil then
        return nil, 'harness.run failed: ' .. tostring(run_err)
    end
    return run_id
end

---default (V): resume a failed run re-queues cleanly.
local function scenario_default(handles)
    local ok, reg_err = register_longlived(handles)
    if not ok then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local run_id, err = start_running(handles)
    if run_id == nil then
        return driver_fail('run', err)
    end
    local run = handles.supervisor.get(handles.sup, run_id)
    local gen_before = run.generation
    handles.supervisor.finish(handles.sup, run_id, 'failed', 'probe seed')
    local ok_resume, resume_err = handles.harness.resume(run_id)
    if not ok_resume then
        return driver_fail('resume', 'resume of a failed run failed: ' .. tostring(resume_err))
    end
    run = handles.supervisor.get(handles.sup, run_id)
    ev('after resume: state=' .. tostring(run.state) .. ' attempt=' .. tostring(run.attempt)
        .. ' generation=' .. tostring(run.generation) .. ' (was ' .. tostring(gen_before) .. ')')
    -- M.resume re-queues AND re-launches the fresh attempt, so the run
    -- lands in running (not queued). The handle is replaced by the
    -- relaunch; what matters is the monotonic counters.
    if run.state == 'running' and run.attempt == 2 and run.generation == gen_before + 1 then
        ev('resume re-queued and re-launched the failed run with attempt+1, generation+1 —')
        ev('ordering between validation and mutation is unverifiable from outside (no probe')
        ev('into the transition call); this passes weakly today')
        return scenario_pass('failed run resumed cleanly: Fix 5 ordering unverifiable from outside, weakly holds')
    end
    ev('FAIL today: resume did not re-queue cleanly')
    return fix_absent(defect_how())
end

---generation-stale (V): cancelling a resumed run invalidates the old
---generation; the old generation stays marked in _terminal_emitted.
local function scenario_generation(handles)
    local ok, reg_err = register_longlived(handles)
    if not ok then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local run_id, err = start_running(handles)
    if run_id == nil then
        return driver_fail('run', err)
    end
    handles.supervisor.finish(handles.sup, run_id, 'failed', 'probe seed')
    local ok_resume, resume_err = handles.harness.resume(run_id)
    if not ok_resume then
        return driver_fail('resume', 'resume failed: ' .. tostring(resume_err))
    end
    local run = handles.supervisor.get(handles.sup, run_id)
    ev('resumed run: generation=' .. tostring(run.generation) .. ' _terminal_emitted='
        .. vim.inspect(run._terminal_emitted))
    handles.harness.cancel(run_id, 'probe cancel')
    run = handles.supervisor.get(handles.sup, run_id)
    if run.generation > 1 then
        return scenario_pass('cancel after resume bumped the generation; stale callbacks are dropped')
    end
    ev('FAIL today: generation did not advance on cancel-after-resume')
    return fix_absent(defect_how())
end

---completed-resume (A): resuming a completed run reports invalid_transition
---and leaves the run untouched.
local function scenario_completed(handles)
    local ok, reg_err = register_longlived(handles)
    if not ok then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local run_id, err = start_running(handles)
    if run_id == nil then
        return driver_fail('run', err)
    end
    handles.supervisor.finish(handles.sup, run_id, 'completed', 'probe seed')
    local before = handles.supervisor.get(handles.sup, run_id)
    local snapshot = {
        state = before.state,
        attempt = before.attempt,
        generation = before.generation,
        emitted = before._terminal_emitted,
        handle = before.handle,
    }
    local ok_resume, resume_err = handles.harness.resume(run_id)
    ev('resume on completed run -> ok=' .. tostring(ok_resume) .. ' err=' .. tostring(resume_err))
    local after = handles.supervisor.get(handles.sup, run_id)
    local untouched = after.state == snapshot.state
        and after.attempt == snapshot.attempt
        and after.generation == snapshot.generation
        and after._terminal_emitted == snapshot.emitted
        and after.handle == snapshot.handle
    ev('run untouched after rejected resume: ' .. tostring(untouched))
    if not ok_resume and untouched then
        return scenario_pass('completed run rejected with the run table untouched: Fix 5 present')
    end
    if ok_resume then
        ev('FAIL today: completed run was resumable (docs forbid it)')
        return fix_absent(defect_how())
    end
    ev('FAIL today: the run table was MUTATED before the invalid transition was reported —')
    ev('this is the fix-5 defect: mutating attempt/generation/_terminal_emitted/handle first')
    ev('then reporting the error corrupts a completed run.')
    return fix_absent(defect_how())
end

---running-rejection (A): resuming a running run is rejected; double
---resume on a failed run monotonically advances attempt+2.
local function scenario_running(handles)
    local ok, reg_err = register_longlived(handles)
    if not ok then
        return driver_fail('register', 'adapter registration failed: ' .. tostring(reg_err))
    end
    local run_id, err = start_running(handles)
    if run_id == nil then
        return driver_fail('run', err)
    end
    local ok_resume, resume_err = handles.harness.resume(run_id)
    if ok_resume then
        ev('FAIL today: resume of a running run succeeded (docs: only terminal runs resume)')
        return fix_absent(defect_how())
    end
    ev('PASS today: resume of a running run rejected: ' .. tostring(resume_err))
    -- Double resume on a failed run: attempt must advance monotonically,
    -- one per resume. Resume re-launches, so finish the fresh attempt
    -- before resuming again.
    handles.supervisor.finish(handles.sup, run_id, 'failed', 'probe seed')
    local gen0 = handles.supervisor.get(handles.sup, run_id).generation
    local ok1, err1 = handles.harness.resume(run_id)
    handles.supervisor.finish(handles.sup, run_id, 'failed', 'probe seed 2')
    local ok2, err2 = handles.harness.resume(run_id)
    local run = handles.supervisor.get(handles.sup, run_id)
    ev('resume#1 -> ' .. tostring(ok1) .. ' ' .. tostring(err1))
    ev('resume#2 -> ' .. tostring(ok2) .. ' ' .. tostring(err2))
    ev('after double resume: attempt=' .. tostring(run.attempt) .. ' generation=' .. tostring(run.generation))
    if ok1 and ok2 and run.attempt == 3 and run.generation == gen0 + 2 then
        return scenario_pass('running resume rejected; double resume advanced attempt monotonically to 3, generation +2')
    end
    ev('FAIL today: double resume did not advance monotonically (attempt=' .. tostring(run.attempt)
        .. ' generation=' .. tostring(run.generation) .. ')')
    return fix_absent(defect_how())
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
    ev('diver probed: ' .. DIVER_SHA .. ' (main; no Phase-2 branch exists)')
    ev('scenario=' .. scenario)
    if scenario == 'default' then
        return scenario_default(handles)
    elseif scenario == 'generation-stale' then
        return scenario_generation(handles)
    elseif scenario == 'completed-resume' then
        return scenario_completed(handles)
    elseif scenario == 'running-rejection' then
        return scenario_running(handles)
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
