-- task_80.lua -- gauntlet driver: local model selection tradeoffs.
--
-- The design asks for the local model selector: per tool, choose among
-- local models on latency, memory, and quality tradeoffs, with model
-- cards and measured benchmarks as inputs, trading quality for speed
-- with explicit rationale.
--
-- Seam mapping (verified, not invented): diver's
-- `ai.harness.adapter.negotiate(adapters, needs)` selects the FIRST
-- adapter in sorted name order whose probed boolean capabilities
-- satisfy every requested need. The negotiated vocabulary is exactly
-- the seven boolean CAPABILITY_KEYS in ai.harness.types
-- (streaming, cancellation, resume, permissions, artifacts, remote,
-- tools); `max_input_bytes` is an optional integer the negotiation
-- never reads. There is no model registry, no model card, no latency
-- / memory / quality signal, no benchmark, no rationale — the
-- function returns the adapter table, full stop. The design's model
-- selector has no implementation; the adapter selector is real but
-- solves a different problem (boolean capability coverage, not
-- model tradeoffs).
--
-- This driver plays the harness against the REAL adapter module:
--   selection  V: three adapters with distinct boolean capability
--                  profiles (plus driver-side quality notes the module
--                  never reads); five negotiate() probes. Every probe
--                  must select the first name in sorted order among
--                  the satisfying adapters — the strategy is real,
--                  mechanical, and documented in the module's own
--                  docstring.
--   modelscan  V: resolve the loaded adapter.lua path via
--                  debug.getinfo (never a hardcoded path), read it
--                  plus sibling types.lua, and scan for model-selection
--                  tradeoff vocabulary. Emits the raw hit lists.
--
-- Both scenarios write machine-readable traces into GAUNTLET_WORK_DIR
-- (selection-trace.json, modelscan.json) for the Rust harness probes
-- (capability_needs_boolean_only, no_tradeoff_record), print exactly
-- one JSON verdict line to stdout, always exit 0, write nothing
-- outside GAUNTLET_WORK_DIR, and never modify the diver repo.
--
-- Scenarios via GAUNTLET_SCENARIO (default "selection"):
--   selection    V: first-sorted-wins over boolean capabilities.
--   modelscan    V: tradeoff-vocabulary scan of the real module source.

local EVIDENCE_MAX = 64
local READ_MAX = 1048576
local TASK_ID = 'task-80'

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
    local ok, adapter = pcall(require, 'ai.harness.adapter')
    if not ok then
        return nil, 'require ai.harness.adapter failed: ' .. tostring(adapter)
    end
    local ok2, types = pcall(require, 'ai.harness.types')
    if not ok2 then
        return nil, 'require ai.harness.types failed: ' .. tostring(types)
    end
    return { adapter = adapter, types = types, work_dir = work_dir }
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

---@param name string
---@param caps table<string, boolean>
---@param note string driver-side quality note; negotiate never reads it
---@return table adapter
local function make_adapter(name, caps, note)
    return {
        name = name,
        _quality_note = note,
        probe = function()
            return caps
        end,
        start = function(_run, _sink)
            return nil, 'not started in probe'
        end,
        send_input = function(_handle, _input)
            return nil, 'no input in probe'
        end,
        cancel = function(_handle, _reason)
            return true
        end,
        close = function(_handle) end,
    }
end

---@param keys string[]
---@param value boolean
---@return table<string, boolean>
local function caps_all(keys, value)
    local caps = {}
    for _, key in ipairs(keys) do
        caps[key] = value
    end
    return caps
end

