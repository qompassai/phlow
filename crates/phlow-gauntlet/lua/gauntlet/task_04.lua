-- Gauntlet task-04 driver: unknown adapter terminates invalid_adapter.
--
-- Regression probe of the diver agent harness (`ai.harness`): a bogus
-- adapter name must fail the run with an `invalid_adapter:` diagnostic --
-- no panic, no hang, no silent success. Runs one scenario selected by
-- GAUNTLET_SCENARIO and prints exactly one JSON verdict line to stdout.
--
-- Reads:  vim.env.DIVER_LUA_DIR (diver's lua/, appended to runtimepath),
--         vim.env.GAUNTLET_WORK_DIR (only writable directory),
--         vim.env.GAUNTLET_SCENARIO ("default" | "path-traversal" |
--         "empty" | "phlow-stub").
-- Prints: one JSON line {"id":"task-04","outcome":...,"evidence":[...]}.
-- Never writes outside GAUNTLET_WORK_DIR. Always exits 0.

local EVIDENCE_MAX = 64
local RUN_TIMEOUT_MS = 30000

local SCENARIO_ADAPTERS = {
    ['default'] = 'definitely-not-an-adapter',
    ['path-traversal'] = '../evil',
    ['empty'] = '',
    ['phlow-stub'] = 'phlow',
}

local evidence = {}

---Append one evidence line, newline-stripped, bounded.
---@param fmt string
---@param ... any
local function note(fmt, ...)
    if #evidence >= EVIDENCE_MAX then
        return
    end
    local line = string.format(fmt, ...)
    line = line:gsub('[\r\n]+', ' ')
    evidence[#evidence + 1] = line
end

---List sorted entry names of a directory, or nil on error.
---@param dir string
---@return string[]?
local function list_dir_names(dir)
    local ok, iter = pcall(vim.fs.dir, dir)
    if not ok or iter == nil then
        return nil
    end
    local names = {}
    for name, _ in iter do
        names[#names + 1] = name
    end
    table.sort(names)
    return names
end

---Find the first sink event of `kind` for `run_id`.
---@param sink table
---@param run_id string
---@param kind string
---@return table?
local function find_event(sink, run_id, kind)
    for _, event in ipairs(sink:events(run_id)) do
        if event.kind == kind then
            return event
        end
    end
    return nil
end

---The newest (only) run in the supervisor, or nil.
---@param supervisor_mod table
---@param sup table
---@return table?
local function latest_run(supervisor_mod, sup)
    local runs = supervisor_mod.list(sup)
    return runs[#runs]
end

---@param where string
---@param how string
---@return table verdict
local function fail(where, how)
    return {
        id = 'task-04',
        outcome = 'fail',
        where = where,
        how = how,
        evidence = evidence,
    }
end

---Check the unknown-adapter contract shared by "default"/"path-traversal".
---@param adapter string
---@param run_id string?
---@param run_err string?
---@param supervisor_mod table
---@param sup table
---@param sink table
---@return table? verdict on mismatch, nil when the contract holds
local function check_unknown_adapter(adapter, run_id, run_err, supervisor_mod, sup, sink)
    local expected_err = 'unknown adapter: ' .. adapter
    local expected_reason = 'invalid_adapter: ' .. expected_err
    note('run() returned run_id=%s err=%q', tostring(run_id), tostring(run_err))
    if run_id ~= nil then
        return fail('run', 'harness.run returned a run id for unknown adapter ' .. adapter)
    end
    if run_err ~= expected_err then
        note('expected err %q', expected_err)
        return fail('diagnostic', 'run() error mismatch for adapter ' .. adapter)
    end
    local run = latest_run(supervisor_mod, sup)
    if run == nil then
        return fail('supervisor', 'no run recorded for adapter ' .. adapter)
    end
    note('run state=%q adapter=%q', run.state, run.adapter)
    if run.state ~= 'failed' then
        return fail('lifecycle', 'run did not end failed for adapter ' .. adapter)
    end
    local finished = find_event(sink, run.id, 'run.finished')
    local reason = finished ~= nil and finished.payload.reason or nil
    note('run.finished reason=%q', tostring(reason))
    if reason ~= expected_reason then
        note('expected reason %q', expected_reason)
        return fail('diagnostic', 'invalid_adapter diagnostic mismatch for adapter ' .. adapter)
    end
    note('contract holds: failed run + invalid_adapter diagnostic naming %q', adapter)
    return nil
end

---@param supervisor_mod table
---@param sup table
---@param sink table
---@return table verdict
local function scenario_default(supervisor_mod, sup, sink)
    local run_id, run_err = require('ai.harness').run({
        adapter = SCENARIO_ADAPTERS['default'],
        goal = 'x',
        workflow = 'task-04-probe',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        timeout_ms = RUN_TIMEOUT_MS,
    })
    local verdict =
        check_unknown_adapter(SCENARIO_ADAPTERS['default'], run_id, run_err, supervisor_mod, sup, sink)
    if verdict ~= nil then
        return verdict
    end
    return { id = 'task-04', outcome = 'pass', evidence = evidence }
end

---@param supervisor_mod table
---@param sup table
---@param sink table
---@return table verdict
local function scenario_path_traversal(supervisor_mod, sup, sink)
    local parent = vim.fs.dirname(vim.env.GAUNTLET_WORK_DIR)
    local before = list_dir_names(parent)
    if before == nil then
        return fail('env', 'cannot list parent dir ' .. parent)
    end
    local run_id, run_err = require('ai.harness').run({
        adapter = SCENARIO_ADAPTERS['path-traversal'],
        goal = 'x',
        workflow = 'task-04-probe',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        timeout_ms = RUN_TIMEOUT_MS,
    })
    local after = list_dir_names(parent)
    if after == nil then
        return fail('env', 'cannot re-list parent dir ' .. parent)
    end
    local unchanged = #before == #after
    if unchanged then
        for i, name in ipairs(before) do
            if after[i] ~= name then
                unchanged = false
                break
            end
        end
    end
    note('parent dir entries before=%d after=%d unchanged=%s', #before, #after, tostring(unchanged))
    if not unchanged then
        return fail('filesystem', 'parent dir changed while rejecting ../evil adapter')
    end
    local verdict = check_unknown_adapter(
        SCENARIO_ADAPTERS['path-traversal'],
        run_id,
        run_err,
        supervisor_mod,
        sup,
        sink
    )
    if verdict ~= nil then
        return verdict
    end
    note('zero filesystem writes outside the work dir; ../evil is a plain registry table miss')
    return { id = 'task-04', outcome = 'pass', evidence = evidence }
end

---@param supervisor_mod table
---@param sup table
---@return table verdict
local function scenario_empty(supervisor_mod, sup)
    local expected_err = 'run spec.adapter must be a non-empty string when given'
    local run_id, run_err = require('ai.harness').run({
        adapter = '',
        goal = 'x',
        workflow = 'task-04-probe',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        timeout_ms = RUN_TIMEOUT_MS,
    })
    note('run() returned run_id=%s err=%q', tostring(run_id), tostring(run_err))
    if run_id ~= nil then
        return fail('run', 'harness.run returned a run id for empty adapter')
    end
    if run_err ~= expected_err then
        note('expected err %q', expected_err)
        return fail('validation', 'empty-adapter validation error mismatch')
    end
    local runs = supervisor_mod.list(sup)
    note('runs created=%d', #runs)
    if #runs ~= 0 then
        return fail('lifecycle', 'empty adapter created a run instead of failing validation')
    end
    note('clean validation error, no run created, no panic')
    return { id = 'task-04', outcome = 'pass', evidence = evidence }
end

---@param supervisor_mod table
---@param sup table
---@param sink table
---@return table verdict
local function scenario_phlow_stub(supervisor_mod, sup, sink)
    -- Documentary scenario: the Phase-6 phlow stub declines start() with an
    -- error string. Record exactly what happens; do NOT assume it behaves
    -- like an unknown adapter.
    local run_id, run_err = require('ai.harness').run({
        adapter = 'phlow',
        goal = 'x',
        workflow = 'task-04-probe',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        timeout_ms = RUN_TIMEOUT_MS,
    })
    note('run() returned run_id=%s err=%q', tostring(run_id), tostring(run_err))
    if run_id ~= nil then
        return fail('run', 'phlow stub unexpectedly accepted: returned a run id')
    end
    if type(run_err) ~= 'string' or run_err == '' then
        return fail('diagnostic', 'phlow stub returned no error string')
    end
    local run = latest_run(supervisor_mod, sup)
    if run == nil then
        return fail('supervisor', 'no run recorded for phlow adapter')
    end
    note('run state=%q adapter=%q', run.state, run.adapter)
    if run.state ~= 'failed' then
        return fail('lifecycle', 'phlow-stub run did not end failed')
    end
    local finished = find_event(sink, run.id, 'run.finished')
    local reason = finished ~= nil and finished.payload.reason or nil
    note('run.finished reason=%q', tostring(reason))
    local prefixed = type(reason) == 'string' and reason:sub(1, 16) == 'invalid_adapter:'
    note('reason carries invalid_adapter: prefix=%s', tostring(prefixed))
    for _, event in ipairs(sink:events(run.id)) do
        if event.kind == 'diagnostic.observed' then
            note('diagnostic.observed payload=%s', vim.inspect(event.payload):gsub('[\r\n]+', ' '))
        end
    end
    note('no panic, no hang, no silent success; stub declines start() synchronously')
    return { id = 'task-04', outcome = 'pass', evidence = evidence }
end

---@return table verdict
local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO or 'default'
    local adapter = SCENARIO_ADAPTERS[scenario]
    if adapter == nil then
        return fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. tostring(scenario))
    end
    note('scenario=%q adapter=%q', scenario, adapter)
    local diver_lua_dir = vim.env.DIVER_LUA_DIR or ''
    local work_dir = vim.env.GAUNTLET_WORK_DIR or ''
    if diver_lua_dir == '' then
        return fail('env', 'DIVER_LUA_DIR is empty')
    end
    if work_dir == '' then
        return fail('env', 'GAUNTLET_WORK_DIR is empty')
    end
    vim.opt.runtimepath:append(diver_lua_dir)
    local harness = require('ai.harness')
    local ok_setup, setup_err = harness.setup({})
    if not ok_setup then
        return fail('setup', 'harness.setup failed: ' .. tostring(setup_err))
    end
    note('harness.setup ok')
    local supervisor_mod = require('ai.harness.supervisor')
    local sup = harness._state.supervisor
    local sink = harness._state.sink
    local verdict
    if scenario == 'default' then
        verdict = scenario_default(supervisor_mod, sup, sink)
    elseif scenario == 'path-traversal' then
        verdict = scenario_path_traversal(supervisor_mod, sup, sink)
    elseif scenario == 'empty' then
        verdict = scenario_empty(supervisor_mod, sup)
    else
        verdict = scenario_phlow_stub(supervisor_mod, sup, sink)
    end
    return verdict
end

-- Print exactly one JSON line. pcall guards the whole attempt so a Lua
-- error still becomes a fail verdict; the process always exits 0.
local ok, verdict = pcall(main)
if not ok then
    verdict = fail('driver', 'lua error: ' .. tostring(verdict))
end
io.write(vim.json.encode(verdict) .. '\n')
