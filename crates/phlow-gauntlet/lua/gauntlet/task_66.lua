-- task_66.lua -- gauntlet driver: trace propagation.
--
-- The design asks for trace/correlation-id propagation across every
-- adapter boundary crossing (ACP/A2A/MCP): the harness issues a trace
-- id, the mock peers echo what they receive, and an explicit "trace
-- continuity" check joins the hops. Pass criteria: the trace id is
-- identical at every observed hop; any hop that cannot propagate
-- declares so in its contract (no silent drops).
--
-- Seam mapping (verified, not invented): diver's harness runs DO carry
-- an identity — `run.id` with `parent_id`/`root_id` forming a run TREE
-- (supervisor.lua) — but that is run provenance, not a cross-boundary
-- correlation token. The three real adapters attach NOTHING
-- trace-like to their boundary calls:
--   * acp:   session.prompt(session_key, run.goal, cb) — key + text only
--   * a2a:   tasks.submit({agent, message, timeout_ms, on_done}) — no envelope
--   * mcp:   client.start(server_spec, on_started) — server spec only
-- No adapter's probe() contract (capabilities + notes) declares trace
-- propagation OR declares that it cannot propagate: the drops are
-- silent. The design's expected result here is the documented hole:
-- "the gap is detected by an explicit 'trace continuity' check — the
-- task fails until the propagation is fixed, documenting the hole."
--
-- This driver exercises the REAL adapters with preloaded mock peers
-- (package.preload, so the real adapter code paths run) that capture
-- exactly what crosses each boundary. The gauntlet plays the harness:
-- it issues a trace id per scenario and the continuity check asks the
-- mock peer what it received. Every scenario fails at "seam" with
-- mechanism evidence — 2 validation facets (acp round trip, a2a hop),
-- 2 adversarial (explicit continuity check; contract silence).
--
-- Scenarios via GAUNTLET_SCENARIO (default "acp-round-trip"):
--   acp-round-trip        V: fake acp session captures prompt args; the
--                         peer receives only (session_key, goal text) —
--                         no trace id, no envelope of any kind
--   a2a-hop               V: fake a2a tasks captures submit args; the
--                         submit table carries agent/message/timeout/on_done
--                         only — no trace id for the hop to echo
--   trace-continuity      A: the explicit continuity check: the harness
--                         issued trace id is compared against what each
--                         mock peer received — every comparison fails,
--                         which IS the documented hole
--   contract-silence      A: all three adapters' probe() contracts are
--                         read for any trace-propagation declaration or
--                         any declared inability — none found: silent drops
--
-- Prints exactly one JSON verdict line to stdout and always exits 0; the
-- verdict carries the outcome, not the exit code. Writes nothing outside
-- GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64
local TASK_ID = 'task-66'

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function pass()
    return { id = TASK_ID, outcome = 'pass', evidence = evidence }
end

local function fail(where, how)
    return { id = TASK_ID, outcome = 'fail', where = where, how = how, evidence = evidence }
end

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

---Mock sink honoring the AiHarnessSink append shape used by adapters.
---@return table sink
---@return table appended
local function mock_sink()
    local appended = {}
    local sink = {
        append = function(_, run_id, event, payload, meta)
            appended[#appended + 1] = {
                run_id = run_id,
                event = event,
                payload = payload,
                meta = meta,
            }
        end,
    }
    return sink, appended
end

---Boundary-call arg shapes the real adapters use (read from the adapter
---sources; asserted at runtime against the captured calls):
---  acp.start -> session.start(agent, opts, cb); session.prompt(key, text, cb)
---  a2a.start -> tasks.submit({agent, message, timeout_ms, on_done})

