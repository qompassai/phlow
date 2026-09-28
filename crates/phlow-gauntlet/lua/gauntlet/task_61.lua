-- task_61.lua -- gauntlet driver: plan schema validation recon probe.
--
-- The design asks for the planner -> executor handoff seam: the planner's
-- output validated against a schema BEFORE anything executes (malformed
-- plan — missing step, unknown tool, cyclic dependency — rejected with
-- the schema violation named, nothing executes). Structural pass
-- criterion: the executor's input type IS the validated plan. "Locate the
-- plan representation; the harness `workflow?` spec is a candidate."
--
-- Recon probe: the driver inspects the REAL ai tree —
-- (1) the export tables of ai.harness.supervisor, ai.harness,
-- ai.harness.registry, ai.rose, and ai.rose.agent for plan-validation
-- vocabulary (validate_plan, plan_schema, plan_validator,
-- schema_violation); (2) a BOUNDED source-text scan of lua/ai for the
-- same vocabulary; (3) the closest "plan" candidates — the rose planner
-- (ai.rose M.plan) and the harness registry's workflow defs — to show
-- they carry no machine plan representation.
--
-- Honest result: the seam is ABSENT. ai.rose's M.plan produces plan TEXT
-- for humans ("Output ONLY the plan as plain text", lua/ai/rose/init.lua)
-- — there is no machine plan type, and hence no plan schema to validate
-- against and no executor input type that IS a validated plan. The
-- registry's register_workflow accepts arbitrary def shapes (only
-- `adapter` is validated), so a malformed "plan" cannot be rejected with
-- a schema violation named — there is no schema. The design's structural
-- pass criterion is unmeetable: there is no plan type at all.
--
-- Fail-closed: if plan-validation machinery ever appears, the driver
-- reports where="recon" (premise changed) instead of the seam absence.
-- Diver-owned finding: flagged, never fixed on gauntlet authority.
--
-- Scenarios via GAUNTLET_SCENARIO (default "plan-schema-scan"):
--   plan-schema-scan              export + source-text scan for
--                                 plan-validation vocabulary
--   workflow-def-has-no-plan-schema  behavioral: register a workflow
--                                 with a malformed plan shape —
--                                 accepted; only `adapter` validated
--   malformed-plan-cannot-be-rejected  adversarial: a mock malformed
--                                 plan (missing step, unknown tool,
--                                 cyclic deps) has no validator to be
--                                 fed to — M.validate_plan is nil
--   fail-closed-recon             union scan; hits -> where="recon"
--
-- Prints exactly one JSON verdict line to stdout and always exits 0; the
-- verdict carries the outcome, not the exit code. Writes nothing outside
-- GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64
local TASK_ID = 'task-61'

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function fail(where, how)
    return { id = TASK_ID, outcome = 'fail', where = where, how = how, evidence = evidence }
end

---Modules whose export tables are scanned for plan-validation vocabulary.
local SCAN_MODULES = {
    'ai.harness.supervisor',
    'ai.harness',
    'ai.harness.registry',
    'ai.harness.adapter',
    'ai.rose',
    'ai.rose.agent',
}

---Name fragments indicating plan-schema-validation machinery. Matched
---case-insensitively against exported function names. The rose
---planner-role entry point (`M.plan`, plan TEXT for humans) is
---deliberately NOT a needle — it is a control sample, classified below.
local PLAN_NEEDLES = {
    'validate_plan',
    'plan_schema',
    'plan_validator',
    'schema_violation',
    'plan_rule',
}

