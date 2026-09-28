-- task_64.lua -- gauntlet driver: deterministic tool selection.
--
-- The design asks for the tool-selection / dispatch seam: two tools both
-- match an intent -> the choice must be deterministic and explained.
-- Scenarios: default (one matches -> selected); ambiguity (two match ->
-- the documented precedence rule picks one, and the rationale is
-- logged); adversarial: the same ambiguous input 100 times -> the same
-- choice every time (no hash-order or timing dependence).
--
-- Seam mapping (documented, not invented): diver has no intent-string
-- matcher — dispatch is by exact name everywhere (rose/tools.lua
-- `M.call`, mcp/tools.lua `describe`, registry `get_tool`). The one
-- capability-based SELECTION seam is `ai.harness.adapter.negotiate`:
-- "Choose the first adapter (in sorted name order) whose probed
-- capabilities satisfy every requested need." Intent ~= requested
-- capability needs; tools ~= adapters. The real supervisor uses it at
-- launch (supervisor.lua calls adapter.negotiate(adapters,
-- { cancellation = true })). This driver exercises the REAL
-- ai.harness.adapter.negotiate with mock adapters.
--
-- Honest result: PASS on the core dimension, with two banked caveats.
-- Selection IS a pure function of (needs, adapter set, probe outcomes):
-- names are table.sort'ed before first-match, so Lua hash order can
-- never leak in; the 100-run stability test proves it, including across
-- reversed registration order. The design's "explained" half is met
-- DOCUMENTARILY (the rule is in the docstring and the registry header:
-- "listed in sorted order so behavior is deterministic and auditable")
-- but NOT per-selection: negotiate returns only the adapter, no
-- rationale record is produced or logged (caveat C1, banked). The
-- "precedence config" is likewise absent: precedence is hardcoded
-- sorted-name order, not a config (caveat C2, banked). A flapping probe
-- (intermittent probe failure) changes the outcome — negotiate treats
-- probe failure as "unavailable" and skips, never raises — but that is
-- an input change, not nondeterminism: determinism is conditioned on
-- probe outcomes, and the probe contract says failures become
-- unavailable capabilities.
--
-- Scenarios via GAUNTLET_SCENARIO (default "single-match"):
--   single-match            V: one adapter satisfies the need -> selected
--   ambiguity-sorted-name   V: two satisfy -> first in sorted name order,
--                           regardless of registration order
--   hundred-run-stability   A: 100 ambiguous selections, alternating
--                           registration order -> identical winner
--   probe-flap-boundary     A: intermittently failing probe changes the
--                           outcome (input change, not nondeterminism)
--
-- Prints exactly one JSON verdict line to stdout and always exits 0; the
-- verdict carries the outcome, not the exit code. Writes nothing outside
-- GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64
local TASK_ID = 'task-64'

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

---All capability keys negotiate iterates (ai.harness.types CAPABILITY_KEYS).
local CAP_KEYS = {
    'streaming',
    'cancellation',
    'resume',
    'permissions',
    'artifacts',
    'remote',
    'tools',
}

---Build a mock adapter honoring the AiHarnessAdapter contract.
---@param name string
---@param caps table<string, boolean> capabilities to enable
---@return table adapter
local function mock_adapter(name, caps)
    local full = {}
    for _, key in ipairs(CAP_KEYS) do
        full[key] = caps[key] == true
    end
    return {
        name = name,
        probe = function()
            return full
        end,
        start = function()
            return nil, 'mock: start not used'
        end,
        cancel = function()
            return true
        end,
        close = function() end,
    }
end

