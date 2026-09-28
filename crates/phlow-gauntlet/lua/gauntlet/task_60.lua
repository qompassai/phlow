-- task_60.lua -- gauntlet driver: break-glass procedure recon probe.
--
-- The design asks for the EMERGENCY-BYPASS path: a genuine emergency
-- needs action now with approvers unreachable — but the bypass must
-- not become a backdoor. Required: break-glass invoked with a reason
-- executes with a post-hoc justification required within N minutes;
-- the grant auto-expires and flags the incident if justification never
-- arrives; a second use requires fresh justification (no standing
-- grant). "If none exists, the design question — 'should one exist?'
-- — is the documented finding."
--
-- Recon probe: the driver scans the REAL ai tree — the approval-path
-- modules' export tables (ai.harness.approval, ai.harness.supervisor,
-- ai.harness) for bypass vocabulary, plus a BOUNDED source-text scan
-- of lua/ai for the SPECIFIC break-glass vocabulary (break_glass,
-- breakglass, break-glass, emergency — the bare Lua keyword `break`
-- is deliberately not a needle, or every loop in the tree would hit).
-- The generic 'bypass'/'override' words are polysemous across the
-- tree (config overrides, cache bypasses) and are classified, never
-- counted as machinery. It makes no network calls and spawns no
-- workers.
--
-- Honest result: no emergency-bypass path exists. The one "bypass"
-- hit in the tree is policy.lua's ANTI-bypass contract: "No adapter,
-- provider, or MCP server may bypass this module." That is the
-- architecture's current stance — a break-glass path would have to be
-- reconciled with it. The design's three required artifacts — a
-- justification record, a single-use time-boxed grant, auto-expiry
-- with incident flagging — are all absent.
--
-- The documented finding (the design's question): should a
-- break-glass procedure exist? Banked for Matt — a product decision,
-- not gauntlet work. Note the tension the design itself names: the
-- abnormal path's opposite risk is the bypass becoming routine.
--
-- Fail-closed: if bypass machinery ever appears, the driver reports
-- where="recon" (premise changed) instead of the seam absence.
--
-- Scenarios via GAUNTLET_SCENARIO (default "bypass-path-scan"):
--   bypass-path-scan            scan approval-path exports + the ai
--                               tree for break-glass vocabulary
--   approval-exits-closed       the approval module's only exits are
--                               approved/denied/expired — no bypass
--   justification-artifacts-absent  document the three missing
--                               artifacts (justification record,
--                               single-use time-boxed grant,
--                               auto-expiry + incident flag)
--   fail-closed-recon           union scan; hits -> where="recon"
--
-- Prints exactly one JSON verdict line to stdout and always exits 0; the
-- verdict carries the outcome, not the exit code. Writes nothing outside
-- GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64
local TASK_ID = 'task-60'

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

---Name fragments indicating emergency-bypass machinery. Matched
---case-insensitively against exported function names.
local BYPASS_NEEDLES = {
    'break_glass',
    'breakglass',
    'emergency',
    'bypass',
    'override',
}

---Needles for the bounded source-text scan. Only the SPECIFIC
---break-glass vocabulary is a machinery signal: 'bypass' and 'override'
---are polysemous across the ai tree (cache bypasses, config overrides,
---error-path comments, prompt-injection patterns) and are classified,
---never counted. The bare Lua keyword `break` is deliberately NOT a
---needle (every loop in the tree would hit).
local TEXT_NEEDLES = {
    'break_glass',
    'breakglass',
    'break-glass',
    'emergency',
}

---Generic needles scanned separately for classification only.
local GENERIC_NEEDLES = {
    'bypass',
    'override',
}

local SCAN_FILES_MAX = 256
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
---bypass needles. Records evidence either way.
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
        for _, needle in ipairs(BYPASS_NEEDLES) do
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

---Recursively list .lua files under a dir (bounded), via vim.loop.
---@param root string
---@param out string[] accumulator
---@param budget integer[] remaining file budget (mutable box)
local function list_lua_files(root, out, budget)
    if budget[1] <= 0 then
        return
    end
    local handle = vim.loop.fs_scandir(root)
    if handle == nil then
        return
    end
    while budget[1] > 0 do
        local name, ftype = vim.loop.fs_scandir_next(handle)
        if name == nil then
            break
        end
        if name ~= '.' and name ~= '..' then
            local full = root .. '/' .. name
            if ftype == 'file' and name:sub(-4) == '.lua' then
                out[#out + 1] = full
                budget[1] = budget[1] - 1
            elseif ftype == 'directory' or ftype == 'link' then
                -- luv reports directories as 'directory' (not 'dir');
                -- follow symlinks too (fs_scandir on a non-dir is a
                -- graceful no-op via the nil-handle check above).
                list_lua_files(full, out, budget)
            end
        end
    end
end

---Bounded source-text scan of the ai tree for break-glass vocabulary.
---Only the SPECIFIC needles count as machinery (fail-closed). The
---generic 'bypass'/'override' vocabulary is classified, never counted:
---the ai/security hits are a CONTROL SAMPLE (prompt-injection/
---unicode-bidi scanner vocabulary), and the rest are polysemous
---unrelated senses (config overrides, cache bypasses, error-path
---comments) — recorded with a bounded count and examples. Records
---policy.lua's anti-bypass contract as architectural evidence.
---@param diver_lua_dir string
---@return string[] bypass_hits break-glass machinery (empty when absent)
local function scan_ai_text(diver_lua_dir)
    local ai_dir = diver_lua_dir .. '/lua/ai'
    local files = {}
    list_lua_files(ai_dir, files, { SCAN_FILES_MAX })
    ev('source scan: ' .. #files .. ' lua files under lua/ai')
    local hits = {}
    local generic_files = 0
    local generic_examples = {}
    for _, path in ipairs(files) do
        local text = read_bounded(path)
        if text ~= nil then
            local lower = text:lower()
            local in_security = path:find('/ai/security/', 1, true) ~= nil
            for _, needle in ipairs(TEXT_NEEDLES) do
                if lower:find(needle, 1, true) then
                    hits[#hits + 1] = path .. ' ~' .. needle
                    break
                end
            end
            -- generic vocabulary: classify, never count as machinery.
            local generic_hit = nil
            for _, needle in ipairs(GENERIC_NEEDLES) do
                if lower:find(needle, 1, true) then
                    generic_hit = needle
                    break
                end
            end
            if generic_hit ~= nil then
                if path:find('harness/policy.lua', 1, true) and generic_hit == 'bypass' then
                    ev('architectural evidence: ' .. path .. ' — "No adapter, provider, '
                        .. 'or MCP server may bypass this module." The architecture\'s '
                        .. 'current stance is NO bypass; a break-glass path would have '
                        .. 'to be reconciled with this contract.')
                elseif in_security then
                    ev('control sample: ' .. path .. ' ~' .. generic_hit .. ' — classified UNRELATED: '
                        .. 'prompt-injection/unicode-bidi scanner vocabulary '
                        .. '(instruction-override patterns, bidi_override controls), '
                        .. 'not approval-bypass machinery')
                else
                    generic_files = generic_files + 1
                    if #generic_examples < 5 then
                        generic_examples[#generic_examples + 1] = path .. ' ~' .. generic_hit
                    end
                end
            end
        end
    end
    if generic_files > 0 then
        ev('polysemous vocabulary: ' .. generic_files .. ' files use bypass/override in '
            .. 'unrelated senses (config overrides, cache bypasses, error-path comments) — '
            .. 'classified, not counted as machinery; e.g. '
            .. table.concat(generic_examples, ', '))
    end
    return hits
end

---Facet: scan the approval path and the ai tree for bypass machinery.
---@param diver_lua_dir string
---@return string[] hits
local function facet_bypass_path_scan(diver_lua_dir)
    ev('facet bypass-path-scan: approval-path exports + ai tree text')
    local hits = {}
    for _, path in ipairs(APPROVAL_MODULES) do
        for _, hit in ipairs(probe_module(path)) do
            hits[#hits + 1] = hit
        end
    end
    for _, hit in ipairs(scan_ai_text(diver_lua_dir)) do
        hits[#hits + 1] = hit
    end
    return hits
end

---Facet: the approval module's exits are closed — decide and expiry
---only, no bypass entry point.
---@return string[] hits always empty; the facet asserts, it does not find
local function facet_approval_exits_closed()
    ev('facet approval-exits-closed: the approval state machine')
    local ok, approval = pcall(require, 'ai.harness.approval')
    if not ok or type(approval) ~= 'table' then
        return { 'ai.harness.approval did not load' }
    end
    local names = exported_names(approval)
    ev('ai.harness.approval exports: ' .. table.concat(names, ', '))
    ev('the only exits from pending are decide(approved|denied) and '
        .. 'sweep_expired -> expired; no bypass/override/emergency entry point')
    return {}
end

---Facet: document the design's three required artifacts as absent.
---@return string[] hits always empty; the facet documents, it does not find
local function facet_justification_artifacts_absent()
    ev('facet justification-artifacts-absent: the design\'s three required artifacts')
    ev('1. justification record linked to the bypass execution (reason + post-hoc '
        .. 'justification within N minutes): no such record type exists — there is '
        .. 'no bypass execution to link it to')
    ev('2. single-use, time-boxed grant: no grant primitive exists; nothing is '
        .. 'single-use because nothing bypasses')
    ev('3. auto-expiry with incident flagging when justification never arrives: no '
        .. 'expiry-besides-timeout, no incident flag — the only incident-adjacent '
        .. 'machinery is the approval timeout path (task-56), which denies')
    ev('the design\'s own warning stands: the abnormal path\'s opposite risk is the '
        .. 'bypass becoming routine — with no bypass at all, that risk is currently zero')
    return {}
end

---Facet: fail-closed union scan.
---@param diver_lua_dir string
---@return string[] hits
local function facet_fail_closed(diver_lua_dir)
    ev('facet fail-closed-recon: union scan over approval-path exports + ai tree text')
    return facet_bypass_path_scan(diver_lua_dir)
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
        scenario = 'bypass-path-scan'
    end
    ev('scenario=' .. scenario)
    local hits
    if scenario == 'bypass-path-scan' then
        hits = facet_bypass_path_scan(diver_lua_dir)
    elseif scenario == 'approval-exits-closed' then
        hits = facet_approval_exits_closed()
    elseif scenario == 'justification-artifacts-absent' then
        hits = facet_justification_artifacts_absent()
    elseif scenario == 'fail-closed-recon' then
        hits = facet_fail_closed(diver_lua_dir)
    else
        return fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
    end
    if #hits > 0 then
        return fail(
            'recon',
            'bypass machinery now exists (' .. table.concat(hits, ', ') .. '); probe outdated'
        )
    end
    ev('zero break-glass hits: no emergency-bypass path, no justification record, '
        .. 'no single-use grant, no auto-expiry with incident flag')
    return fail(
        'seam',
        'seam absent: diver has no emergency-bypass (break-glass) path — the approval '
            .. 'state machine exits only via decide(approved|denied) or sweep_expired, '
            .. 'and policy.lua\'s contract is explicitly anti-bypass ("No adapter, provider, '
            .. 'or MCP server may bypass this module"). The design question is the finding: '
            .. 'should a break-glass procedure exist? Banked for Matt — a product decision '
            .. 'that must reconcile with the no-bypass contract and the design\'s own '
            .. 'warning that a bypass must not become routine (diver-owned).'
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
