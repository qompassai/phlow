-- task-57 driver: escalation chains recon probe.
--
-- The design asks for an approval ROUTING table (L1 -> L2 -> ...): the
-- first approver in the chain is unavailable, the request routes to L2
-- with the full context; chain order is configuration, not code;
-- exhaustion denies; a late L1 response is recorded but never
-- double-decides.
--
-- Recon probe: the driver inspects the REAL approval surfaces —
-- ai.harness.approval, ai.harness.supervisor, ai.harness (the public
-- setup/run/cancel/resume API) — reading exported function tables for
-- routing vocabulary, plus a BOUNDED source-text scan of the harness
-- tree for the same vocabulary. It makes no network calls and spawns no
-- workers.
--
-- Honest result: no routing table exists. The approval module exposes
-- request/decide/get/pending/sweep_expired on a FLAT queue: one request,
-- one decision, no levels, no next-approver, no delegation. The harness
-- public API (setup/run/cancel/resume/version) takes no approver-chain
-- configuration. The design's three required artifacts — a routing
-- table, chain order as configuration, an exhaustion -> deny rule — are
-- all absent.
--
-- Unrelated vocabulary the scan WILL find (recorded, never counted as
-- routing): store.lua's "fallback" (a hash-prefix label for the content
-- store, not an approver fallback) and adapter.lua's "delegates to a
-- verified native call" (a comment about start(), not approval
-- delegation).
--
-- Fail-closed: if routing machinery ever appears on the approval path,
-- the driver reports where="recon" (premise changed) instead of the
-- seam absence.
--
-- Scenarios via GAUNTLET_SCENARIO (default "routing-surface"):
--   routing-surface         probe approval-path export tables for
--                           routing vocabulary
--   chain-order-config      probe the harness setup/config surface for
--                           approver-chain configuration
--   exhaustion-undefined    document: no chain, so exhaustion -> deny
--                           and late-response-after-escalation are vacuous
--   fail-closed-recon       union scan; hits -> where="recon"
--
-- Prints exactly one JSON verdict line to stdout and always exits 0; the
-- verdict carries the outcome, not the exit code. Writes nothing outside
-- GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64
local TASK_ID = 'task-57'

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function fail(where, how)
    return { id = TASK_ID, outcome = 'fail', where = where, how = how, evidence = evidence }
end

---Modules on the real approval path.
local APPROVAL_MODULES = {
    'ai.harness.approval',
    'ai.harness.supervisor',
    'ai.harness',
}

---Name fragments indicating approval ROUTING machinery. Matched
---case-insensitively against exported function names.
local ROUTING_NEEDLES = {
    'escalat',
    'rout',
    'chain',
    'delegat',
    'fallback',
    'next_approver',
    'approver_level',
}

---Name fragments for the bounded source-text scan of the harness tree.
---Broader than the export scan on purpose: the scan must FIND the
---unrelated vocabulary (store.lua's "fallback", adapter.lua's
---"delegates") so the zero-routing finding is evidenced, not assumed.
local TEXT_NEEDLES = {
    'escalat',
    'routing',
    'approver chain',
    'approver_chain',
    'next_approver',
    'fallback_approver',
    'delegat',
}