---Build a mock adapter whose probe FAILS intermittently (raises on every
---other call): models a flapping capability source.
---@param name string
---@param caps table<string, boolean>
---@return table adapter
---@return table counter
local function flapping_adapter(name, caps)
    local full = {}
    for _, key in ipairs(CAP_KEYS) do
        full[key] = caps[key] == true
    end
    local counter = { calls = 0 }
    local adapter = {
        name = name,
        probe = function()
            counter.calls = counter.calls + 1
            if counter.calls % 2 == 0 then
                error('mock: intermittent probe failure', 0)
            end
            return full
        end,
        start = function()
            return nil, 'mock: start not used'
        end,
        cancel = function()
            return true
        end,
        close = function() end,
    }
    return adapter, counter
end

---Facet V: one adapter satisfies the need -> it is selected.
---@param negotiate table the real ai.harness.adapter module
---@return string? err
local function facet_single_match(negotiate)
    ev('facet single-match: one of two adapters satisfies {cancellation=true}')
    local adapters = {
        alpha = mock_adapter('alpha', {}),
        beta = mock_adapter('beta', { cancellation = true }),
    }
    local chosen, err = negotiate.negotiate(adapters, { cancellation = true })
    if chosen == nil then
        return 'negotiate failed: ' .. tostring(err)
    end
    if chosen.name ~= 'beta' then
        return 'expected beta, got ' .. tostring(chosen.name)
    end
    ev('winner=beta: the only adapter satisfying the need was selected')
    return nil
end

---Facet V: two adapters satisfy -> first in sorted name order wins,
---independent of registration (insertion) order.
---@param negotiate table
---@return string? err
local function facet_ambiguity_sorted_name(negotiate)
    ev('facet ambiguity-sorted-name: two adapters satisfy; registered zeta-first')
    local adapters = {}
    adapters['zeta'] = mock_adapter('zeta', { tools = true })
    adapters['alpha'] = mock_adapter('alpha', { tools = true })
    local chosen, err = negotiate.negotiate(adapters, { tools = true })
    if chosen == nil then
        return 'negotiate failed: ' .. tostring(err)
    end
    if chosen.name ~= 'alpha' then
        return 'expected alpha (sorted-name-first), got ' .. tostring(chosen.name)
    end
    ev('winner=alpha despite zeta being registered first: precedence rule = '
        .. '"first adapter (in sorted name order) whose probed capabilities satisfy '
        .. 'every requested need" (ai.harness.adapter.negotiate docstring)')
    return nil
end

---Facet A: 100 ambiguous selections with alternating registration order
---must pick the same winner every time — no hash-order or timing
---dependence.
---@param negotiate table
---@return string? err
local function facet_hundred_run_stability(negotiate)
    ev('facet hundred-run-stability: 100 ambiguous selections, alternating insertion order')
    local first = nil
    for i = 1, 100 do
        local adapters = {}
        if i % 2 == 0 then
            adapters['zeta'] = mock_adapter('zeta', { tools = true, remote = true })
            adapters['alpha'] = mock_adapter('alpha', { tools = true, remote = true })
        else
            adapters['alpha'] = mock_adapter('alpha', { tools = true, remote = true })
            adapters['zeta'] = mock_adapter('zeta', { tools = true, remote = true })
        end
        local chosen, err = negotiate.negotiate(adapters, { tools = true, remote = true })
        if chosen == nil then
            return 'negotiate failed on run ' .. i .. ': ' .. tostring(err)
        end
        if first == nil then
            first = chosen.name
        elseif chosen.name ~= first then
            return 'run ' .. i .. ' chose ' .. tostring(chosen.name) .. ', expected ' .. first
        end
    end
    if first ~= 'alpha' then
        return 'stability winner should be alpha, got ' .. tostring(first)
    end
    ev('100/100 runs chose alpha: selection is a pure function of (needs, adapter set); '
        .. 'table.sort over names keeps Lua hash order out of the decision')
    return nil
end

