-- task_69.lua -- gauntlet driver: hallucinated tool rejection.
--
-- The design asks for the tool dispatcher (name -> implementation):
-- the model invents a tool that doesn't exist. Scenarios: default
-- (real tool name -> dispatched); hallucination (`read_files_fast`
-- when only `read_file` exists -> clean `unknown_tool` rejection —
-- *no* fuzzy matching, *no* did-you-mean execution); adversarial:
-- near-miss names at edit-distance 1 of a privileged tool (still
-- rejected — similarity is not authority); adversarial: hallucinated
-- tool *with* valid args (rejected on the name, before args are even
-- parsed). Pass criteria: the dispatcher is exact-match only (proven
-- by the near-miss battery); the rejection suggests nothing
-- executable. Distinct from task-04 (unknown *adapter* at config
-- time) — this is an unknown *tool* at model-output time; and from
-- task-10 (the tool exists but its description is malicious).
--
-- Seam mapping (verified, not invented): diver's real dispatcher is
-- `ai.rose.tools.M.call(name, args)` — "Call a tool by name; never
-- raises, always returns a status table." Dispatch is a table lookup
-- `by_name[name]` with `assert(spec, 'unknown tool: ' .. tostring(name))`
-- BEFORE `validate_args(args, spec)`: name resolution precedes arg
-- parsing by construction. The whole module contains no fuzzy matching,
-- no edit-distance, no did-you-mean (verified by source read).
--
-- This driver exercises the REAL dispatcher (ai.rose.tools) with
-- scripted caller names. Real tools: editor_context, editor_diagnostics,
-- editor_symbols, editor_references, editor_lint, editor_check,
-- editor_scip, editor_debug, file_read, file_write, file_list.
-- Privileged example: file_write (requires trust).
--
-- Scenarios via GAUNTLET_SCENARIO (default "real-tool-dispatches"):
--   real-tool-dispatches   V: file_read of a scratch file dispatches by
--                          exact name and returns the exact bytes
--   hallucination          V: read_files_fast -> clean unknown-tool
--                          rejection; error suggests nothing executable
--   near-miss-battery      A: 10 edit-distance-1 names of real tools
--                          (incl. the privileged file_write) -> all
--                          rejected; a would-be write leaves no file
--   name-before-args       A: hallucinated name with VALID args is
--                          rejected on the name (unknown tool), while a
--                          real tool with bogus args fails on the ARGS
--                          (unknown argument) — proving name lookup runs
--                          before arg parsing
--
-- Prints exactly one JSON verdict line to stdout and always exits 0; the
-- verdict carries the outcome, not the exit code. Writes nothing outside
-- GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64
local TASK_ID = 'task-69'

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function pass()
    return { id = TASK_ID, outcome = 'pass', evidence = evidence }
end

local function fail(where, how)
    return { id = TASK_ID, outcome = 'fail', where = where, how = how, evidence = evidence }
end

---@return boolean? ok
---@return string? err
local function bootstrap()
    local diver_lua_dir = vim.env.DIVER_LUA_DIR
    if type(diver_lua_dir) ~= 'string' or diver_lua_dir == '' then
        return nil, 'DIVER_LUA_DIR is not set'
    end
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    if type(work_dir) ~= 'string' or work_dir == '' then
        return nil, 'GAUNTLET_WORK_DIR is not set'
    end
    vim.opt.runtimepath:append(diver_lua_dir)
    return true
end

---Load the real dispatcher and point it at the scratch workspace.
---@return table? tools
---@return string? err
local function load_tools()
    local ok, tools = pcall(require, 'ai.rose.tools')
    if not ok or type(tools) ~= 'table' then
        return nil, 'cannot require ai.rose.tools: ' .. tostring(tools)
    end
    if type(tools.call) ~= 'function' then
        return nil, 'ai.rose.tools.call is not a function: seam changed shape'
    end
    if type(tools.setup) ~= 'function' then
        return nil, 'ai.rose.tools.setup is not a function: seam changed shape'
    end
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    local setup_ok, setup_err = pcall(tools.setup, { workspace = work_dir })
    if not setup_ok then
        return nil, 'tools.setup failed: ' .. tostring(setup_err)
    end
    return tools
