-- task-124 driver: prompt fallback and abort atomicity acceptance probe
-- (diver Phase-2 Decision 2).
--
-- The design: missing pieces fall back to vim.ui.select (adapter, from the
-- registry) / vim.ui.input (workflow, goal), in that order; aborting any
-- prompt aborts the WHOLE command — zero runs created, zero events
-- appended. A whitespace-only goal is treated as abort, not as a goal.
--
-- Diver probed: c84352cc850d507df477706b9166b6541ebe9e1c (main; no Phase-2
-- branch exists).
--
-- Scenarios via GAUNTLET_SCENARIO (default "bare-harnessrun"):
--   bare-harnessrun  bare :HarnessRun -> three prompts in order, one run,
--                    one run.created event (V, gap)
--   partial-args     :HarnessRun a2a -> prompts only for the missing two (V,
--                    gap)
--   abort-atomicity  abort at each of the three stages (nil AND empty
--                    string) -> zero runs, zero events (A, gap)
--   whitespace-goal  whitespace-only goal -> treated as abort (A, gap;
--                    plus the building-block characterization that
--                    validate_run_spec accepts '   ', so the trim rule must
--                    live in the command layer)
--
-- The command module does not exist today, so every scenario records the
-- exact gap; that record IS the Phase-2 acceptance artifact. When Phase 2
-- ships the command, this driver becomes its executable spec: headless
-- nvim with scripted vim.ui.select/vim.ui.input sequences, counting runs
-- and sink events after each scenario.
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
        id = 'task-124',
        outcome = 'fail',
        where = 'command-module-absent',
        how = how,
        evidence = evidence,
    }
end

local function driver_fail(where, how)
    return { id = 'task-124', outcome = 'fail', where = where, how = how, evidence = evidence }
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
        sup = st.supervisor,
        sink = st.sink,
        supervisor = require('ai.harness.supervisor'),
        types = require('ai.harness.types'),
    }
end

local function command_exists()
    return vim.fn.exists(':HarnessRun') ~= 0
end

---Count sink events right now.
local function event_count(handles)
    return #(handles.sink:events())
end

---bare-harnessrun (V): bare :HarnessRun prompts adapter -> workflow ->
---goal, creates exactly one run, appends one run.created event.
local function scenario_bare(handles)
    if command_exists() then
        return gap(':HarnessRun exists — prompt-order spec testable (Phase-2 shipped); driver needs scripted vim.ui')
    end
    ev('vim.fn.exists(":HarnessRun") == 0 — no command, no prompts, nothing to order')
    ev('runs before/after: ' .. #handles.supervisor.list(handles.sup) .. ' (command cannot run)')
    ev('file evidence: lua/ai/harness/ has no command module; no vim.ui.select/input call sites in harness')
    local how = 'decision-2 gap: bare `:HarnessRun` must prompt in order adapter (vim.ui.select over the '
        .. 'registry) -> workflow (vim.ui.input) -> goal (vim.ui.input), then create EXACTLY one run and '
        .. 'append exactly one run.created event. Unverifiable today (no command module). Phase-2 '
        .. 'acceptance: scripted vim.ui sequences produce the prompt order, one run, one run.created event.'
    return gap(how)
end

---partial-args (V): `:HarnessRun a2a` prompts only for workflow and goal.
local function scenario_partial(handles)
    if command_exists() then
        return gap(':HarnessRun exists — partial-args spec testable (Phase-2 shipped); driver needs scripted vim.ui')
    end
    local ok, err = pcall(vim.cmd, 'HarnessRun a2a')
    ev("vim.cmd('HarnessRun a2a') fails: " .. tostring(err):sub(1, 80))
    if ok then
        return driver_fail('command', ':HarnessRun unexpectedly executed')
    end
    ev('file evidence: lua/ai/harness/ has no command module')
    local how = 'decision-2 gap: `:HarnessRun a2a` must prompt ONLY for the missing pieces (workflow, '
        .. 'goal) — never re-prompt for the given adapter. Unverifiable today (no command module). '
        .. 'Phase-2 acceptance: scripted vim.ui shows exactly two prompts, one run, one run.created event.'
    return gap(how)
end

---abort-atomicity (A): aborting any prompt aborts the whole command.
local function scenario_abort(handles)
    if command_exists() then
        return gap(':HarnessRun exists — abort spec testable (Phase-2 shipped); driver needs scripted vim.ui')
    end
    ev('GAP today: no command module, so prompt-abort atomicity is unverifiable')
    ev('runs: ' .. #handles.supervisor.list(handles.sup) .. '; sink events: ' .. event_count(handles))
    ev('file evidence: lua/ai/harness/ has no command module')
    local how = 'decision-2 gap: aborting at ANY of the three prompt stages — vim.ui.select/vim.ui.input '
        .. 'returning nil OR empty string (both behave identically) — must abort the WHOLE command: zero '
        .. 'runs created, zero events appended (atomicity — the command either fully specifies a run or '
        .. 'does nothing). Unverifiable today (no command module). Phase-2 acceptance: six scripted '
        .. 'scenarios (3 stages x {nil, empty}) each leave run count and event count unchanged.'
    return gap(how)
end

---whitespace-goal (A): a whitespace-only goal is an abort, not a goal.
---Building block: validate_run_spec accepts '   ' (non-empty string), so
---the trim rule MUST live in the command layer — the spec validator alone
---cannot enforce it.
local function scenario_whitespace(handles)
    local validate = handles.types.validate_run_spec
    local spec = {
        workflow = 'w',
        goal = '   ',
        workspace = vim.env.GAUNTLET_WORK_DIR,
    }
    local ok, err = validate(spec)
    if not ok then
        return driver_fail(
            'validate',
            'validate_run_spec rejected a whitespace-only goal: ' .. tostring(err)
        )
    end
    ev("characterization: validate_run_spec ACCEPTS goal='   ' (non-empty string)")
    ev('consequence: the whitespace trim rule must live in the command layer, not the spec validator')
    if command_exists() then
        return gap(':HarnessRun exists — whitespace-goal spec testable (Phase-2 shipped)')
    end
    ev('GAP today: no command module, so the whitespace-as-abort rule is unverifiable')
    ev('file evidence: lua/ai/harness/ has no command module; types.lua validate_run_spec (no trim)')
    local how = 'decision-2 gap: a prompt returning a whitespace-only goal must be treated as ABORT '
        .. '(zero runs, zero events), never as a goal — the command must trim-check because '
        .. "validate_run_spec accepts '   '. Unverifiable today (no command module). Phase-2 "
        .. 'acceptance: whitespace-only goal input aborts the command identically to nil/empty.'
    return gap(how)
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'bare-harnessrun'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return driver_fail('bootstrap', boot_err)
    end
    ev('diver probed: ' .. DIVER_SHA .. ' (main; no Phase-2 branch exists)')
    ev('scenario=' .. scenario)
    if scenario == 'bare-harnessrun' then
        return scenario_bare(handles)
    elseif scenario == 'partial-args' then
        return scenario_partial(handles)
    elseif scenario == 'abort-atomicity' then
        return scenario_abort(handles)
    elseif scenario == 'whitespace-goal' then
        return scenario_whitespace(handles)
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