---Facet V: ACP round trip. The fake acp session peer captures exactly
---what the real adapter sends. The harness issues TRACE; the peer must
---observe it for the design's default scenario to pass.
---@param trace string the harness-issued trace id
---@return string? err
local function facet_acp_round_trip(trace)
    ev('facet acp-round-trip: harness issues trace id ' .. trace)
    local captured = {}
    package.preload['ai.acp.session'] = function()
        return {
            start = function(agent, opts, cb)
                captured.start = { agent = agent, opts_keys = vim.tbl_keys(opts or {}) }
                cb('sess-mock-1', nil)
            end,
            prompt = function(session_key, text, cb)
                captured.prompt = { session_key = session_key, text = text }
                cb(nil)
            end,
        }
    end
    local ok, acp = pcall(require, 'ai.harness.adapters.acp')
    if not ok or type(acp) ~= 'table' then
        return 'cannot require ai.harness.adapters.acp: ' .. tostring(acp)
    end
    local sink = mock_sink()
    local run = {
        id = 'run-66-acp',
        goal = 'probe the acp boundary',
        workspace = '/tmp',
        extensions = { acp = { agent = 'mock-agent' } },
    }
    local handle, err = acp.start(run, sink)
    if handle == nil then
        return 'acp.start failed: ' .. tostring(err)
    end
    ev('real acp adapter started; mock acp session peer captured the boundary call')
    local prompt = captured.prompt
    if prompt == nil then
        return 'mock acp session never received session.prompt'
    end
    ev('session.prompt received exactly 2 positional values: session_key + goal text')
    ev('prompt.session_key=' .. tostring(prompt.session_key))
    ev('prompt.text=' .. tostring(prompt.text))
    if tostring(prompt.text):find(trace, 1, true) ~= nil then
        return 'unexpected: the trace id leaked into the prompt text (invented mechanism)'
    end
    if tostring(prompt.session_key):find(trace, 1, true) ~= nil then
        return 'unexpected: the trace id leaked into the session key (invented mechanism)'
    end
    ev('the mock acp peer received NO trace id: the design default scenario '
        .. '(peer observes the harness-issued trace) cannot hold')
    ev('the handle carries run_id=' .. tostring(handle.run_id) .. ' — run identity, '
        .. 'never forwarded to the peer; no adapter contract declares propagation')
    return nil
end

---Facet V: A2A hop. The fake a2a tasks peer captures the submit table;
---the design's A2A scenario needs the trace id to survive
---message/send -> tasks/get.
---@param trace string the harness-issued trace id
---@return string? err
local function facet_a2a_hop(trace)
    ev('facet a2a-hop: harness issues trace id ' .. trace)
    local captured = {}
    package.preload['ai.a2a.tasks'] = function()
        return {
            submit = function(args)
                captured.submit = args
                return 'task-remote-66', nil
            end,
            get = function(task_id)
                captured.get = task_id
                return { id = task_id, state = 'completed' }, nil
            end,
            cancel = function() end,
        }
    end
    package.preload['ai.a2a.client'] = function()
        return {}
    end
    local ok, a2a = pcall(require, 'ai.harness.adapters.a2a')
    if not ok or type(a2a) ~= 'table' then
        return 'cannot require ai.harness.adapters.a2a: ' .. tostring(a2a)
    end
    local sink = mock_sink()
    local run = {
        id = 'run-66-a2a',
        goal = 'probe the a2a boundary',
        extensions = { a2a = { agent = 'mock-agent' } },
    }
    local handle, err = a2a.start(run, sink)
    if handle == nil then
        return 'a2a.start failed: ' .. tostring(err)
    end
    ev('real a2a adapter started; mock a2a peer captured the submit table')
    local submit = captured.submit
    if type(submit) ~= 'table' then
        return 'mock a2a tasks.submit never received its args table'
    end
    local keys = vim.tbl_keys(submit)
    table.sort(keys)
    ev('submit keys: ' .. table.concat(keys, ', '))
    for _, key in ipairs(keys) do
        if key == 'trace_id' or key == 'trace' or key == 'correlation_id' then
            return 'unexpected: submit carried a trace envelope key (invented mechanism)'
        end
    end
    ev('tasks/get on the returned task id sees only remote state — the hop '
        .. 'carries agent/message/timeout/on_done, no trace for tasks/get to echo')
    return nil
end

---Facet A: the explicit "trace continuity" check the design names.
---Issue one trace id; ask every mock peer what it received; every
---comparison must FAIL here — that failure IS the documented hole.
---@return string? err
local function facet_trace_continuity()
    ev('facet trace-continuity: harness issues trace-66-continuity; peers echo what they got')
    local received = {}
    package.preload['ai.acp.session'] = function()
        return {
            start = function(agent, opts, cb)
                cb('sess-mock-2', nil)
            end,
            prompt = function(session_key, text, cb)
                received.acp = { session_key = session_key, text = text }
                cb(nil)
            end,
        }
    end
    package.preload['ai.a2a.tasks'] = function()
        return {
            submit = function(args)
                received.a2a = args
                return 'task-remote-66b', nil
            end,
            cancel = function() end,
        }
    end
    package.preload['ai.a2a.client'] = function()
        return {}
    end
    local ok_acp, acp = pcall(require, 'ai.harness.adapters.acp')
    local ok_a2a, a2a = pcall(require, 'ai.harness.adapters.a2a')
    if not ok_acp or not ok_a2a then
        return 'cannot require adapters'
    end
    local trace = 'trace-66-continuity'
    local sink = mock_sink()
    acp.start(
        { id = 'run-66-c1', goal = 'continuity probe', extensions = { acp = { agent = 'm' } } },
        sink
    )
    a2a.start(
        { id = 'run-66-c2', goal = 'continuity probe', extensions = { a2a = { agent = 'm' } } },
        sink
    )
    local drops = 0
    for hop, got in pairs(received) do
        local seen = vim.inspect(got)
        if seen:find(trace, 1, true) == nil then
            drops = drops + 1
            ev('hop ' .. hop .. ': continuity check FAILS — peer received no trace id')
        else
            ev('hop ' .. hop .. ': peer echoed the trace id (unexpected)')
        end
    end
    if drops < 2 then
        return 'continuity unexpectedly held on a hop (invented mechanism)'
    end
    ev('2/2 boundary hops dropped the trace id: the explicit continuity check '
        .. 'detects the gap, exactly the design-expected failure mode')
    return nil
