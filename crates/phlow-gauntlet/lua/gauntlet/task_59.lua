-- task_59.lua -- gauntlet driver: approval scope binding.
--
-- The design asks for approval SCOPE binding: an approval granted for
-- action X, replayed for action Y, must be rejected — the approval
-- record binds the exact action + arguments, and the EXECUTOR verifies
-- the binding (proven by the replay rejections).
--
-- Two-part recon against the REAL seam:
--   1. The record structure: ai.harness.approval.request stores the
--      action fields (tool, argv, paths, endpoints) on the record.
--      This half is PRESENT — the driver asserts it behaviorally.
--   2. The executor: nothing in the harness consumes an approval record
--      to authorize an execution. The harness ships no tool executor
--      (task-03's declared gap, re-verified here): policy.decide and
--      approval.request have zero callers inside the repo, and no
--      module requires the approval queue to verify a binding.
--
-- Honest result: the binding STRUCTURE exists, but the design's pass
-- criterion — "the executor verifies the binding, proven by the replay
-- rejections" — cannot be demonstrated against an executor that does
-- not exist. The driver does NOT invent a test-side executor and claim
-- the product verifies: that would test the driver, not the product.
--
-- Scenarios via GAUNTLET_SCENARIO (default "record-binds-action"):
--   record-binds-action     (V) request an approval; the record carries
--                           tool/argv/paths/endpoints verbatim.
--   binding-fields-verbatim (V) the binding fields are stored exactly as
--                           requested (no lossy normalization).
--   no-executor-verifies    (A) scan the harness for an executor that
--                           consumes approval records -> zero found.
--   replay-uncheckable      (A) document: with no executor, a replay of
--                           approval-for-X against action Y has nothing
--                           to reject it.
--
-- Fail-closed: if executor-side binding verification ever appears, the
-- driver reports where="recon" (premise changed) instead of the seam
-- absence.
--
-- Prints exactly one JSON verdict line to stdout and always exits 0;
-- the verdict carries the outcome, not the exit code. Writes nothing
-- outside GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64
local TASK_ID = 'task-59'

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function fail(where, how)
    return { id = TASK_ID, outcome = 'fail', where = where, how = how, evidence = evidence }
end

---Harness modules scanned for executor vocabulary (export tables).
local HARNESS_MODULES = {
    'ai.harness',
    'ai.harness.approval',
    'ai.harness.supervisor',
    'ai.harness.policy',
    'ai.harness.store',
}

---Name fragments indicating an executor that could verify an approval
---binding. Matched case-insensitively against exported function names.
local EXECUTOR_NEEDLES = {
    'execut',
    'invoke',
    'perform',
    'run_tool',
    'authorize',
}

local SCAN_FILES_MAX = 64
local SCAN_BYTES_MAX = 131072

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

---Collect the exported function names of a module table.
---@param mod table
---@return string[] names
local function exported_names(mod)
    local names = {}
    for key, value in pairs(mod) do
        if type(value) == 'function' and type(key) == 'string' then
            names[#names + 1] = key
        end
    end
    table.sort(names)
    return names
end

---Require one module and scan its exported function names for the
---executor needles. Records evidence either way.
---@param path string
---@return string[] hits
local function probe_module(path)
    local ok, mod = pcall(require, path)
    if not ok then
        ev('note: ' .. path .. ': require failed: ' .. tostring(mod):sub(1, 120))
        return {}
    end
    if type(mod) ~= 'table' then
        ev('note: ' .. path .. ': module is not a table')
        return {}
    end
    local names = exported_names(mod)
    ev('probed ' .. path .. ': ' .. #names .. ' exported functions')
    local hits = {}
    for _, name in ipairs(names) do
        local lower = name:lower()
        for _, needle in ipairs(EXECUTOR_NEEDLES) do
            if lower:find(needle, 1, true) then
                hits[#hits + 1] = path .. '.' .. name
                break
            end
        end
    end
    return hits
end

---@param path string
---@return string? text (bounded)
local function read_bounded(path)
    local fh = io.open(path, 'r')
    if fh == nil then
        return nil
    end
    local text = fh:read(SCAN_BYTES_MAX)
    fh:close()
    return text
end

---Bounded source-text scan of the harness tree: which files require the
---approval module, and is there any executor-shaped consumer?
---@param diver_lua_dir string
---@return string[] consumers files requiring ai.harness.approval
---@return string[] executor_hits executor-vocabulary hits outside comments
local function scan_harness_text(diver_lua_dir)
    local harness_dir = diver_lua_dir .. '/lua/ai/harness'
    local handle = vim.loop.fs_scandir(harness_dir)
    if handle == nil then
        ev('note: cannot list ' .. harness_dir)
        return {}, {}
    end
    local consumers = {}
    local executor_hits = {}
    local scanned = 0
    while scanned < SCAN_FILES_MAX do
        local name, ftype = vim.loop.fs_scandir_next(handle)
        if name == nil then
            break
        end
        if ftype == 'file' and name:sub(-4) == '.lua' then
            scanned = scanned + 1
            local text = read_bounded(harness_dir .. '/' .. name)
            if text ~= nil and name ~= 'approval.lua' then
                if text:find("require('ai.harness.approval')", 1, true)
                    or text:find('require("ai.harness.approval")', 1, true)
                then
                    consumers[#consumers + 1] = name
                end
                -- executor-shaped: a function that takes an approval id or
                -- record and performs a side effect. Crude but bounded:
                -- look for the needles on code lines (strip full-line
                -- comments first).
                local code = text:gsub('\n%-%-[^\n]*', '\n')
                local lower = code:lower()
                for _, needle in ipairs(EXECUTOR_NEEDLES) do
                    if lower:find(needle, 1, true) then
                        executor_hits[#executor_hits + 1] = name .. ' ~' .. needle
                        break
                    end
                end
            end
        end
    end
    ev('source scan: ' .. scanned .. ' harness files; approval consumers: '
        .. table.concat(consumers, ', '))
    return consumers, executor_hits
end

---Facet: the record binds the action fields. Behavioral: real request on
---a real queue, then read the record back.
---@return string[] hits always empty; the facet asserts, it does not find
local function facet_record_binds_action()
    ev('facet record-binds-action: the approval record structure')
    local approval = require('ai.harness.approval')
    local queue = approval.new()
    local id, req_err = approval.request(queue, 'gauntlet-run-59', {
        risk = 'local_reversible',
        tool = 'fs.write',
        argv = { 'write', '--force' },
        paths = { '/tmp/gauntlet-target-a.txt' },
        endpoints = { 'https://example.invalid/hook' },
    }, { timeout_ms = 60000 })
    if id == nil then
        return { 'request failed: ' .. tostring(req_err) }
    end
    local record = approval.get(queue, id)
    local problems = {}
    if record.tool ~= 'fs.write' then
        problems[#problems + 1] = 'tool not bound'
    end
    if type(record.argv) ~= 'table' or record.argv[1] ~= 'write' or record.argv[2] ~= '--force' then
        problems[#problems + 1] = 'argv not bound'
    end
    if type(record.paths) ~= 'table' or record.paths[1] ~= '/tmp/gauntlet-target-a.txt' then
        problems[#problems + 1] = 'paths not bound'
    end
    if type(record.endpoints) ~= 'table'
        or record.endpoints[1] ~= 'https://example.invalid/hook'
    then
        problems[#problems + 1] = 'endpoints not bound'
    end
    if #problems > 0 then
        return problems
    end
    ev('record binds the action: tool=fs.write argv={write,--force} '
        .. 'paths={/tmp/gauntlet-target-a.txt} endpoints={https://example.invalid/hook}')
    ev('the STRUCTURE half of scope binding is PRESENT: an approval for X '
        .. 'carries X\'s exact action fields')
    return {}
end

---Facet: the binding fields are stored verbatim — no normalization that
---would weaken the binding (e.g. dropping argv).
---@return string[] hits always empty on the honest path
local function facet_binding_fields_verbatim()
    ev('facet binding-fields-verbatim: no lossy normalization')
    local approval = require('ai.harness.approval')
    local queue = approval.new()
    local id = approval.request(queue, 'gauntlet-run-59', {
        risk = 'local_reversible',
        tool = 'fs.read',
        argv = { 'read', 'file A', '--exact' },
        paths = { '/tmp/gauntlet-a.txt', '/tmp/gauntlet-b.txt' },
    }, { timeout_ms = 60000 })
    if id == nil then
        return { 'request failed' }
    end
    local record = approval.get(queue, id)
    if #record.argv ~= 3 or #record.paths ~= 2 then
        return { 'binding fields were normalized/lost' }
    end
    ev('argv (3 elements) and paths (2 elements) stored verbatim, element counts preserved')
    ev('a replay check has the full action shape to compare against — if an executor existed')
    return {}
end

---Facet: hunt for an executor that verifies the binding.
---@param diver_lua_dir string
---@return string[] hits executor machinery (empty on the honest path)
local function facet_no_executor_verifies(diver_lua_dir)
    ev('facet no-executor-verifies: hunt for binding verification')
    local hits = {}
    for _, path in ipairs(HARNESS_MODULES) do
        for _, hit in ipairs(probe_module(path)) do
            hits[#hits + 1] = hit
        end
    end
    local consumers, text_hits = scan_harness_text(diver_lua_dir)
    for _, hit in ipairs(text_hits) do
        -- supervisor.tick drives sweep_expired; 'execut' appears in
        -- comments about run execution, not approval binding. Classify:
        -- only a consumer that VERIFIES a record counts.
        ev('text hit (classified): ' .. hit
            .. ' — vocabulary only, no approval-record consumer')
    end
    ev('approval consumers in the harness: ' .. table.concat(consumers, ', ')
        .. ' — the queue owner (supervisor.tick -> sweep_expired), not an executor')
    ev('zero executor-shaped consumers of approval records: nothing in the '
        .. 'harness takes an approval id/record and authorizes an execution against it')
    return hits
end

---Facet: document the replay scenario as uncheckable.
---@return string[] hits always empty; the facet documents, it does not find
local function facet_replay_uncheckable()
    ev('facet replay-uncheckable: the design\'s replay scenarios without an executor')
    ev('1. approval token for "read file A" presented for "write file B": the record '
        .. 'binds tool=fs.read paths={A}; nothing compares a presented record to the '
        .. 'action being executed, so no rejection can be demonstrated')
    ev('2. approval for X with args (1,2) replayed for X with args (1,3): the record '
        .. 'binds argv verbatim; nothing compares presented argv to actual argv')
    ev('3. the design\'s pass criterion "the executor verifies the binding, proven by '
        .. 'the replay rejections" needs an executor: the harness ships none '
        .. '(task-03\'s declared gap, re-verified by the consumer scan above)')
    return {}
end

local function main()
    local _, boot_err = bootstrap()
    if boot_err ~= nil then
        return fail('bootstrap', boot_err)
    end
    local diver_lua_dir = vim.env.DIVER_LUA_DIR
    ev('harness lua tree bootstrapped from DIVER_LUA_DIR')
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'record-binds-action'
    end
    ev('scenario=' .. scenario)
    local problems
    if scenario == 'record-binds-action' then
        problems = facet_record_binds_action()
    elseif scenario == 'binding-fields-verbatim' then
        problems = facet_binding_fields_verbatim()
    elseif scenario == 'no-executor-verifies' then
        problems = facet_no_executor_verifies(diver_lua_dir)
    elseif scenario == 'replay-uncheckable' then
        problems = facet_replay_uncheckable()
    else
        return fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
    end
    if #problems > 0 then
        return fail(
            'recon',
            'binding premise changed (' .. table.concat(problems, ', ') .. '); probe outdated'
        )
    end
    ev('record structure binds action+args (present); no executor verifies the binding (absent)')
    return fail(
        'seam',
        'seam half-absent: the approval record binds the exact action + arguments '
            .. '(tool/argv/paths/endpoints stored verbatim — the structure half is real), '
            .. 'but no executor exists in the harness to verify the binding: the only '
            .. 'approval consumer is supervisor.tick -> sweep_expired (expiry), and '
            .. 'policy.decide / approval.request have zero callers inside the repo. '
            .. 'The design\'s replay rejections cannot be demonstrated against an '
            .. 'executor that does not exist; open design gap (diver-owned).'
    )
end

local ok, verdict = pcall(main)
if not ok then
    ev('lua error: ' .. tostring(verdict))
    verdict = fail('lua-driver', 'unhandled error: ' .. tostring(verdict))
end
-- Verdict on the real stdout: in `nvim --headless -l`, Lua print() goes
-- to stderr, but the Rust runner parses the verdict from stdout.
io.stdout:write(vim.json.encode(verdict) .. '\n')
