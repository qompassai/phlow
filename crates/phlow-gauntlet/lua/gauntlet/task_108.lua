-- task-108 driver: cross-harness transfer (diver Phase-2 n/a; SkillOpt).
--
-- Evolve an F-bind skill in the Rust harness (done by task_108.rs, which
-- writes one SKILL.md per variant into GAUNTLET_VARIANTS_DIR), freeze its
-- exact text, and pass it through diver's REAL skill load path under
-- headless Neovim: require('ai.mcp.skills') -> setup({skills_dir}) ->
-- scan() -> get(name) -> tool_def(skill).procedure. The procedure bytes
-- are written back to GAUNTLET_OUT_DIR/<name>.bin; the Rust side asserts
-- byte-exactness and scores both harnesses on the same F-bind fixtures.
--
-- Diver probed: c84352cc850d507df477706b9166b6541ebe9e1c (main).
-- Diver is read-only: this driver never writes outside GAUNTLET_OUT_DIR.
--
-- Scenarios via GAUNTLET_SCENARIO (default "assemble"):
--   assemble      load every SKILL.md through the real path, write back
--                 the procedure bytes + manifest.json (V1/V2/A1 data)
--   invalid-name  a SKILL.md with an invalid skill name must be rejected
--                 LOUDLY: scan() reports the error, get() returns nil (A2)
--
-- Prints exactly one JSON verdict line to stdout and always exits 0.

local DIVER_SHA = 'c84352cc850d507df477706b9166b6541ebe9e1c'
local EVIDENCE_MAX = 64

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function scenario_pass(note)
    ev(note)
    return { id = 'task-108', outcome = 'pass', evidence = evidence }
end

local function driver_fail(where, how)
    return { id = 'task-108', outcome = 'fail', where = where, how = how, evidence = evidence }
end

---Resolve DIVER_LUA_DIR to the runtimepath root. Accepts either the rtp
---root itself (holding lua/ai/mcp/skills.lua) or the lua/ dir directly.
---@param diver_lua_dir string
---@return string|nil rtp_root
local function resolve_diver_dirs(diver_lua_dir)
    local function is_file(path)
        local fh = io.open(path, 'r')
        if fh == nil then
            return false
        end
        fh:close()
        return true
    end
    if is_file(diver_lua_dir .. '/lua/ai/mcp/skills.lua') then
        return diver_lua_dir
    end
    if is_file(diver_lua_dir .. '/ai/mcp/skills.lua') then
        local parent = diver_lua_dir:gsub('/+$', ''):gsub('/[^/]+$', '')
        if parent == '' then
            parent = '/'
        end
        return parent
    end
    return nil
end

---Bounded binary write. Returns (true) or (nil, err).
---@param path string
---@param data string
---@return boolean|nil ok
---@return string|nil err
local function write_file(path, data)
    local fh, open_err = io.open(path, 'wb')
    if fh == nil then
        return nil, 'cannot open ' .. path .. ' for writing: ' .. tostring(open_err)
    end
    fh:write(data)
    fh:close()
    return true, nil
end

