-- task_72.lua -- gauntlet driver: context handoff fidelity.
--
-- The design asks what EXACTLY crosses the delegation boundary —
-- goal, constraints, budget remainder, tool allowlist — byte-exact
-- where it must be, with authority monotonically narrowing (never
-- widening). Pass criteria: allowlist narrowing enforced (child ⊆
-- parent); budget remainder arithmetic exact across siblings;
-- oversized handoffs fail explicitly; the envelope schema documented
-- field-by-field.
--
-- Seam mapping (verified, not invented): diver's spawn path
-- (supervisor.spawn_child -> supervisor.create, lua/ai/harness/) has
-- NO handoff envelope. What the child run record actually carries:
--   * id / parent_id / root_id / workflow / adapter / workspace /
--     budget / extensions / acceptance — and NOTABLY no `goal`:
--     validate_run_spec REQUIRES spec.goal non-empty, then create()
--     drops it from the run table;
--   * budget = budget.new(spec.budget or DEFAULT_BUDGET_LIMITS): a
--     child with no explicit budget gets FRESH FULL defaults
--     (turn=50, tool_call=200, token=200000, time_ms=600000,
--     byte=10000000) — never the parent's remainder;
--   * no per-run tool allowlist exists: policy.lua's allowlists live
--     in supervisor-global policy RULES (rule.tools), decided per
--     request by M.decide; spawn_child performs no narrowing step;
--   * no size bound on the spec: a 10MB extensions blob is stored
--     silently — no explicit error, no truncation.
-- The design's expected result here is the documented hole: there is
-- no envelope to be faithful to, so every fidelity property fails at
-- the seam. Diver-owned (flagged, never fixed on gauntlet authority).
--
-- This driver plays the harness with the REAL supervisor (mock sink +
-- mock registry; runs are never started). Every scenario fails at
-- "seam" with mechanism evidence — 2 validation facets (the run
-- record's actual field set; budget remainder is NOT inherited), 2
-- adversarial (10MB handoff blob silently accepted; no allowlist
-- narrowing at spawn).
--
-- Scenarios via GAUNTLET_SCENARIO (default "handoff-shape"):
--   handoff-shape   V: enumerate the child run record's fields: what
--                   crossed, and that spec.goal was validated then
--                   dropped (no `goal` field on the run).
--   budget-not-split V: the parent consumes token budget; the child
--                   spawned without an explicit budget gets the FULL
--                   default limit with zero used — not the remainder.
--                   Two siblings each get full defaults: no shared
--                   remainder, double-spend by construction.
--   oversized-blob  A: a 10MB string in spec.extensions.blob — create
--                   succeeds silently. No bound, no explicit error.
--   no-allowlist    A: the supervisor's policy is global; spawn does
--                   no allowlist narrowing. M.decide still enforces
--                   the global rules per request, but the child spec
--                   carries no allowlist and the spawn path has no
--                   narrowing step (child ⊆ parent is unrepresentable).
--
-- Prints exactly one JSON verdict line to stdout and always exits 0.
-- Writes nothing outside GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64
local BLOB_BYTES = 10 * 1024 * 1024
local TASK_ID = 'task-72'

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

---@return table sink
local function mock_sink()
    return {
        append = function(_, _run_id, _event, _payload, _meta) end,
    }
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
    local supervisor = require('ai.harness.supervisor')
    local registry = require('ai.harness.registry')
    local sup, err = supervisor.new({ registry = registry.new(), sink = mock_sink() })
    if sup == nil then
        return nil, 'supervisor.new failed: ' .. tostring(err)
    end
    return { sup = sup, supervisor = supervisor }
end

---@param goal string
---@return table spec
local function base_spec(goal)
    return {
        workflow = 'gauntlet-handoff',
        goal = goal,
        workspace = vim.env.GAUNTLET_WORK_DIR,
        timeout_ms = 60000,
    }
end

---handoff-shape (V): document the envelope field-by-field. The run
---record carries id/parent_id/root_id/workflow/adapter/workspace/
---budget/extensions/acceptance — and spec.goal, though REQUIRED
---non-empty by validate_run_spec, is dropped from the run table.
local function scenario_handoff_shape(handles)
    local parent, perr =
        handles.supervisor.create(handles.sup, base_spec('handoff probe parent'))
    if parent == nil then
        return fail('spawn', 'parent create failed: ' .. tostring(perr))
    end
    local child, cerr =
        handles.supervisor.spawn_child(handles.sup, parent.id, base_spec('handoff probe child'))
    if child == nil then
        return fail('spawn', 'child spawn failed: ' .. tostring(cerr))
    end
    local keys = {}
    for k in pairs(child) do
        keys[#keys + 1] = k
    end
    table.sort(keys)
    ev('child run record fields: ' .. table.concat(keys, ', '))
    if child.goal ~= nil then
        return fail('handoff', 'run record carries goal: the envelope is richer than documented')
    end
    ev('run.goal is ABSENT: validate_run_spec requires spec.goal non-empty, '
        .. 'then create() drops it — the delegation boundary does not even carry the goal '
        .. 'onto the child run record')
    if child.parent_id ~= parent.id then
        return fail('handoff', 'child.parent_id mismatch')
    end
    if child.root_id ~= parent.root_id then
        return fail('handoff', 'child.root_id mismatch')
    end
    ev('parent_id + root_id linked; no envelope schema beyond the run table exists')
    return fail(
        'seam',
        'there is no handoff envelope: the child run record is the spawn spec minus the '
            .. 'goal (validated then dropped), plus a fresh budget. The design\'s '
            .. '"goal, constraints, budget remainder, tool allowlist" boundary has no '
            .. 'implementation to be faithful to.'
    )
end

---budget-not-split (V): the parent spends; the child inherits nothing.
---A child with no explicit budget gets FULL defaults (used = 0), and
---two siblings each get full defaults — the shared remainder the
---design demands does not exist.
local function scenario_budget_not_split(handles)
    local parent, perr =
        handles.supervisor.create(handles.sup, base_spec('budget probe parent'))
    if parent == nil then
        return fail('spawn', 'parent create failed: ' .. tostring(perr))
    end
    local ok, cerr = handles.supervisor.consume(handles.sup, parent.id, 'token', 10)
    if not ok then
        return fail('budget', 'parent consume failed: ' .. tostring(cerr))
    end
    local spent = parent.budget.used.token
    local limit = parent.budget.limits.token
    ev('parent spent token=10: used=' .. tostring(spent) .. ' limit=' .. tostring(limit))
    local child_a, aerr =
        handles.supervisor.spawn_child(handles.sup, parent.id, base_spec('sibling A'))
    if child_a == nil then
        return fail('spawn', 'sibling A spawn failed: ' .. tostring(aerr))
    end
    local child_b, berr =
        handles.supervisor.spawn_child(handles.sup, parent.id, base_spec('sibling B'))
    if child_b == nil then
        return fail('spawn', 'sibling B spawn failed: ' .. tostring(berr))
    end
    local a_used = child_a.budget.used.token
    local a_limit = child_a.budget.limits.token
    local b_used = child_b.budget.used.token
    local b_limit = child_b.budget.limits.token
    ev('sibling A budget: used=' .. tostring(a_used) .. ' limit=' .. tostring(a_limit))
    ev('sibling B budget: used=' .. tostring(b_used) .. ' limit=' .. tostring(b_limit))
    if a_used ~= 0 or b_used ~= 0 then
        return fail('budget', 'child budget used ~= 0: remainder WAS inherited — finding refuted')
    end
    if a_limit ~= limit or b_limit ~= limit then
        return fail('budget', 'child budget limit ~= parent default: unexpected')
    end
    ev('parent remainder would be ' .. (limit - spent) .. '; children got ' .. a_limit
        .. ' each: create() calls budget.new(spec.budget or DEFAULT_BUDGET_LIMITS) — '
        .. 'fresh full budgets, never the parent\'s remainder')
    return fail(
        'seam',
        'budget remainder is not handed off: each child starts with full default limits '
            .. '(token ' .. a_limit .. ', used 0) regardless of the parent\'s spend. Two '
            .. 'siblings can each spend the full ' .. a_limit .. ' — the design\'s "spend by '
            .. 'one deducts from the shared remainder, no double-spend" is absent.'
    )
end

---oversized-blob (A): a 10MB handoff blob is stored silently — no
---bound, no explicit error, no truncation.
local function scenario_oversized_blob(handles)
    local parent, perr =
        handles.supervisor.create(handles.sup, base_spec('blob probe parent'))
    if parent == nil then
        return fail('spawn', 'parent create failed: ' .. tostring(perr))
    end
    local spec = base_spec('blob probe child')
    spec.extensions = { blob = string.rep('x', BLOB_BYTES) }
    local child, cerr = handles.supervisor.spawn_child(handles.sup, parent.id, spec)
    if child == nil then
        return fail(
            'handoff',
            'oversized handoff was REJECTED (' .. tostring(cerr) .. '): '
                .. 'the bound exists — finding refuted'
        )
    end
    local got_bytes = #(child.extensions.blob or '')
    ev('10MB extensions.blob stored on the child run: ' .. got_bytes .. ' bytes round-tripped')
    if got_bytes ~= BLOB_BYTES then
        return fail('handoff', 'blob was truncated to ' .. got_bytes .. ' bytes silently')
    end
    ev('no size bound on the spec, no explicit error, no truncation marker: silent unbounded '
        .. 'acceptance across the delegation boundary')
    return fail(
        'seam',
        'the design demands oversized handoffs "fail explicitly, not silent truncation"; '
            .. 'diver silently stores the full 10MB on the run record. No handoff size bound '
            .. 'exists anywhere in the spawn path.'
    )
end

---no-allowlist (A): authority never narrows at spawn because there is
---no per-run allowlist to narrow. policy.lua allowlists live in
---supervisor-global RULES; M.decide enforces them per request — the
---spawn path has no narrowing step, so child ⊆ parent is
---unrepresentable and a widened child cannot be rejected at spawn.
local function scenario_no_allowlist(handles)
    local policy_mod = require('ai.harness.policy')
    local registry = require('ai.harness.registry')
    local policy, pol_err = policy_mod.new({
        default = 'deny',
        rules = {
            { risk = 'observe', decision = 'allow', tools = { 'fs.read' } },
        },
    })
    if policy == nil then
        return fail('policy', 'policy.new failed: ' .. tostring(pol_err))
    end
    local sup, sup_err =
        handles.supervisor.new({ registry = registry.new(), sink = mock_sink(), policy = policy })
    if sup == nil then
        return fail('supervisor', 'supervisor.new failed: ' .. tostring(sup_err))
    end
    local parent, perr = handles.supervisor.create(sup, base_spec('allowlist probe parent'))
    if parent == nil then
        return fail('spawn', 'parent create failed: ' .. tostring(perr))
    end
    -- The global policy works per request: fs.write is denied.
    local decision = policy_mod.decide(policy, {
        risk = 'observe',
        tool = 'fs.write',
        workspace = vim.env.GAUNTLET_WORK_DIR,
    })
    ev('global policy decides fs.write -> ' .. decision.decision .. ' (per-request enforcement works)')
    if decision.decision ~= 'deny' then
        return fail('policy', 'global policy did not deny fs.write')
    end
    -- But spawn performs no narrowing: the child spec has no allowlist
    -- field, the run record stores none, and the "child requests a tool
    -- the parent lacks" rejection cannot happen at spawn.
    local child, cerr = handles.supervisor.spawn_child(sup, parent.id, base_spec('allowlist probe child'))
    if child == nil then
        return fail('spawn', 'child spawn failed: ' .. tostring(cerr))
    end
    local narrowed = child.tool_allowlist or (child.extensions or {}).tool_allowlist
    ev('child run record tool_allowlist: ' .. tostring(narrowed) .. ' (no such field exists)')
    if narrowed ~= nil then
        return fail('handoff', 'child carries a tool allowlist: narrowing may exist — finding refuted')
    end
    local child_decision = policy_mod.decide(policy, {
        risk = 'observe',
        tool = 'fs.read',
        workspace = vim.env.GAUNTLET_WORK_DIR,
    })
    ev('after spawn, policy still decides fs.read -> ' .. child_decision.decision
        .. ' globally: nothing was narrowed for the child')
    return fail(
        'seam',
        'allowlist narrowing is unrepresentable at spawn: policy allowlists are '
            .. 'supervisor-global rules (policy.lua), the spawn spec has no allowlist field, '
            .. 'and spawn_child performs no narrowing step. The design\'s "child ⊆ parent, '
            .. 'verified; a widened child is rejected at spawn" has no implementation.'
    )
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'handoff-shape'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return fail('bootstrap', boot_err)
    end
    ev('scenario=' .. scenario)
    if scenario == 'handoff-shape' then
        return scenario_handoff_shape(handles)
    elseif scenario == 'budget-not-split' then
        return scenario_budget_not_split(handles)
    elseif scenario == 'oversized-blob' then
        return scenario_oversized_blob(handles)
    elseif scenario == 'no-allowlist' then
        return scenario_no_allowlist(handles)
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
