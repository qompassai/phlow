-- task-51 driver: byzantine worker detection recon probe.
--
-- The design asks for byzantine worker detection on the result-aggregation
-- path: 1 of 3 workers returns plausible-but-wrong results, redundancy /
-- quorum outvotes it (the wrong answer never becomes the verdict), a
-- worker that equivocates (different wrong answers to different quorums)
-- is detected via signed/attributed results, and the dissenting worker is
-- identified in the evidence.
--
-- Recon probe: the driver inspects the REAL fan-out consumers and
-- verifiers — ai.a2a.fanout, ai.a2a.orchestrator, ai.harness.supervisor,
-- ai.harness.verdict — reading their exported function tables (and, for
-- the attribution facet, the real result shapes in source) for
-- aggregation/quorum/voting/dissent machinery. It makes no network calls
-- and spawns no workers.
--
-- Honest result: the aggregation half of the design's seam exists only
-- as spec-order collection. ai.a2a.fanout.run hands the same job to N
-- agents and calls one callback with every result in spec order;
-- ai.a2a.orchestrator collects per-language results with an on_partial
-- callback; ai.harness.verdict.evaluate grades ONE run's acceptance
-- criteria. No module computes a verdict over multiple workers'
-- answers, no quorum/voting rule exists, and no dissenter is
-- identified. Results ARE attributed per worker (agent / language /
-- spec index), so equivocation would be *visible* — but nothing reads
-- the attributed results to detect it.
--
-- Fail-closed: if aggregation APIs ever appear in these modules, the
-- driver reports where="recon" (premise changed) instead of the seam
-- absence.
--
-- Scenarios via GAUNTLET_SCENARIO (default "fanout-consumers"):
--   fanout-consumers          probe ai.a2a.fanout + ai.a2a.orchestrator
--   verifiers                 probe ai.harness.supervisor + ai.harness.verdict
--   attribution-without-verdict
--                             document attributed results, show no verdict
--                             logic reads them
--   fail-closed-recon         union scan; hits -> where="recon"
--
-- Prints exactly one JSON verdict line to stdout and always exits 0; the
-- verdict carries the outcome, not the exit code. Writes nothing outside
-- GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64
local TASK_ID = 'task-51'

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function fail(where, how)
    return { id = TASK_ID, outcome = 'fail', where = where, how = how, evidence = evidence }
end

---Modules that own the fan-out consumer path (scatter-gather) and the
---verifier path (per-run acceptance + run lifecycle).
local FANOUT_MODULES = { 'ai.a2a.fanout', 'ai.a2a.orchestrator' }
local VERIFIER_MODULES = { 'ai.harness.supervisor', 'ai.harness.verdict' }

---Name fragments indicating result-aggregation / byzantine machinery.
---Matched case-insensitively against exported function names.
local AGGREGATION_NEEDLES = {
    'aggregat',
    'quorum',
    'byzantine',
    'equivoc',
    'majority',
    'ballot',
    'dissent',
    'attest',
}

---Source files (relative to the diver lua tree) whose result shapes the
---attribution facet reads. Both the rtp layout (<dir>/lua/...) and the
---flat layout (<dir>/...) are tried.
local RESULT_SOURCE_FILES = {
    'ai/a2a/fanout.lua',
    'ai/a2a/orchestrator.lua',
}