---Read a text file's lines (no trailing-newline artifact).
---@param path string
---@return string[]|nil lines
local function read_lines(path)
    local fh = io.open(path, 'r')
    if fh == nil then
        return nil
    end
    local lines = {}
    for line in fh:lines() do
        if line ~= '' then
            lines[#lines + 1] = line
        end
    end
    fh:close()
    return lines
end

---Common bootstrap: env validation, rtp wiring, real skills module.
---@return table|nil handles
---@return string|nil err
local function bootstrap()
    local diver_lua_dir = vim.env.DIVER_LUA_DIR
    local variants_dir = vim.env.GAUNTLET_VARIANTS_DIR
    local out_dir = vim.env.GAUNTLET_OUT_DIR
    if type(diver_lua_dir) ~= 'string' or diver_lua_dir == '' then
        return nil, 'DIVER_LUA_DIR is not set'
    end
    if type(variants_dir) ~= 'string' or variants_dir == '' then
        return nil, 'GAUNTLET_VARIANTS_DIR is not set'
    end
    if type(out_dir) ~= 'string' or out_dir == '' then
        return nil, 'GAUNTLET_OUT_DIR is not set'
    end
    local rtp_root = resolve_diver_dirs(diver_lua_dir)
    if rtp_root == nil then
        return nil, 'DIVER_LUA_DIR has no ai/mcp/skills.lua under <dir>/lua or <dir>'
    end
    vim.opt.runtimepath:append(rtp_root)
    ev('diver sha ' .. DIVER_SHA .. ' rtp ' .. rtp_root)
    local ok, skills = pcall(require, 'ai.mcp.skills')
    if not ok then
        return nil, "require('ai.mcp.skills') failed: " .. tostring(skills)
    end
    -- The skills module requires ai.security.annotations at tool_def
    -- time; fail fast here so a missing module is a driver error, not a
    -- per-variant mystery.
    local ann_ok, ann_mod = pcall(require, 'ai.security.annotations')
    if not ann_ok then
        return nil, "require('ai.security.annotations') failed: " .. tostring(ann_mod)
    end
    assert(ann_mod ~= nil)
    vim.fn.mkdir(out_dir, 'p')
    return { skills = skills, variants_dir = variants_dir, out_dir = out_dir }, nil
end

---Scenario: assemble every variant through the real load path and write
---the procedure bytes back for the Rust side to compare and score.
---@param h table bootstrap handles
---@return table verdict
local function scenario_assemble(h)
    local skills = h.skills
    -- Diver's setup() returns nothing on success (it only configures);
    -- a Lua error is the failure signal, so pcall it.
    local sok, serr = pcall(skills.setup, { skills_dir = h.variants_dir })
    if not sok then
        return driver_fail('skills-setup', 'setup raised: ' .. tostring(serr))
    end
    -- scan() returns (loaded_count, errors); the variant names come
    -- from the names.txt the Rust side wrote next to the skill dirs.
    local loaded, errs = skills.scan()
    if #errs > 0 then
        return driver_fail('skills-scan', 'scan errors: ' .. table.concat(errs, ' | '))
    end
    if loaded == 0 then
        return driver_fail('skills-scan', 'no skills found in ' .. h.variants_dir)
    end
    local names = read_lines(h.variants_dir .. '/names.txt')
    if names == nil or #names == 0 then
        return driver_fail('names', 'names.txt missing or empty in ' .. h.variants_dir)
    end
    if #names ~= loaded then
        return driver_fail(
            'skills-scan',
            'scan loaded ' .. loaded .. ' skills but names.txt lists ' .. #names
        )
    end
    local manifest = {}
    for _, name in ipairs(names) do
        local got = skills.get(name)
        if got == nil then
            return driver_fail('skills-get', 'get(' .. name .. ') returned nil after scan')
        end
        local def = skills.tool_def(got)
        if type(def.procedure) ~= 'string' then
            return driver_fail('tool-def', 'procedure is not a string for ' .. name)
        end
        local out_path = h.out_dir .. '/' .. name .. '.bin'
        local wok, werr = write_file(out_path, def.procedure)
        if not wok then
            return driver_fail('write-out', werr)
        end
        manifest[#manifest + 1] = { name = name, bytes = #def.procedure }
        ev('assembled ' .. name .. ' ' .. #def.procedure .. ' bytes -> ' .. out_path)
    end
    local mok, merr = write_file(h.out_dir .. '/manifest.json', vim.json.encode(manifest))
    if not mok then
        return driver_fail('write-manifest', merr)
    end
    return scenario_pass('assembled ' .. #manifest .. ' variants through ai.mcp.skills')
end

---Scenario: an invalid skill name must fail LOUDLY — scan() reports the
---error and get() returns nil. Silent acceptance would be the finding.
---@param h table bootstrap handles
---@return table verdict
local function scenario_invalid_name(h)
    local skills = h.skills
    local sok, serr = pcall(skills.setup, { skills_dir = h.variants_dir })
    if not sok then
        return driver_fail('skills-setup', 'setup raised: ' .. tostring(serr))
    end
    local loaded, errs = skills.scan()
    local got = skills.get('Bad_Name')
    ev('scan loaded ' .. loaded .. ' skills, ' .. #errs .. ' errors')
    for _, e in ipairs(errs) do
        ev('scan error: ' .. e)
    end
    ev("get('Bad_Name') -> " .. tostring(got))
    if got ~= nil then
        return driver_fail(
            'invalid-name-accepted',
            'a SKILL.md with an invalid name was loaded silently'
        )
    end
    if #errs == 0 then
        return driver_fail(
            'invalid-name-silent',
            'invalid skill rejected but scan() reported no error'
        )
    end
    return scenario_pass('invalid skill name rejected loudly: scan error + get() nil')
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'assemble'
    end
    local h, err = bootstrap()
    if h == nil then
        print(vim.json.encode(driver_fail('bootstrap', err)))
        return
    end
    local verdict
    if scenario == 'assemble' then
        verdict = scenario_assemble(h)
    elseif scenario == 'invalid-name' then
        verdict = scenario_invalid_name(h)
    else
        verdict = driver_fail('scenario', "unknown GAUNTLET_SCENARIO '" .. scenario .. "'")
    end
    print(vim.json.encode(verdict))
end

main()
