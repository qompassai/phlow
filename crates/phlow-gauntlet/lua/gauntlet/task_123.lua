-- task-123 driver: `:HarnessRun` argument parsing contract acceptance probe
-- (diver Phase-2 Decision 2).
--
-- The design: hybrid — explicit args when given, prompts for the rest;
-- everything after `--` is the goal VERBATIM. Bare `:HarnessRun` prompts
-- for adapter, workflow, goal in that order.
--
-- Diver probed: c84352cc850d507df477706b9166b6541ebe9e1c (main; no Phase-2
-- branch exists).
--
-- Scenarios via GAUNTLET_SCENARIO (default "parse-harnessrun"):
--   parse-harnessrun    :HarnessRun does not exist; pin the hybrid parsing
--                       contract (V, gap)
--   goal-required       validate_run_spec rejects missing/empty goal — the
--                       building block behind "never a goal-less run" (V)
--   double-dash-in-goal goal containing `--`: only the first `--` is the
--                       separator (A, gap)
--   percent-hash-newline `%`/`#` never filename-expanded; newline preserved
--                       or cleanly rejected, never truncated (A, gap)
--
-- The command module does not exist today, so three scenarios record the
-- exact gap; that record IS the Phase-2 acceptance artifact. The goal
-- requirement characterizes today (it passes).
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
        id = 'task-123',
        outcome = 'fail',
        where = 'command-module-absent',
        how = how,
        evidence = evidence,
    }
end

local function scenario_pass(note)
    ev(note)
    return { id = 'task-123', outcome = 'pass', evidence = evidence }
end

local function driver_fail(where, how)
    return { id = 'task-123', outcome = 'fail', where = where, how = how, evidence = evidence }
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
    local rtp_root, lua_dir = resolve_diver_dirs(diver_lua_dir)
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
    return {
        types = require('ai.harness.types'),
        harness_dir = lua_dir .. '/ai/harness/',
    }
end

---True when the harness ships any user-command surface for :HarnessRun.
local function harnessrun_exists(handles)
    if vim.fn.exists(':HarnessRun') ~= 0 then
        return true
    end
    -- Static belt-and-braces: no command registration anywhere in the
    -- harness sources. The name must occur as its own identifier — the
    -- LuaCATS type AiHarnessRun in supervisor.lua is NOT the command, so
    -- a match glued to a preceding identifier char does not count.
    for _, name in ipairs({ 'init.lua', 'supervisor.lua', 'registry.lua' }) do
        local fh = io.open(handles.harness_dir .. name, 'r')
        if fh ~= nil then
            local text = fh:read('*a')
            fh:close()
            local start = 1
            while true do
                local s, e = text:find('HarnessRun', start, true)
                if s == nil then
                    break
                end
                local prev = s > 1 and text:sub(s - 1, s - 1) or ''
                if not prev:match('[A-Za-z0-9_]') then
                    return true
                end
                start = e + 1
            end
            if text:find('nvim_create_user_command', 1, true) ~= nil then
                return true
            end
        end
    end
    return false
end

---parse-harnessrun (V): the command does not exist; pin the contract.
local function scenario_parse(handles)
    if harnessrun_exists(handles) then
        return scenario_pass(':HarnessRun exists — contract testable (Phase-2 shipped)')
    end
    local ok, err = pcall(vim.cmd, 'HarnessRun a2a deploy -- fix the "quoted" thing')
    if ok then
        return driver_fail('command', ':HarnessRun executed unexpectedly: ' .. vim.inspect(err))
    end
    ev('vim.fn.exists(":HarnessRun") == 0')
    ev("vim.cmd('HarnessRun ...') fails: " .. tostring(err):sub(1, 80))
    ev('static: no HarnessRun / nvim_create_user_command in harness sources')
    ev('file evidence: lua/ai/harness/ has no command module (no commands.lua, no :Harness* registration)')
    local how = 'decision-2 gap: no command module exists, so the hybrid parsing contract is untestable. '
        .. 'Phase-2 acceptance: `:HarnessRun a2a deploy -- fix the "quoted" thing | properly` parses to '
        .. 'adapter=a2a, workflow=deploy, goal byte-identical including quotes and `|` (everything after '
        .. 'the first `--` is the goal VERBATIM — no shell splitting, no quote stripping).'
    return gap(how)
