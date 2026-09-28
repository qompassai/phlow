-- task_85.lua -- gauntlet driver: provider usage accounting integrity.
--
-- The design asks for the harness per-run cost ledger fed by provider
-- usage reports: provider-reported usage is untrusted input, so null,
-- missing, or absurd values must not corrupt the ledger. Well-formed
-- usage is exact; missing usage is recorded as an estimate with a
-- documented method and flagged estimated:true; absurd usage is
-- capped at a named sanity bound and flagged; corrections keep the
-- max observed with an append-only audit note.
--
-- Seam mapping (verified, not invented): diver's `ai.harness.budget`
-- (`lua/ai/harness/budget.lua`) is the per-run budget ledger, but it
-- has no usage-ingestion layer at all. `M.new(limits)` builds the
-- ledger, `M.check`/`M.consume(budget, kind, amount)` add numbers at
-- face value (check only rejects negative amounts and unknown
-- kinds), and `M.snapshot()` returns bare numbers. There is no
-- estimated/measured flag, no sanity cap, no correction API, no
-- audit trail — the metrology-integrity layer the design asks for
-- has no implementation.
--
-- This driver plays the REAL budget module:
--   wellformed  V: new({token=1000}), consume(100,'token'),
--                  snapshot shows used.token == 100 exactly.
--   estimated   V: documents the absent estimated/measured
--                  distinction — consume's real declaration is
--                  resolved via debug.getinfo('S') and read from
--                  the actual budget.lua source: (budget, kind,
--                  amount), no flag parameter; snapshot shows bare
--                  numbers.
--   absurd      A: consume(b, 'token', 1e12) succeeds silently —
--                  check() only requires amount >= 0; no cap, no flag.
--   shrinking   A: enumerates the module's functions (new, check,
--                  consume, remaining, exhausted, snapshot) — no
--                  correction/audit API; the only downward path is
--                  raw table mutation, invisible to any audit.
--
-- All scenarios write machine-readable traces into GAUNTLET_WORK_DIR
-- (usage-trace.json) for the Rust harness probes, print exactly one
-- JSON verdict line to stdout, always exit 0, write nothing outside
-- GAUNTLET_WORK_DIR, and never modify the diver repo.
--
-- Scenarios via GAUNTLET_SCENARIO (default "wellformed"):
--   wellformed   V: well-formed usage is exact.
--   estimated    V: estimated/measured distinction is absent.
--   absurd       A: absurd usage is absorbed silently.
--   shrinking    A: corrections have no audit path.

local EVIDENCE_MAX = 64
local TASK_ID = 'task-85'
local SOURCE_BYTES_MAX = 524288

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function fail(where, how)
    return { id = TASK_ID, outcome = 'fail', where = where, how = how, evidence = evidence }
end

local function pass()
    return { id = TASK_ID, outcome = 'pass', evidence = evidence }
end

---@return table? handles
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
    local ok, budget = pcall(require, 'ai.harness.budget')
    if not ok then
        return nil, 'require ai.harness.budget failed: ' .. tostring(budget)
    end
    return { budget = budget, work_dir = work_dir }
end

---Write `text` to a file inside GAUNTLET_WORK_DIR.
---@param handles table
---@param name string
---@param text string
---@return string? err
local function write_work_file(handles, name, text)
    local path = handles.work_dir .. '/' .. name
    local f, err = io.open(path, 'w')
    if f == nil then
        return 'cannot open ' .. path .. ': ' .. tostring(err)
    end
    f:write(text)
    f:close()
    return nil
end