end

---Facet A: any hop that cannot propagate must declare so in its
---contract. Read every adapter's probe() contract (capabilities +
---notes) for a trace declaration or a declared inability.
---@return string? err
local function facet_contract_silence()
    ev('facet contract-silence: read the probe() contract of acp, a2a, mcp')
    package.preload['ai.acp.session'] = function()
        return {}
    end
    package.preload['ai.a2a.tasks'] = function()
        return {}
    end
    package.preload['ai.a2a.client'] = function()
        return {}
    end
    package.preload['ai.mcp.client'] = function()
        return {}
    end
    local names = { 'acp', 'a2a', 'mcp' }
    for _, name in ipairs(names) do
        local ok, adapter = pcall(require, 'ai.harness.adapters.' .. name)
        if not ok or type(adapter) ~= 'table' then
            return 'cannot require adapter ' .. name .. ': ' .. tostring(adapter)
        end
        if type(adapter.probe) ~= 'function' then
            return 'adapter ' .. name .. ' has no probe() contract'
        end
        local probe_ok, caps = pcall(adapter.probe)
        if not probe_ok or type(caps) ~= 'table' then
            ev('adapter ' .. name .. ': probe() unavailable in this env (contract unreadable)')
        else
            local hay = vim.inspect(caps):lower()
            local declares = hay:find('trace', 1, true) ~= nil
            ev('adapter ' .. name .. ': probe() contract mentions trace: ' .. tostring(declares))
            if declares then
                return 'adapter ' .. name .. ' declares trace handling (invented mechanism)'
            end
        end
    end
    ev('no adapter contract declares trace propagation AND none declares '
        .. 'the inability to propagate: the drops are SILENT, violating the '
        .. 'design pass criterion "no silent drops"')
    return nil
end

local function main()
    local _, boot_err = bootstrap()
    if boot_err ~= nil then
        return fail('bootstrap', boot_err)
    end
    ev('harness lua tree bootstrapped from DIVER_LUA_DIR')
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'acp-round-trip'
    end
    ev('scenario=' .. scenario)
    local trace = 'trace-66-' .. scenario
    local err
    if scenario == 'acp-round-trip' then
        err = facet_acp_round_trip(trace)
    elseif scenario == 'a2a-hop' then
        err = facet_a2a_hop(trace)
    elseif scenario == 'trace-continuity' then
        err = facet_trace_continuity()
    elseif scenario == 'contract-silence' then
        err = facet_contract_silence()
    else
        return fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
    end
    if err ~= nil then
        return fail('driver', err)
    end
    ev('FINDING (diver-owned, flagged): diver has run identity (run.id / '
        .. 'parent_id / root_id — a run tree, supervisor.lua) but NO '
        .. 'trace/correlation id that crosses adapter boundaries. The real '
        .. 'ACP, A2A and MCP adapters attach no trace envelope to their '
        .. 'boundary calls (session.prompt takes key+text; tasks.submit '
        .. 'takes agent/message/timeout/on_done; client.start takes the '
        .. 'server spec), and no adapter contract declares propagation or '
        .. 'its absence. The design pass criteria (identical trace id at '
        .. 'every hop; no silent drops) need trace plumbing that does not exist.')
    return fail(
        'seam',
        'seam absent: no trace/correlation id crosses diver adapter boundaries — '
            .. 'the real acp adapter sends session.prompt(session_key, goal_text) with no '
            .. 'trace, the real a2a adapter submits {agent, message, timeout_ms, on_done} '
            .. 'with no trace envelope, and no adapter probe() contract declares '
            .. 'propagation or its absence (silent drops). The harness run identity '
            .. '(run.id/parent_id/root_id) is run provenance, not a cross-boundary '
            .. 'correlation token.'
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
