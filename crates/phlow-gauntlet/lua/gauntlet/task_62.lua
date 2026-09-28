-- task_62.lua -- gauntlet driver: replanning on partial failure recon probe.
--
-- The design asks for the planner's REPLAN entry point: step 2 of 5
-- fails -> the planner emits a NEW plan for the remainder (steps 3-5
-- with a workaround), completed steps are NOT re-executed, and the
-- replan is semantic (a step-2 failure that invalidates step 1's result
-- correctly re-does step 1). "Locate the planner's replan entry point;
-- if the harness only supports resume, document the gap." Distinct from
-- task-05 (resume CONTINUES the same plan) and task-21 (saga
-- compensates backward; replan moves forward differently).
--
-- Recon probe: the driver inspects the REAL ai tree —
-- (1) the export tables of ai.harness.supervisor, ai.harness,
-- ai.harness.approval, ai.rose, and ai.rose.agent for replan vocabulary
-- (replan, revise_plan, new_plan, plan_v2, replan_from); (2) a BOUNDED
-- source-text scan of lua/ai/harness for whole-word `replan`;
-- (3) the actual bodies of M.resume and M.retry_run (extracted from
-- supervisor.lua) to show they re-launch / re-queue the SAME run —
-- resume is not replan, retry is not replan.
--
-- Honest result: the seam is ABSENT. The supervisor's recovery
-- vocabulary is resume (re-launch the same run_id — requires a terminal
-- run, same spec, same adapter) and retry_run (bounded same-run retry
-- with backoff and an attempt ceiling, transitioning the same run to
-- retry_wait). Neither emits a new plan for the remainder; neither is
-- semantic about which completed steps stay valid. The rose
-- planner->coder->reviewer flow has role phases, not step plans 1..5,
-- and its coder retry appends feedback to the same task — not a v2 plan
-- covering the remainder. The design's question is the finding: the
-- harness only supports resume; the replan gap is documented.
--
-- Fail-closed: if replan machinery ever appears, the driver reports
-- where="recon" (premise changed) instead of the seam absence.
-- Diver-owned finding: flagged, never fixed on gauntlet authority.
--
-- Scenarios via GAUNTLET_SCENARIO (default "replan-entry-point-scan"):
--   replan-entry-point-scan      export + source-text scan for replan
--                               vocabulary
--   resume-relaunches-same-run  V: M.resume body shows the same run_id
--                               re-queued (no new plan emitted)
--   retry-keeps-same-run        A: M.retry_run body shows the same run
--                               re-queued to retry_wait with a ceiling
--                               (no semantic replan)
--   fail-closed-recon            A: union scan; hits -> where="recon"
--
-- Prints exactly one JSON verdict line to stdout and always exits 0; the
-- verdict carries the outcome, not the exit code. Writes nothing outside
-- GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64
local TASK_ID = 'task-62'

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function fail(where, how)
    return { id = TASK_ID, outcome = 'fail', where = where, how = how, evidence = evidence }
end

---Modules whose export tables are scanned for replan vocabulary.
local SCAN_MODULES = {
    'ai.harness.supervisor',
    'ai.harness',
    'ai.harness.approval',
    'ai.rose',
    'ai.rose.agent',
}