end

---True when the rejection carries no suggestion phrasing: nothing
---executable is offered to the caller.
---@param err string
---@return boolean
local function suggests_nothing(err)
    local lower = err:lower()
    return lower:find('did you mean', 1, true) == nil
        and lower:find('did-you-mean', 1, true) == nil
        and lower:find('perhaps you meant', 1, true) == nil
        and lower:find('suggestion', 1, true) == nil
end

---Assert a rejection table is a clean unknown-tool rejection.
---@param result table
---@param name string the hallucinated name that was called
---@return string? err
local function assert_unknown_tool(result, name)
    if type(result) ~= 'table' then
        return 'M.call(' .. name .. ') did not return a table'
    end
    if result.status ~= 'error' then
        return 'M.call(' .. name .. ') was NOT rejected: status=' .. tostring(result.status)
    end
    local err = tostring(result.error)
    if err:find('unknown tool', 1, true) == nil then
        return 'rejection is not an unknown-tool rejection: ' .. err
    end
    if not suggests_nothing(err) then
        return 'rejection suggests an alternative: ' .. err
    end
    return nil
end

---Facet V: a real tool name dispatches and runs. Write known bytes,
---read them back through the dispatcher by exact name.
---@param tools table
---@return string? err
local function facet_real_tool_dispatches(tools)
    ev('facet real-tool-dispatches: M.call("file_read") by exact name')
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    local marker = work_dir .. '/task69-marker.txt'
    local fh = io.open(marker, 'w')
    if fh == nil then
        return 'cannot write marker file'
    end
    fh:write('task-69 dispatch probe')
    fh:close()
    local result = tools.call('file_read', { path = 'task69-marker.txt' })
    if type(result) ~= 'table' or result.status ~= 'ok' then
        return 'real tool failed to dispatch: ' .. vim.inspect(result)
    end
    ev('file_read dispatched by exact name and returned the exact bytes')
    return nil
end

---Facet V: the design's hallucination example — `read_files_fast`
---when the real tool is `file_read` — is cleanly rejected.
---@param tools table
---@return string? err
local function facet_hallucination(tools)
    ev('facet hallucination: M.call("read_files_fast", {path="."}) — the design example')
    local result = tools.call('read_files_fast', { path = '.' })
    local err = assert_unknown_tool(result, 'read_files_fast')
    if err ~= nil then
        return err
    end
    ev('clean unknown_tool rejection: status=error, error names the hallucinated tool, '
        .. 'no fuzzy match ran, no did-you-mean was offered, nothing executed')
    return nil
end

---Edit distance between two ASCII strings (for asserting the battery
---really is distance-1; iterative DP, bounded by name length).
---@param a string
---@param b string
---@return integer
local function edit_distance(a, b)
    local la, lb = #a, #b
    local prev = {}
    for j = 0, lb do
        prev[j] = j
    end
    for i = 1, la do
        local curr = { [0] = i }
        for j = 1, lb do
            local cost = (a:sub(i, i) == b:sub(j, j)) and 0 or 1
            curr[j] = math.min(prev[j] + 1, curr[j - 1] + 1, prev[j - 1] + cost)
        end
        prev = curr
    end
    return prev[lb]
end