---@return string? diver_lua_dir
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
    return diver_lua_dir
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
---aggregation needles. Records evidence either way.
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
    ev('probed ' .. path .. ': ' .. #names .. ' exported functions (' .. table.concat(names, ', ') .. ')')
    local hits = {}
    for _, name in ipairs(names) do
        local lower = name:lower()
        for _, needle in ipairs(AGGREGATION_NEEDLES) do
            if lower:find(needle, 1, true) then
                hits[#hits + 1] = path .. '.' .. name
                break
            end
        end
    end
    return hits
end

---Read a diver source file, trying the rtp layout then the flat layout.
---@param diver_lua_dir string
---@param rel string
---@return string? text
local function read_source(diver_lua_dir, rel)
    for _, base in ipairs({ diver_lua_dir .. '/lua/' .. rel, diver_lua_dir .. '/' .. rel }) do
        local handle = io.open(base, 'r')
        if handle ~= nil then
            local text = handle:read('*a')
            handle:close()
            return text
        end
    end
    return nil
end

---Facet: the fan-out consumers collect results but compute no verdict.
---Returns the aggregation-API hits (expected: none).
---@return string[] hits
local function facet_fanout_consumers()
    ev('facet fanout-consumers: the scatter-gather path the design names')
    local hits = {}
    for _, path in ipairs(FANOUT_MODULES) do
        for _, hit in ipairs(probe_module(path)) do
            hits[#hits + 1] = hit
        end
    end
    ev('ai.a2a.fanout.run: one callback with every result IN SPEC ORDER — collection, not verdict')
    ev('ai.a2a.orchestrator: per-language results + on_partial — collection, not verdict')
    ev('no quorum / voting / majority rule in either consumer: the wrong answer is never outvoted because nothing votes')
    return hits
end

---Facet: the verifiers grade single runs; none arbitrates between workers.
---Returns the aggregation-API hits (expected: none).
---@return string[] hits
local function facet_verifiers()
    ev('facet verifiers: per-run acceptance and the run lifecycle')
    local hits = {}
    for _, path in ipairs(VERIFIER_MODULES) do
        for _, hit in ipairs(probe_module(path)) do
            hits[#hits + 1] = hit
        end
    end
    ev('ai.harness.verdict.evaluate: grades ONE run against acceptance criteria — no multi-worker arbitration')
    ev('ai.harness.supervisor: run lifecycle (create/start/finish/retry) — no cross-worker result comparison')
    ev('distinct from task-19: phlow-council votes on opinions; nothing here votes on worker results at all')
    return hits
end

---Facet: results are attributed per worker (so equivocation would be
---visible), but no module reads the attributed results to detect it.
---@param diver_lua_dir string
---@return string[] hits always empty; the facet documents, it does not find
local function facet_attribution(diver_lua_dir)
    ev('facet attribution-without-verdict: what the design\'s "signed/attributed results" need')
    local seen_attribution = {}
    for _, rel in ipairs(RESULT_SOURCE_FILES) do
        local text = read_source(diver_lua_dir, rel)
        if text == nil then
            ev('note: could not read ' .. rel)
        else
            -- The result shapes carry per-worker attribution fields.
            for _, field in ipairs({ 'agent', 'lang', 'state', 'error' }) do
                if text:find(field .. ' =', 1, true) then
                    seen_attribution[#seen_attribution + 1] = rel .. ': ' .. field
                end
            end
        end
    end
    if #seen_attribution > 0 then
        ev('attributed results EXIST: ' .. table.concat(seen_attribution, '; '))
    else
        ev('note: no attribution fields found in the result shapes')
    end
    ev('but no module consumes the attribution to detect a liar: fanout hands raw results to the caller; the caller is diver UI/operator code, not a quorum')
    ev('equivocation (different wrong answers to different quorums) is therefore visible in principle and detected in practice by nothing')
    return {}
end

---Facet: fail-closed union scan over every probed module.
---@return string[] hits
local function facet_fail_closed()
    ev('facet fail-closed-recon: union scan over all fan-out/verifier modules')
    local hits = {}
    for _, path in ipairs(FANOUT_MODULES) do
        for _, hit in ipairs(probe_module(path)) do
            hits[#hits + 1] = hit
        end
    end
    for _, path in ipairs(VERIFIER_MODULES) do
        for _, hit in ipairs(probe_module(path)) do
            hits[#hits + 1] = hit
        end
    end
    return hits
end

local function main()
    local diver_lua_dir, boot_err = bootstrap()
    if diver_lua_dir == nil then
        return fail('bootstrap', boot_err)
    end
    ev('harness lua tree bootstrapped from DIVER_LUA_DIR')
    ev('aggregation needles: ' .. table.concat(AGGREGATION_NEEDLES, ', '))
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'fanout-consumers'
    end
    ev('scenario=' .. scenario)
    local hits
    if scenario == 'fanout-consumers' then
        hits = facet_fanout_consumers()
    elseif scenario == 'verifiers' then
        hits = facet_verifiers()
    elseif scenario == 'attribution-without-verdict' then
        hits = facet_attribution(diver_lua_dir)
    elseif scenario == 'fail-closed-recon' then
        hits = facet_fail_closed()
    else
        return fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
    end
    if #hits > 0 then
        return fail(
            'recon',
            'aggregation machinery now exists (' .. table.concat(hits, ', ') .. '); probe outdated'
        )
    end
    ev('zero aggregation-API hits: no quorum, no voting, no byzantine detection, no dissenter identification')
    return fail(
        'seam',
        'seam absent: diver\'s fan-out consumers collect attributed per-worker results in order but compute no verdict over them — '
            .. 'no quorum/voting rule outvotes a lying worker, no module identifies the dissenter, and nothing detects equivocation. '
            .. 'The design\'s "final verdict equals the honest majority" has no seam to assert against; open design gap (diver-owned).'
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