---Sorted list of the module's function names.
---@param mod table
---@return string[] names
local function module_functions(mod)
    local names = {}
    for k, v in pairs(mod) do
        if type(v) == 'function' then
            names[#names + 1] = k
        end
    end
    table.sort(names)
    return names
end

---Read `path` as text, bounded at `max_bytes`.
---@param path string
---@param max_bytes integer
---@return string? text
---@return string? err
local function read_bounded_file(path, max_bytes)
    local f, ferr = io.open(path, 'r')
    if f == nil then
        return nil, tostring(ferr)
    end
    local text = f:read('a')
    f:close()
    if text == nil then
        return nil, 'empty read'
    end
    if #text > max_bytes then
        return nil, 'file exceeds bound ' .. tostring(max_bytes)
    end
    return text, nil
end

---Scenario "wellformed": well-formed provider usage is recorded
---exactly.
---@param handles table
---@return table verdict
local function scenario_wellformed(handles)
    local budget = handles.budget
    local b, berr = budget.new({ token = 1000, cost = 10 })
    if b == nil then
        return fail('wellformed', 'budget.new failed: ' .. tostring(berr))
    end
    local ok, cerr = budget.consume(b, 'token', 100)
    if not ok then
        return fail('wellformed', 'consume failed: ' .. tostring(cerr))
    end
    local snap = budget.snapshot(b)
    ev('consumed 100 token; snapshot.used.token = ' .. tostring(snap.used.token))
    if snap.used.token ~= 100 then
        return fail('wellformed', 'used.token is not 100: ' .. tostring(snap.used.token))
    end
    local werr = write_work_file(
        handles,
        'usage-trace.json',
        vim.json.encode({
            scenario = 'wellformed',
            consumed = 100,
            snapshot = snap,
        })
    )
    if werr ~= nil then
        return fail('trace', werr)
    end
    ev('wrote usage-trace.json')
    return pass()
end

---Scenario "estimated": the estimated/measured distinction does not
---exist. `consume`'s real declaration is resolved through
---debug.getinfo('S') — never a hardcoded path — and read from the
---actual budget.lua source: (budget, kind, amount), no flag
---parameter. The snapshot carries bare numbers.
---@param handles table
---@return table verdict
local function scenario_estimated(handles)
    local budget = handles.budget
    local info = debug.getinfo(budget.consume, 'S')
    if info == nil or type(info.source) ~= 'string' or info.source:sub(1, 1) ~= '@' then
        return fail('estimated', 'cannot resolve budget.consume source via debug.getinfo')
    end
    local src_path = info.source:sub(2)
    local sig, sread_err = read_bounded_file(src_path, SOURCE_BYTES_MAX)
    if sig == nil then
        return fail('estimated', 'cannot read ' .. src_path .. ': ' .. tostring(sread_err))
    end
    -- Find consume's declaration at or after its linedefined and
    -- extract the parameter list from the real source text.
    local decl_line_no = 0
    local params = nil
    local ln = 0
    for line in (sig .. '\n'):gmatch('([^\n]*)\n') do
        ln = ln + 1
        if ln >= (info.linedefined or 0) and params == nil then
            local pl = line:match('^%s*function%s+[%w_%.:]+%.consume%s*%(([^)]*)%)')
            if pl ~= nil then
                decl_line_no = ln
                params = pl
            end
        end
    end
    if params == nil then
        return fail('estimated', 'no consume declaration found in ' .. src_path)
    end
    local count = 0
    local has_flag_param = false
    for p in params:gmatch('[^,%s]+') do
        count = count + 1
        if p:find('estimat', 1, true) ~= nil or p:find('flag', 1, true) ~= nil then
            has_flag_param = true
        end
    end
    ev('budget.consume declared at ' .. src_path .. ':' .. tostring(decl_line_no)
        .. ': (' .. params:gsub('%s+', ' ') .. ') — params=' .. tostring(count)
        .. ', flag param=' .. tostring(has_flag_param))
    if count ~= 3 then
        return fail('estimated', 'consume does not take 3 params: ' .. tostring(count))
    end
    if has_flag_param then
        return fail('estimated', 'unexpected flag parameter in consume signature')
    end
    local b, berr = budget.new({ token = 1000 })
    if b == nil then
        return fail('estimated', 'budget.new failed: ' .. tostring(berr))
    end
    -- A provider that reports no usage: the only recording path is a
    -- bare number. There is no "estimated" flag to set.
    local ok, cerr = budget.consume(b, 'token', 42)
    if not ok then
        return fail('estimated', 'consume failed: ' .. tostring(cerr))
    end
    local snap = budget.snapshot(b)
    local has_estimated_flag = snap.estimated ~= nil
        or (snap.used and snap.used.estimated ~= nil)
    ev('snapshot has estimated flag: ' .. tostring(has_estimated_flag))
    if has_estimated_flag then
        return fail('estimated', 'unexpected estimated flag in snapshot')
    end
    local werr = write_work_file(
        handles,
        'usage-trace.json',
        vim.json.encode({
            scenario = 'estimated',
            consume_declared_at = src_path .. ':' .. tostring(decl_line_no),
            consume_params = params,
            consume_param_count = count,
            estimated_flag_present = has_estimated_flag,
            snapshot = snap,
        })
    )
    if werr ~= nil then
        return fail('trace', werr)
    end
    ev('wrote usage-trace.json')
    return pass()
end

---Scenario "absurd": an absurd provider report (1e12 tokens for
---"hi") is absorbed silently — check() only requires a
---non-negative number; there is no sanity cap and no flag.
---@param handles table
---@return table verdict
local function scenario_absurd(handles)
    local budget = handles.budget
    local b, berr = budget.new({ token = 1e15 })
    if b == nil then
        return fail('absurd', 'budget.new failed: ' .. tostring(berr))
    end
    local ok, cerr = budget.consume(b, 'token', 1e12)
    ev('consume(b, token, 1e12) -> ok=' .. tostring(ok) .. ' err=' .. tostring(cerr))
    if not ok then
        return fail('absurd', 'absurd consume was rejected: ' .. tostring(cerr))
    end
    local snap = budget.snapshot(b)
    ev('snapshot.used.token = ' .. tostring(snap.used.token))
    if snap.used.token ~= 1e12 then
        return fail('absurd', 'used.token is not 1e12: ' .. tostring(snap.used.token))
    end
    local werr = write_work_file(
        handles,
        'usage-trace.json',
        vim.json.encode({
            scenario = 'absurd',
            consumed = 1e12,
            flagged = false,
            capped = false,
            snapshot = snap,
        })
    )
    if werr ~= nil then
        return fail('trace', werr)
    end
    ev('wrote usage-trace.json')
    return pass()
end

---Scenario "shrinking": corrections have no audit path. The module
---exposes exactly new/check/consume/remaining/exhausted/snapshot —
---no correction, no audit, no max-observed. The only way "down" is
---raw table mutation, invisible to any ledger history.
---@param handles table
---@return table verdict
local function scenario_shrinking(handles)
    local budget = handles.budget
    local names = module_functions(budget)
    ev('ai.harness.budget functions: ' .. table.concat(names, ', '))
    for _, banned in ipairs({ 'correct', 'audit', 'revise', 'adjust' }) do
        for _, name in ipairs(names) do
            if name:find(banned, 1, true) ~= nil then
                return fail('shrinking', 'unexpected correction/audit function: ' .. name)
            end
        end
    end
    local b, berr = budget.new({ token = 1000 })
    if b == nil then
        return fail('shrinking', 'budget.new failed: ' .. tostring(berr))
    end
    local ok, cerr = budget.consume(b, 'token', 500)
    if not ok then
        return fail('shrinking', 'consume failed: ' .. tostring(cerr))
    end
    -- A provider correction ("actually 300, not 500"): the ledger has
    -- no correction entry point. The only downward path is raw
    -- mutation of the published table — no audit note, no max-kept.
    b.used.token = 300
    local snap = budget.snapshot(b)
    ev('after raw mutation b.used.token=300: snapshot.used.token = ' .. tostring(snap.used.token))
    local werr = write_work_file(
        handles,
        'usage-trace.json',
        vim.json.encode({
            scenario = 'shrinking',
            functions = names,
            correction_api = false,
            audit_trail = false,
            after_correction = snap,
        })
    )
    if werr ~= nil then
        return fail('trace', werr)
    end
    ev('wrote usage-trace.json')
    return pass()
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'wellformed'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return fail('bootstrap', boot_err)
    end
    ev('scenario=' .. scenario)
    if scenario == 'wellformed' then
        return scenario_wellformed(handles)
    elseif scenario == 'estimated' then
        return scenario_estimated(handles)
    elseif scenario == 'absurd' then
        return scenario_absurd(handles)
    elseif scenario == 'shrinking' then
        return scenario_shrinking(handles)
    end
    return fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
end

local ok, verdict = pcall(main)
if not ok then
    ev('lua error: ' .. tostring(verdict))
    verdict = fail('lua-driver', 'unhandled error: ' .. tostring(verdict))
end
-- verdict on the real stdout: in `nvim --headless -l`, Lua print() goes
-- to stderr, but the Rust runner parses the verdict from stdout.
io.stdout:write(vim.json.encode(verdict) .. '\n')