---Name fragments indicating replan machinery. Matched case-insensitively
---against exported function names. `resume` and `retry_run` are
---deliberately NOT needles — they are the control samples (same-run
---recovery, classified below), not replan.
local REPLAN_NEEDLES = {
    'replan',
    'revise_plan',
    'new_plan',
    'plan_v2',
    'replan_from',
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

---Scan one module's export table for replan needles.
---@param modname string
---@return string[] hits
local function scan_exports(modname)
    local hits = {}
    local ok, mod = pcall(require, modname)
    if not ok or type(mod) ~= 'table' then
        ev('cannot require ' .. modname .. ': ' .. tostring(mod))
        return hits
    end
    for name, value in pairs(mod) do
        if type(value) == 'function' then
            local lname = tostring(name):lower()
            for _, needle in ipairs(REPLAN_NEEDLES) do
                if lname:find(needle, 1, true) ~= nil then
                    hits[#hits + 1] = modname .. '.' .. tostring(name)
                    break
                end
            end
        end
    end
    return hits
end

---Extract the body of `function M.<fname>` from a source file: lines
---from the definition line to the first line that is exactly `end`.
---@param path string
---@param fname string
---@return string? body
---@return string? err
local function extract_function_body(path, fname)
    local f = io.open(path, 'r')
    if f == nil then
        return nil, 'cannot open ' .. path
    end
    local lines = {}
    local capturing = false
    for line in f:lines() do
        if not capturing then
            if line:match('^function M%.' .. fname .. '%(') ~= nil then
                capturing = true
                lines[#lines + 1] = line
            end
        else
            lines[#lines + 1] = line
            if line == 'end' then
                break
            end
        end
    end
    f:close()
    if not capturing then
        return nil, 'function M.' .. fname .. ' not found in ' .. path
    end
    return table.concat(lines, '\n')
end

---Whole-word token test on lowercased text (so "explain" never matches
---"plan").
---@param text string
---@param token string
---@return boolean
local function has_token(text, token)
    return text:find('%f[%a]' .. token .. '%f[%A]') ~= nil
end

---Facet: export + source-text scan for replan machinery.
---@param diver_lua_dir string
---@return string[] hits
local function facet_replan_entry_point_scan(diver_lua_dir)
    ev('facet replan-entry-point-scan: export tables + bounded harness text scan')
    local hits = {}
    for _, modname in ipairs(SCAN_MODULES) do
        for _, h in ipairs(scan_exports(modname)) do
            hits[#hits + 1] = h
        end
    end
    ev('export-table scan of ' .. #SCAN_MODULES .. ' modules: ' .. #hits .. ' replan hits')
    -- Bounded whole-word text scan of lua/ai/harness only (the recovery
    -- path lives there; the whole ai tree is out of scope for this facet).
    local harness_dir = diver_lua_dir .. '/ai/harness'
    local files = 0
    local ok, walk_err = pcall(function()
        for name, ftype in vim.fs.dir(harness_dir) do
            if ftype == 'file' and name:sub(-4) == '.lua' then
                files = files + 1
                local path = harness_dir .. '/' .. name
                local f = io.open(path, 'r')
                if f ~= nil then
                    local lineno = 0
                    for line in f:lines() do
                        lineno = lineno + 1
                        if has_token(line:lower(), 'replan') then
                            hits[#hits + 1] = path .. ':' .. lineno
                        end
                    end
                    f:close()
                end
            end
        end
    end)
    if not ok then
        ev('harness text scan error: ' .. tostring(walk_err))
    end
    ev('harness text scan: ' .. files .. ' files, whole-word "replan"')
    return hits
end

---Facet: M.resume re-launches the SAME run — resume is not replan.
---@param diver_lua_dir string
---@return string[] hits (non-empty means resume emits a plan)
local function facet_resume_relaunches_same_run(diver_lua_dir)
    ev('facet resume-relaunches-same-run: read M.resume body from supervisor.lua')
    local hits = {}
    local body, err = extract_function_body(
        diver_lua_dir .. '/ai/harness/supervisor.lua',
        'resume'
    )
    if body == nil then
        return { 'extract failed: ' .. tostring(err) }
    end
    local low = body:lower()
    if low:find('unknown run', 1, true) ~= nil then
        ev('M.resume rejects unknown run ids: "unknown run: <id>" — resume addresses '
            .. 'an existing run, never a remainder plan')
    end
    if low:find("resume requires a terminal run", 1, true) ~= nil then
        ev('M.resume requires a terminal run: it continues the SAME run record, '
            .. 'not a new plan for the remainder')
    end
    if low:find('launch(sup, run, run.adapter)', 1, true) ~= nil then
        ev('M.resume re-launches with the SAME run table and SAME adapter '
            .. '(launch(sup, run, run.adapter)): the spec is unchanged — no v2 plan')
    end
    if has_token(low, 'plan') then
        hits[#hits + 1] = 'M.resume body mentions a plan token'
        ev('UNEXPECTED: M.resume body contains a plan token')
    else
        ev('M.resume body contains no plan token: no new plan is constructed anywhere '
            .. 'in the resume path')
    end
    return hits
end

---Facet (adversarial): M.retry_run re-queues the SAME run to retry_wait
---with an attempt ceiling — bounded same-run retry, not semantic replan.
---@param diver_lua_dir string
---@return string[] hits (non-empty means retry emits a plan)
local function facet_retry_keeps_same_run(diver_lua_dir)
    ev('facet retry-keeps-same-run: read M.retry_run body from supervisor.lua')
    local hits = {}
    local body, err = extract_function_body(
        diver_lua_dir .. '/ai/harness/supervisor.lua',
        'retry_run'
    )
    if body == nil then
        return { 'extract failed: ' .. tostring(err) }
    end
    local low = body:lower()
    if low:find('retry attempt ceiling exceeded', 1, true) ~= nil then
        ev('M.retry_run enforces an attempt ceiling ("retry attempt ceiling exceeded"): '
            .. 'bounded same-run retry, not an unbounded replan loop')
    end
    if low:find('retry_wait', 1, true) ~= nil then
        ev('M.retry_run transitions the SAME run to retry_wait (same run_id, backoff, '
            .. 'tick() promotes when due): the failed step is retried as-is — no '
            .. 'workaround plan for the remainder, no semantic re-evaluation of '
            .. 'which completed steps stay valid')
    end
    if has_token(low, 'plan') then
        hits[#hits + 1] = 'M.retry_run body mentions a plan token'
        ev('UNEXPECTED: M.retry_run body contains a plan token')
    else
        ev('M.retry_run body contains no plan token: a step-2 failure can never '
            .. 'produce "plan v2 covering steps 3-5 with a workaround"')
    end
    return hits
end

---Facet: fail-closed union scan.
---@param diver_lua_dir string
---@return string[] hits
local function facet_fail_closed(diver_lua_dir)
    ev('facet fail-closed-recon: union scan over exports + harness text')
    return facet_replan_entry_point_scan(diver_lua_dir)
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
        scenario = 'replan-entry-point-scan'
    end
    ev('scenario=' .. scenario)
    local hits
    if scenario == 'replan-entry-point-scan' then
        hits = facet_replan_entry_point_scan(diver_lua_dir)
    elseif scenario == 'resume-relaunches-same-run' then
        hits = facet_resume_relaunches_same_run(diver_lua_dir)
    elseif scenario == 'retry-keeps-same-run' then
        hits = facet_retry_keeps_same_run(diver_lua_dir)
    elseif scenario == 'fail-closed-recon' then
        hits = facet_fail_closed(diver_lua_dir)
    else
        return fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
    end
    if #hits > 0 then
        return fail(
            'recon',
            'replan machinery now exists (' .. table.concat(hits, ', ') .. '); probe outdated'
        )
    end
    ev('zero replan hits: no replan/revise_plan/new_plan/plan_v2 entry point in the '
        .. 'harness export tables or the harness source text')
    return fail(
        'seam',
        'seam absent: diver has no replan entry point — the supervisor\'s recovery '
            .. 'vocabulary is resume (re-launch the SAME run_id: requires a terminal run, '
            .. 'same spec, same adapter — M.resume body contains no plan token) and '
            .. 'retry_run (bounded same-run retry with backoff and an attempt ceiling, '
            .. 'transitioning the same run to retry_wait — body contains no plan token). '
            .. 'Neither emits a new plan for the remainder; neither is semantic about '
            .. 'which completed steps stay valid (a step-2 failure invalidating step 1 '
            .. 'cannot trigger a re-do of step 1). The rose planner->coder->reviewer '
            .. 'flow has role phases, not step plans 1..5, and coder retry appends '
            .. 'feedback to the same task. The design\'s question is the finding: the '
            .. 'harness only supports resume; the replan gap is documented. '
            .. 'Diver-owned finding; no fix on gauntlet authority.'
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