---Facet A: an intermittently failing probe changes the outcome. This is
---an INPUT change (probe outcomes are inputs), not nondeterminism:
---negotiate treats probe failure as "unavailable" and skips, never
---raises. It also surfaces the logging gap: negotiate returns only the
---adapter, so the flap is silent unless the caller instruments probes.
---@param negotiate table
---@return string? err
local function facet_probe_flap_boundary(negotiate)
    ev('facet probe-flap-boundary: gamma satisfies the need but its probe fails every 2nd call')
    local beta = mock_adapter('beta', { tools = false })
    local gamma, counter = flapping_adapter('gamma', { tools = true })
    local adapters = { beta = beta, gamma = gamma }
    local winners = {}
    for i = 1, 6 do
        -- gamma sorts after beta; when gamma's probe is healthy it wins
        -- (beta lacks tools); when it flaps, no adapter satisfies.
        local chosen, err = negotiate.negotiate(adapters, { tools = true })
        winners[#winners + 1] = (chosen ~= nil) and chosen.name or ('none:' .. tostring(err))
    end
    ev('winners over 6 runs: ' .. table.concat(winners, ', ') .. ' (probe calls: ' .. counter.calls .. ')')
    if winners[1] ~= 'gamma' or winners[3] ~= 'gamma' or winners[5] ~= 'gamma' then
        return 'expected gamma on healthy-probe runs, got: ' .. table.concat(winners, ', ')
    end
    if winners[2] == 'gamma' or winners[4] == 'gamma' or winners[6] == 'gamma' then
        return 'flapping probe must not win: ' .. table.concat(winners, ', ')
    end
    ev('probe failure -> adapter skipped (never raises): determinism holds CONDITIONAL on '
        .. 'probe outcomes; a flapping probe is an input change, and the probe contract '
        .. '("probe failures become unavailable capabilities") makes that explicit')
    ev('gap surfaced: negotiate returns only the adapter table — no rationale record '
        .. 'is produced or logged, so a selection change caused by a probe flap is '
        .. 'silent unless the caller instruments probes (caveat C1)')
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
        scenario = 'single-match'
    end
    ev('scenario=' .. scenario)
    local ok, adapter_mod = pcall(require, 'ai.harness.adapter')
    if not ok or type(adapter_mod) ~= 'table' then
        return fail('bootstrap', 'cannot require ai.harness.adapter: ' .. tostring(adapter_mod))
    end
    if type(adapter_mod.negotiate) ~= 'function' then
        return fail('seam', 'ai.harness.adapter.negotiate is not a function: seam changed shape')
    end
    ev('driving the REAL ai.harness.adapter.negotiate (used by supervisor.lua at launch)')
    local err
    if scenario == 'single-match' then
        err = facet_single_match(adapter_mod)
    elseif scenario == 'ambiguity-sorted-name' then
        err = facet_ambiguity_sorted_name(adapter_mod)
    elseif scenario == 'hundred-run-stability' then
        err = facet_hundred_run_stability(adapter_mod)
    elseif scenario == 'probe-flap-boundary' then
        err = facet_probe_flap_boundary(adapter_mod)
    else
        return fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
    end
    if err ~= nil then
        return fail('selection', err)
    end
    ev('CAVEAT C1 (banked): negotiate returns only the adapter — no rationale record is '
        .. 'produced or logged per selection. The rule is documented (docstring + registry '
        .. '"sorted order so behavior is deterministic and auditable"), not logged.')
    ev('CAVEAT C2 (banked): precedence is hardcoded sorted-name order; there is no '
        .. 'precedence config. Selection is a pure function of (needs, adapter set, '
        .. 'probe outcomes) — fewer degrees of freedom, still pure.')
    ev('PASS: deterministic selection verified by mechanism (table.sort before '
        .. 'first-match) and by the 100-run stability battery')
    return pass()
end

local ok, verdict = pcall(main)
if not ok then
    ev('lua error: ' .. tostring(verdict))
    verdict = fail('lua-driver', 'unhandled error: ' .. tostring(verdict))
end
-- Verdict on the real stdout: in `nvim --headless -l`, Lua print() goes
-- to stderr, but the Rust runner parses the verdict from stdout.
io.stdout:write(vim.json.encode(verdict) .. '\n')