---Known-unrelated text hits: (file fragment, needle, why-unrelated).
local UNRELATED_HITS = {
    { 'store.lua', 'fallback', 'hash-prefix label for the content store, not an approver fallback' },
    { 'adapter.lua', 'delegat', 'comment: start() delegates to a verified native call, not approval delegation' },
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
---routing needles. Records evidence either way.
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
        for _, needle in ipairs(ROUTING_NEEDLES) do
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

---Bounded source-text scan of the harness tree for routing vocabulary.
---Lists the harness dir via vim.loop; unrelated hits are classified, not
---counted.
---@param diver_lua_dir string
---@return string[] routing_hits (empty when the seam is absent)
local function scan_harness_text(diver_lua_dir)
    local harness_dir = diver_lua_dir .. '/lua/ai/harness'
    local handle = vim.loop.fs_scandir(harness_dir)
    if handle == nil then
        ev('note: cannot list ' .. harness_dir)
        return {}
    end
    local routing_hits = {}
    local scanned = 0
    while scanned < SCAN_FILES_MAX do
        local name, ftype = vim.loop.fs_scandir_next(handle)
        if name == nil then
            break
        end
        if ftype == 'file' and name:sub(-4) == '.lua' then
            scanned = scanned + 1
            local text = read_bounded(harness_dir .. '/' .. name)
            if text ~= nil then
                local lower = text:lower()
                for _, needle in ipairs(TEXT_NEEDLES) do
                    if lower:find(needle, 1, true) then
                        local unrelated = false
                        for _, known in ipairs(UNRELATED_HITS) do
                            if name:find(known[1], 1, true)
                                and needle:find(known[2], 1, true)
                            then
                                ev('text hit (UNRELATED): ' .. name .. ' ~' .. needle
                                    .. ' — ' .. known[3])
                                unrelated = true
                                break
                            end
                        end
                        if not unrelated then
                            routing_hits[#routing_hits + 1] = name .. ' ~' .. needle
                        end
                        break
                    end
                end
            end
        end
    end
    ev('source scan: ' .. scanned .. ' harness files, ' .. #routing_hits .. ' routing hits')
    return routing_hits
end

---Facet: the approval path is a flat queue, not a chain.
---@return string[] hits
local function facet_routing_surface()
    ev('facet routing-surface: the real approval path export tables')
    local hits = {}
    for _, path in ipairs(APPROVAL_MODULES) do
        for _, hit in ipairs(probe_module(path)) do
            hits[#hits + 1] = hit
        end
    end
    ev('ai.harness.approval: request/decide/get/pending/sweep_expired on a FLAT queue — '
        .. 'one request, one decision; no levels, no next-approver, no delegation')
    return hits
end

---Facet: chain order as configuration — the public setup() surface.
---@return string[] hits
local function facet_chain_order_config()
    ev('facet chain-order-config: the harness setup/config surface')
    local hits = {}
    for _, path in ipairs(APPROVAL_MODULES) do
        for _, hit in ipairs(probe_module(path)) do
            hits[#hits + 1] = hit
        end
    end
    local ok, harness = pcall(require, 'ai.harness')
    if ok and type(harness) == 'table' then
        ev('ai.harness public API: setup/run/cancel/resume/version — '
            .. 'no approver-chain option on any entry point')
    end
    ev('chain order is not configuration: there is no approver-level option '
        .. 'to configure, because there is no chain to order')
    return hits
end

---Facet: document the design's escalation scenarios as vacuous.
---@return string[] hits always empty; the facet documents, it does not find
local function facet_exhaustion_undefined()
    ev('facet exhaustion-undefined: the design\'s escalation scenarios without a chain')
    ev('1. L1 unavailable -> route to L2 with full context: no L1, no L2, no route — '
        .. 'the request waits on the flat queue until decided or expired')
    ev('2. every level unavailable -> denied after chain exhausts: no chain, no exhaustion '
        .. 'rule — a silent queue expires to denied by the timeout path (task-56), not by chain exhaustion')
    ev('3. L1 responds after escalation (recorded, no double-decide): no escalation exists, '
        .. 'so there is no late-escalation double-decide to guard; per-request decide-once '
        .. '(approval.decide rejects non-pending) is the only related guard')
    ev('4. every hop logged with cause: no hops exist, so no hop log exists')
    return {}
end

---Facet: fail-closed union scan over export tables + source text.
---@param diver_lua_dir string
---@return string[] hits
local function facet_fail_closed(diver_lua_dir)
    ev('facet fail-closed-recon: union scan over approval-path exports + harness source text')
    local hits = {}
    for _, path in ipairs(APPROVAL_MODULES) do
        for _, hit in ipairs(probe_module(path)) do
            hits[#hits + 1] = hit
        end
    end
    for _, hit in ipairs(scan_harness_text(diver_lua_dir)) do
        hits[#hits + 1] = hit
    end
    return hits
end

local function main()
    local _, boot_err = bootstrap()
    if boot_err ~= nil then
        return fail('bootstrap', boot_err)
    end
    local diver_lua_dir = vim.env.DIVER_LUA_DIR
    ev('harness lua tree bootstrapped from DIVER_LUA_DIR')
    ev('routing needles: ' .. table.concat(ROUTING_NEEDLES, ', '))
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'routing-surface'
    end
    ev('scenario=' .. scenario)
    local hits
    if scenario == 'routing-surface' then
        hits = facet_routing_surface()
    elseif scenario == 'chain-order-config' then
        hits = facet_chain_order_config()
    elseif scenario == 'exhaustion-undefined' then
        hits = facet_exhaustion_undefined()
    elseif scenario == 'fail-closed-recon' then
        hits = facet_fail_closed(diver_lua_dir)
    else
        return fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
    end
    if #hits > 0 then
        return fail(
            'recon',
            'approval routing machinery now exists (' .. table.concat(hits, ', ') .. '); probe outdated'
        )
    end
    ev('zero routing hits on the approval path: no routing table, no chain, no next-approver, no delegation')
    return fail(
        'seam',
        'seam absent: diver has no approval routing table — ai.harness.approval is a flat queue '
            .. '(request/decide/get/pending/sweep_expired), the harness public API takes no approver-chain '
            .. 'configuration, and no exhaustion->deny rule exists for a chain that does not exist. '
            .. 'The design\'s "chain order is configuration, not code" has no configuration surface to assert '
            .. 'against; open design gap (diver-owned).'
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