---Facet A: 10 edit-distance-1 near misses of real tools — including the
---privileged file_write — must ALL be rejected. Similarity is not
---authority. A would-be write through a near-miss name must leave no file.
---@param tools table
---@return string? err
local function facet_near_miss_battery(tools)
    local battery = {
        { 'file_rea', 'file_read' }, -- deletion
        { 'file_reads', 'file_read' }, -- insertion
        { 'gile_read', 'file_read' }, -- substitution
        { 'File_read', 'file_read' }, -- case substitution (exact match is case-sensitive)
        { 'file_writ', 'file_write' }, -- deletion, privileged tool
        { 'file_writes', 'file_write' }, -- substitution, privileged tool
        { 'file_writee', 'file_write' }, -- insertion, privileged tool
        { 'file_lis', 'file_list' }, -- deletion
        { 'file_listx', 'file_list' }, -- insertion
        { 'editor_chek', 'editor_check' }, -- deletion
    }
    ev('facet near-miss-battery: ' .. #battery .. ' edit-distance-1 names, incl. file_write')
    for _, pair in ipairs(battery) do
        local near, real = pair[1], pair[2]
        local d = edit_distance(near, real)
        if d ~= 1 then
            return 'battery construction error: ' .. near .. ' is distance ' .. d .. ' from ' .. real
        end
        local result = tools.call(near, { path = '.', limit = 10 })
        local err = assert_unknown_tool(result, near)
        if err ~= nil then
            return err
        end
    end
    ev('10/10 near misses rejected with unknown tool; no fuzzy matching engaged')
    -- The privileged near-miss with write-shaped args must not execute:
    -- no file may appear.
    local write_result = tools.call('file_writ', { path = 'task69-should-not-exist.txt', content = 'x' })
    local err = assert_unknown_tool(write_result, 'file_writ')
    if err ~= nil then
        return err
    end
    local probe = io.open(vim.env.GAUNTLET_WORK_DIR .. '/task69-should-not-exist.txt', 'r')
    if probe ~= nil then
        probe:close()
        return 'a near-miss of the privileged file_write EXECUTED a write (invented failure)'
    end
    ev('file_writ with valid write args created no file: the hallucination never executed')
    return nil
end

---Facet A: a hallucinated name WITH valid args is rejected on the NAME
---(unknown tool), while a real tool with bogus args fails on the ARGS
---(unknown argument) — proving name lookup precedes arg parsing.
---@param tools table
---@return string? err
local function facet_name_before_args(tools)
    ev('facet name-before-args: hallucinated name + args valid for the real file_read')
    local hallucinated = tools.call('file_rea', { path = '.' })
    local err = assert_unknown_tool(hallucinated, 'file_rea')
    if err ~= nil then
        return err
    end
    ev('rejected as unknown tool — the valid args were never parsed')
    local real_bad_args = tools.call('file_list', { bogus_arg = 'x' })
    if type(real_bad_args) ~= 'table' or real_bad_args.status ~= 'error' then
        return 'real tool with bogus args was not rejected'
    end
    local arg_err = tostring(real_bad_args.error)
    if arg_err:find('unknown argument', 1, true) == nil then
        return 'real-tool arg failure is not an arg failure: ' .. arg_err
    end
    ev('control: file_list with bogus args fails on the ARGS (unknown argument) — '
        .. 'so the hallucinated case failing on the NAME proves lookup order: '
        .. 'by_name[name] assert runs before validate_args')
    return nil
end

local function main()
    local _, boot_err = bootstrap()
    if boot_err ~= nil then
        return fail('bootstrap', boot_err)
    end
    ev('harness lua tree bootstrapped from DIVER_LUA_DIR')
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'real-tool-dispatches'
    end
    ev('scenario=' .. scenario)
    local tools, load_err = load_tools()
    if tools == nil then
        return fail('seam', load_err)
    end
    ev('driving the REAL ai.rose.tools.M.call (exact-match dispatcher)')
    local err
    if scenario == 'real-tool-dispatches' then
        err = facet_real_tool_dispatches(tools)
    elseif scenario == 'hallucination' then
        err = facet_hallucination(tools)
    elseif scenario == 'near-miss-battery' then
        err = facet_near_miss_battery(tools)
    elseif scenario == 'name-before-args' then
        err = facet_name_before_args(tools)
    else
        return fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
    end
    if err ~= nil then
        return fail('dispatch', err)
    end
    ev('PASS: the dispatcher is exact-match only — the near-miss battery proves '
        .. 'similarity is not authority, and rejections suggest nothing executable')
    return pass()
end

local ok, verdict = pcall(main)
if not ok then
    ev('lua error: ' .. tostring(verdict))
    verdict = fail('lua-driver', 'unhandled error: ' .. tostring(verdict))
end
-- Verdict on the real stdout: in `nvim --headless -l`, Lua print() goes
-- to stderr, but the Rust runner parses the verdict from stdout.
io.stdout:write(vim.json.encode(verdict) .. '\n')
