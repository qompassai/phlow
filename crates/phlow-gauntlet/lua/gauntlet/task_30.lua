-- task-30 driver: leader election under partition for diver's ai.harness.
--
-- Recon probe: the design asks for leader election among workers with
-- partition behavior. This driver inspects the REAL coordination modules
-- for election machinery — ai.harness.supervisor, the herd adapter, and
-- ai.herd / ai.herd.api (the multi-worker system the design names) —
-- looking for election APIs (elect/campaign/leader/heartbeat/quorum/
-- partition handling). It makes no network calls and spawns no workers;
-- it only reads the modules' exported function tables.
--
-- Honest result: none of these modules implements leader election. The
-- supervisor manages runs (spawn/create/finish/retry), the herd adapter
-- translates runs to remote herd workers, and ai.herd manages worker
-- agent processes (spawn/prompt/kill) — coordination of *tasks*, not
-- election of *leaders*. There is no quorum rule, no heartbeat-based
-- leader detection, no partition handling, so the design's scenarios
-- (3 workers elect one leader; 2-vs-1 partition keeps at most one
-- leader; dead-leader re-election within a bound) have no seam to run
-- against.
--
-- Fail-closed: if election APIs ever appear in these modules, the driver
-- reports where="recon" (premise changed) instead of the seam absence.
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

local function fail(where, how)
    return { id = 'task-30', outcome = 'fail', where = where, how = how, evidence = evidence }
end

---Module paths probed, in order. These are the coordination surfaces the
---design names: the supervisor, the herd adapter, and the herd worker
---system itself.
local PROBE_MODULES = {
    'ai.harness.supervisor',
    'ai.harness.adapters.herd',
    'ai.herd',
    'ai.herd.api',
}

---Name fragments that indicate election machinery. Matched
---case-insensitively against exported function names.
local ELECTION_NEEDLES = {
    'elect',
    'leader',
    'campaign',
    'ballot',
    'quorum',
    'heartbeat',
    'partition',
}

---@return table? harness
---@return string? err
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
    return harness
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

---Check one module for election API names. Returns the hits.
---@param path string
---@return string[] hits descriptions of what was probed
---@return string? err
local function probe_module(path)
    local ok, mod = pcall(require, path)
    if not ok then
        return {}, 'require failed: ' .. tostring(mod)
    end
    if type(mod) ~= 'table' then
        return {}, 'module is not a table'
    end
    local names = exported_names(mod)
    ev('probed ' .. path .. ': ' .. #names .. ' exported functions')
    local hits = {}
    for _, name in ipairs(names) do
        local lower = name:lower()
        for _, needle in ipairs(ELECTION_NEEDLES) do
            if lower:find(needle, 1, true) then
                hits[#hits + 1] = path .. '.' .. name
                break
            end
        end
    end
    return hits
end

local function main()
    local _, boot_err = bootstrap()
    if boot_err ~= nil then
        return fail('bootstrap', boot_err)
    end
    ev('harness bootstrapped from DIVER_LUA_DIR')
    ev('election needles: ' .. table.concat(ELECTION_NEEDLES, ', '))
    local all_hits = {}
    for _, path in ipairs(PROBE_MODULES) do
        local hits, err = probe_module(path)
        if err ~= nil then
            ev('note: ' .. path .. ': ' .. err)
        end
        for _, hit in ipairs(hits) do
            all_hits[#all_hits + 1] = hit
        end
    end
    if #all_hits > 0 then
        return fail(
            'recon',
            'election machinery now exists (' .. table.concat(all_hits, ', ') .. '); probe outdated'
        )
    end
    ev('no election APIs in ai.harness.supervisor (run lifecycle: spawn/create/finish/retry)')
    ev('no election APIs in ai.harness.adapters.herd (run <-> remote worker translation)')
    ev('no election APIs in ai.herd / ai.herd.api (worker agent processes: spawn/prompt/kill)')
    ev('diver coordinates tasks, not leaders: no quorum rule, no heartbeat-based leader detection, no partition handling')
    return fail(
        'seam',
        'seam absent: diver has no leader election machinery — the supervisor manages runs, herd manages worker processes; '
            .. 'no module elects a leader, detects leader death, or handles partitions. '
            .. 'The "at most one leader" invariant has no seam to assert against; open design gap.'
    )
end

local ok, verdict = pcall(main)
if not ok then
    ev('lua error: ' .. tostring(verdict))
    verdict = fail('lua-driver', 'unhandled error: ' .. tostring(verdict))
end
-- verdict on the real stdout: in `nvim --headless -l`, Lua print() goes
-- to stderr, but the Rust runner parses the verdict from stdout.
io.stdout:write(vim.json.encode(verdict) .. '\n')