end

---goal-required (V): validate_run_spec rejects missing/empty goals — the
---building block behind "never creates a goal-less run".
local function scenario_goal_required(handles)
    local validate = handles.types.validate_run_spec
    local good = { workflow = 'w', goal = 'do the thing', workspace = vim.env.GAUNTLET_WORK_DIR }
    local ok, err = validate(good)
    if not ok then
        return driver_fail('validate', 'validate_run_spec rejected a valid spec: ' .. tostring(err))
    end
    ev('validate_run_spec accepts a spec with a non-empty goal')

    local missing = { workflow = 'w', workspace = vim.env.GAUNTLET_WORK_DIR }
    local ok2, err2 = validate(missing)
    if ok2 then
        return driver_fail('validate', 'validate_run_spec accepted a spec with no goal')
    end
    ev("validate_run_spec rejects missing goal ('" .. tostring(err2) .. "')")

    local empty = { workflow = 'w', goal = '', workspace = vim.env.GAUNTLET_WORK_DIR }
    local ok3, err3 = validate(empty)
    if ok3 then
        return driver_fail('validate', 'validate_run_spec accepted an empty goal')
    end
    ev("validate_run_spec rejects empty goal ('" .. tostring(err3) .. "')")

    ev('source: types.lua validate_run_spec — spec.goal must be a non-empty string')
    ev('contract pin: the future command must never create a goal-less run; empty goal after `--`')
    ev('falls back to PROMPTING for the goal, not to an error and not to a goal-less create')
    return scenario_pass('validate_run_spec requires a non-empty goal')
end

---double-dash-in-goal (A): a goal containing `--` itself.
local function scenario_double_dash(handles)
    if harnessrun_exists(handles) then
        return scenario_pass(':HarnessRun exists — separator semantics testable (Phase-2 shipped)')
    end
    ev('GAP today: no command module, so separator semantics are unverifiable')
    ev('file evidence: lua/ai/harness/ has no command module')
    local how = 'decision-2 gap: `:HarnessRun a2a w -- -- -- --` must parse the goal as `-- -- --` — '
        .. 'only the FIRST `--` is the separator; later `--` tokens are literal goal text. '
        .. 'Unverifiable today (no command module). Phase-2 acceptance: goal byte-identical after '
        .. 'the first `--`, including embedded `--` tokens.'
    return gap(how)
end

---percent-hash-newline (A): `%`/`#` must never filename-expand; a newline
---in the goal is preserved or cleanly rejected, never silently truncated.
local function scenario_percent_hash_newline(handles)
    if harnessrun_exists(handles) then
        return scenario_pass(':HarnessRun exists — expansion semantics testable (Phase-2 shipped)')
    end
    ev('GAP today: no command module, so expansion/newline semantics are unverifiable')
    ev('file evidence: lua/ai/harness/ has no command module')
    local how = 'decision-2 gap: `%`/`#` in the goal must pass through LITERAL (never filename-expanded '
        .. '— the command must not use expand()/<q-args> expansion on the goal span); a goal containing '
        .. 'a newline must be preserved byte-identical or cleanly rejected, never silently truncated at '
        .. 'the newline. Unverifiable today (no command module). Phase-2 acceptance: literal `%`/`#`, '
        .. 'newline preserved-or-rejected.'
    return gap(how)
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'parse-harnessrun'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return driver_fail('bootstrap', boot_err)
    end
    ev('diver probed: ' .. DIVER_SHA .. ' (main; no Phase-2 branch exists)')
    ev('scenario=' .. scenario)
    if scenario == 'parse-harnessrun' then
        return scenario_parse(handles)
    elseif scenario == 'goal-required' then
        return scenario_goal_required(handles)
    elseif scenario == 'double-dash-in-goal' then
        return scenario_double_dash(handles)
    elseif scenario == 'percent-hash-newline' then
        return scenario_percent_hash_newline(handles)
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