---Sorted adapter names of a name->adapter table.
---@param adapters table<string, table>
---@return string[]
local function sorted_names(adapters)
    local names = {}
    for name in pairs(adapters) do
        names[#names + 1] = name
    end
    table.sort(names)
    return names
end

---Names (sorted) whose probed capabilities satisfy every boolean need,
---computed through the module's own M.probe — the same predicate
---negotiate uses.
---@param handles table
---@param adapters table<string, table>
---@param needs table<string, boolean>
---@return string[] satisfying
local function satisfying_names(handles, adapters, needs)
    local names = sorted_names(adapters)
    local out = {}
    for _, name in ipairs(names) do
        local caps = handles.adapter.probe(adapters[name])
        if caps ~= nil then
            local ok = true
            for _, key in ipairs(handles.types.CAPABILITY_KEYS) do
                if needs[key] == true and caps[key] ~= true then
                    ok = false
                    break
                end
            end
            if ok then
                out[#out + 1] = name
            end
        end
    end
    return out
end

---Scenario "selection": validate the real selection strategy.
---@param handles table
---@return table verdict
local function scenario_selection(handles)
    local keys = handles.types.CAPABILITY_KEYS
    local adapters = {
        ['alpha-fast'] = make_adapter(
            'alpha-fast',
            caps_all(keys, true),
            'driver metadata: low latency, high quality (invisible to negotiate)'
        ),
        ['zeta-slow'] = make_adapter(
            'zeta-slow',
            caps_all(keys, true),
            'driver metadata: HIGHER quality, higher latency (invisible to negotiate)'
        ),
        ['mid-limited'] = make_adapter('mid-limited', {
            streaming = true,
            cancellation = false,
            resume = false,
            permissions = false,
            artifacts = false,
            remote = false,
            tools = true,
        }, 'driver metadata: limited capabilities'),
    }
    local probes = {
        { label = 'tool needs tools', needs = { tools = true }, allow = nil },
        { label = 'tool needs resume', needs = { resume = true }, allow = nil },
        {
            label = 'tool needs remote+permissions',
            needs = { remote = true, permissions = true },
            allow = nil,
        },
        -- No needs at all: name order alone decides, even though the
        -- driver-side metadata rates zeta-slow higher quality.
        { label = 'no needs (name order decides)', needs = {}, allow = nil },
        -- Driver-side allowlist: the harness has no allow concept, so
        -- the caller filters the table before negotiating.
        {
            label = 'allowlist {mid-limited, zeta-slow} needs artifacts',
            needs = { artifacts = true },
            allow = { 'mid-limited', 'zeta-slow' },
        },
    }
    local trace_probes = {}
    for i, probe in ipairs(probes) do
        local pool = adapters
        if probe.allow ~= nil then
            pool = {}
            for _, name in ipairs(probe.allow) do
                pool[name] = adapters[name]
            end
        end
        local selected, err = handles.adapter.negotiate(pool, probe.needs)
        if selected == nil then
            return fail('negotiate', 'probe ' .. i .. ' (' .. probe.label .. ') failed: ' .. tostring(err))
        end
        local satisfying = satisfying_names(handles, pool, probe.needs)
        local want = satisfying[1]
        ev('probe ' .. i .. ' [' .. probe.label .. ']: satisfying={' .. table.concat(satisfying, ',')
            .. '} selected=' .. selected.name)
        if selected.name ~= want then
            return fail(
                'strategy',
                'probe ' .. i .. ' selected ' .. selected.name .. ', want first-sorted-satisfying '
                    .. tostring(want) .. ' — selection strategy refuted'
            )
        end
        trace_probes[#trace_probes + 1] = {
            label = probe.label,
            needs = probe.needs,
            candidates_sorted = sorted_names(pool),
            satisfying_sorted = satisfying,
            selected = selected.name,
        }
    end
    ev('all 5 probes selected the first name in sorted order among satisfying adapters')
    ev('zeta-slow carries the higher driver-side quality note and still loses every '
        .. 'tie to alpha-fast: quality is not an input')
    local trace = {
        capability_keys = keys,
        adapters = {},
        probes = trace_probes,
    }
    for name, a in pairs(adapters) do
        trace.adapters[name] = { caps = a.probe(), quality_note = a._quality_note }
    end
    local werr = write_work_file(handles, 'selection-trace.json', vim.json.encode(trace))
    if werr ~= nil then
        return fail('trace', werr)
    end
    ev('wrote selection-trace.json (5 probes, 3 adapters)')
    return pass()
end

---Read at most READ_MAX bytes of a file.
---@param path string
---@return string? data
---@return string? err
local function read_file(path)
    local f, err = io.open(path, 'r')
    if f == nil then
        return nil, tostring(err)
    end
    local data = f:read(READ_MAX)
    f:close()
    if data == nil then
        return '', nil
    end
    return data, nil
end

---Case-insensitive substring scan; returns {line, token} hits.
---@param data string
---@param tokens string[]
---@return table[] hits
local function scan_tokens(data, tokens)
    local hits = {}
    local lineno = 0
    for line in (data .. '\n'):gmatch('([^\n]*)\n') do
        lineno = lineno + 1
        local lower = line:lower()
        for _, token in ipairs(tokens) do
            if lower:find(token, 1, true) ~= nil then
                hits[#hits + 1] = { line = lineno, token = token, text = line:sub(1, 120) }
            end
        end
    end
    return hits
end

---Scenario "modelscan": scan the real module source for model-selection
---tradeoff vocabulary. Emits raw hits; the verdict is the harness's job.
---@param handles table
---@return table verdict
local function scenario_modelscan(handles)
    local info = debug.getinfo(handles.adapter.negotiate, 'S')
    local source = (info and info.source) or ''
    if source:sub(1, 1) ~= '@' then
        return fail('modelscan', 'negotiate source is not a file: ' .. source)
    end
    local adapter_path = source:sub(2)
    local types_path = adapter_path:gsub('adapter%.lua$', 'types.lua')
    ev('adapter.lua resolved via debug.getinfo: ' .. adapter_path)
    ev('types.lua sibling: ' .. types_path)
    local adata, aerr = read_file(adapter_path)
    if adata == nil then
        return fail('modelscan', 'cannot read adapter.lua: ' .. aerr)
    end
    local tdata, terr = read_file(types_path)
    if tdata == nil then
        return fail('modelscan', 'cannot read types.lua: ' .. terr)
    end
    local tradeoff_tokens = {
        'latency',
        'memory',
        'quality',
        'benchmark',
        'model_card',
        'model card',
        'throughput',
        'vram',
        'quant',
        'tradeoff',
        'trade-off',
        'cost_per',
        'price',
    }
    local tradeoff_hits = {}
    for _, hit in ipairs(scan_tokens(adata, tradeoff_tokens)) do
        hit.file = 'adapter.lua'
        tradeoff_hits[#tradeoff_hits + 1] = hit
    end
    for _, hit in ipairs(scan_tokens(tdata, tradeoff_tokens)) do
        hit.file = 'types.lua'
        tradeoff_hits[#tradeoff_hits + 1] = hit
    end
    local model_hits = {}
    for _, hit in ipairs(scan_tokens(adata, { 'model' })) do
        hit.file = 'adapter.lua'
        model_hits[#model_hits + 1] = hit
    end
    for _, hit in ipairs(scan_tokens(tdata, { 'model' })) do
        hit.file = 'types.lua'
        model_hits[#model_hits + 1] = hit
    end
    ev('tradeoff-vocabulary hits in adapter.lua + types.lua: ' .. #tradeoff_hits)
    for _, hit in ipairs(tradeoff_hits) do
        ev('tradeoff hit: ' .. hit.file .. ':' .. hit.line .. ' token=' .. hit.token)
    end
    ev("'model' token hits: " .. #model_hits)
    for _, hit in ipairs(model_hits) do
        ev("model hit: " .. hit.file .. ':' .. hit.line .. ': ' .. hit.text)
    end
    local scan = {
        files = { adapter = adapter_path, types = types_path },
        tradeoff_tokens = tradeoff_tokens,
        tradeoff_hits = tradeoff_hits,
        model_hits = model_hits,
    }
    local werr = write_work_file(handles, 'modelscan.json', vim.json.encode(scan))
    if werr ~= nil then
        return fail('trace', werr)
    end
    ev('wrote modelscan.json')
    return pass()
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'selection'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return fail('bootstrap', boot_err)
    end
    ev('scenario=' .. scenario)
    if scenario == 'selection' then
        return scenario_selection(handles)
    elseif scenario == 'modelscan' then
        return scenario_modelscan(handles)
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