---Needles for the bounded source-text scan of lua/ai.
local TEXT_NEEDLES = {
    'validate_plan',
    'plan_schema',
    'plan_validator',
    'schema_violation',
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

---Scan one module's export table for plan-validation needles.
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
            for _, needle in ipairs(PLAN_NEEDLES) do
                if lname:find(needle, 1, true) ~= nil then
                    hits[#hits + 1] = modname .. '.' .. tostring(name)
                    break
                end
            end
        end
    end
    return hits
end

---Bounded recursive text scan of a directory for whole-word needles.
---@param dir string
---@param needles string[]
---@return string[] hits "path:line"
local function scan_tree(dir, needles)
    local hits = {}
    local files = 0
    local function walk(d)
        if files >= SCAN_FILES_MAX then
            return
        end
        for name, ftype in vim.fs.dir(d) do
            if #hits >= EVIDENCE_MAX then
                return
            end
            local path = d .. '/' .. name
            if ftype == 'directory' then
                walk(path)
            elseif ftype == 'file' and name:sub(-4) == '.lua' then
                files = files + 1
                if files > SCAN_FILES_MAX then
                    return
                end
                local f = io.open(path, 'r')
                if f ~= nil then
                    local lineno = 0
                    for line in f:lines() do
                        lineno = lineno + 1
                        local ll = line:lower()
                        for _, needle in ipairs(needles) do
                            -- whole-word: "planner" must not match "plan"
                            if ll:find('%f[%a]' .. needle .. '%f[%A]') ~= nil then
                                hits[#hits + 1] = path .. ':' .. lineno
                                break
                            end
                        end
                        if #hits >= EVIDENCE_MAX then
                            break
                        end
                    end
                    f:close()
                end
            end
        end
    end
    local ok, err = pcall(walk, dir)
    if not ok then
        ev('tree walk error: ' .. tostring(err))
    end
    ev('text scan: ' .. files .. ' lua files under ' .. dir)
    return hits
end

---Facet: export + text scan for plan-validation machinery.
---@param diver_lua_dir string
---@return string[] hits
local function facet_plan_schema_scan(diver_lua_dir)
    ev('facet plan-schema-scan: export tables + bounded ai-tree text scan')
    local hits = {}
    for _, modname in ipairs(SCAN_MODULES) do
        for _, h in ipairs(scan_exports(modname)) do
            hits[#hits + 1] = h
        end
    end
    ev('export-table scan of ' .. #SCAN_MODULES .. ' modules: ' .. #hits .. ' machinery hits')
    -- The rose planner-role control sample: M.plan exists but produces
    -- plan TEXT for humans, not a machine plan — classified UNRELATED.
    local rok, rose = pcall(require, 'ai.rose')
    if rok and type(rose) == 'table' and type(rose.plan) == 'function' then
        ev('control sample: ai.rose.plan exists but is the planner ROLE — '
            .. '"Output ONLY the plan as plain text" (lua/ai/rose/init.lua); '
            .. 'plan text for humans, not a machine plan type; classified UNRELATED')
    end
    for _, h in ipairs(scan_tree(diver_lua_dir .. '/ai', TEXT_NEEDLES)) do
        hits[#hits + 1] = h
    end
    return hits
end

---Facet: the closest "plan" object (registry workflow def) accepts an
---arbitrary shape — only `adapter` is validated, so there is no
---step/tool/dependency schema to reject a malformed plan against.
---@return string[] hits (non-empty means a schema appeared)
local function facet_workflow_def_has_no_plan_schema()
    ev('facet workflow-def-has-no-plan-schema: register_workflow with a malformed plan shape')
    local hits = {}
    local ok, registry = pcall(require, 'ai.harness.registry')
    if not ok or type(registry) ~= 'table' then
        return { 'cannot require ai.harness.registry: ' .. tostring(registry) }
    end
    local reg = registry.new()
    -- A "plan" with a missing step, an unknown tool, and a cyclic
    -- dependency, expressed as a workflow def. If any plan schema
    -- existed, this shape would be rejected with the rule named.
    local malformed = {
        adapter = 'acp',
        steps = { { id = 1 }, { id = 3 } }, -- step 2 missing
        tools = { 'nonexistent_tool' }, -- unknown tool
        deps = { [1] = 3, [3] = 1 }, -- cyclic dependency
        garbage_field = true,
    }
    local reg_ok, reg_err = registry.register_workflow(reg, 'malformed_plan', malformed)
    if reg_ok then
        ev('register_workflow ACCEPTED a malformed plan shape (missing step, unknown tool, '
            .. 'cyclic deps, garbage field): no step/tool/dependency schema exists')
    else
        hits[#hits + 1] = 'register_workflow rejected the shape: ' .. tostring(reg_err)
        ev('unexpected: register_workflow rejected the malformed shape: ' .. tostring(reg_err))
    end
    local stored = registry.get_workflow(reg, 'malformed_plan')
    if type(stored) == 'table' and stored.garbage_field == true then
        ev('the malformed def round-trips verbatim (garbage_field preserved): '
            .. 'the registry stores defs, it does not validate plans')
    end
    return hits
end

---Facet (adversarial): a mock malformed plan has no validator to be fed
---to — every plausible validator entry point is nil.
---@return string[] hits (non-empty means a validator appeared)
local function facet_malformed_plan_cannot_be_rejected()
    ev('facet malformed-plan-cannot-be-rejected: probe every validator entry point')
    local hits = {}
    local probes = {
        { 'ai.harness.supervisor', 'validate_plan' },
        { 'ai.harness', 'validate_plan' },
        { 'ai.harness.registry', 'validate_plan' },
        { 'ai.harness.registry', 'validate_workflow_plan' },
        { 'ai.rose', 'validate_plan' },
    }
    for _, p in ipairs(probes) do
        local ok, mod = pcall(require, p[1])
        if ok and type(mod) == 'table' and type(mod[p[2]]) == 'function' then
            hits[#hits + 1] = p[1] .. '.' .. p[2]
        end
    end
    ev('validator entry points probed: ' .. #probes .. '; present: ' .. #hits)
    ev('a malformed plan (missing step, unknown tool, cyclic deps) therefore cannot be '
        .. 'rejected "with the schema violation named" — there is no schema and no validator')
    return hits
end

---Facet: fail-closed union scan.
---@param diver_lua_dir string
---@return string[] hits
local function facet_fail_closed(diver_lua_dir)
    ev('facet fail-closed-recon: union scan over exports + ai tree text')
    return facet_plan_schema_scan(diver_lua_dir)
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
        scenario = 'plan-schema-scan'
    end
    ev('scenario=' .. scenario)
    local hits
    if scenario == 'plan-schema-scan' then
        hits = facet_plan_schema_scan(diver_lua_dir)
    elseif scenario == 'workflow-def-has-no-plan-schema' then
        hits = facet_workflow_def_has_no_plan_schema()
    elseif scenario == 'malformed-plan-cannot-be-rejected' then
        hits = facet_malformed_plan_cannot_be_rejected()
    elseif scenario == 'fail-closed-recon' then
        hits = facet_fail_closed(diver_lua_dir)
    else
        return fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
    end
    if #hits > 0 then
        return fail(
            'recon',
            'plan-validation machinery now exists (' .. table.concat(hits, ', ') .. '); probe outdated'
        )
    end
    ev('no machine plan type: diver has no plan type, no validated-plan handoff, and no '
        .. 'planner->executor seam — ai.rose M.plan emits plan text for humans, the harness '
        .. 'executes runs not plans')
    ev('zero plan-schema hits: no validate_plan/plan_schema/plan_validator/schema_violation '
        .. 'in harness export tables or the ai tree text')
    return fail(
        'seam',
        'seam absent: diver has no machine plan representation and no plan-schema '
            .. 'validator — ai.rose M.plan emits plan TEXT for humans ("Output ONLY the plan '
            .. 'as plain text", lua/ai/rose/init.lua), the harness executes runs not plans, '
            .. 'and registry workflow defs accept arbitrary shapes (only `adapter` is '
            .. 'validated). The design\'s structural pass criterion — "the executor\'s input '
            .. 'type IS the validated plan" — is unmeetable: there is no plan type. '
            .. 'A malformed plan (missing step, unknown tool, cyclic dependency) cannot be '
            .. 'rejected with the schema violation named because there is no schema. '
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
