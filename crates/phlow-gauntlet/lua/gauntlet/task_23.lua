-- task-23 driver: bounded dynamic fan-out for diver's ai.harness.
--
-- Drives the REAL spawn path: supervisor.spawn_child -> supervisor.create.
-- The bound under test is the supervisor's total-run cap (RUNS_MAX = 256,
-- supervisor.lua): M.create rejects with the explicit error
-- 'supervisor run bound exceeded' once sup.run_count reaches sup.runs_max.
-- run_count never decrements, so the cap covers total runs ever created —
-- which implies live runs can never exceed it either. There is NO
-- per-parent live-child bound and NO explicit depth bound; recursion is
-- stopped only by the total-run cap. Both facts are asserted and
-- documented here, not papered over.
--
-- Two fake adapters: gauntlet_instant completes a run the moment it starts
-- (children); gauntlet_holding never completes on its own, so the parent
-- stays `running` while children spawn — create() refuses
-- 'parent run is already terminal' for a finished parent.
--
-- Scenarios via GAUNTLET_SCENARIO (default "default"):
--   default     small N=5: spawn, start, complete; parent finishes last
--   attacker-n  N=10000 spawn attempts: exactly 255 succeed, rest get the
--               explicit bound error; no hang, no OOM
--   recursive   fork-bomb shape: every run spawns 3 children until the
--               bound; the cascade terminates with explicit errors
--
-- Prints exactly one JSON verdict line to stdout and always exits 0.
-- Writes nothing outside GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64
local WAIT_POLL_MS = 25
local COMPLETE_WAIT_MS = 15000
local ATTACKER_N = 10000
local SMALL_N = 5
local BRANCHING = 3

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function fail(where, how)
    return { id = 'task-23', outcome = 'fail', where = where, how = how, evidence = evidence }
end

local function pass()
    return { id = 'task-23', outcome = 'pass', evidence = evidence }
end

---Fake adapter that completes a run the moment it starts.
local instant_adapter = { name = 'gauntlet_instant' }

function instant_adapter.probe()
    return {
        available = true,
        streaming = false,
        cancellation = true,
        resume = false,
        permissions = false,
        artifacts = false,
        remote = false,
        tools = false,
    }
end

---@param run table
---@param sink table
function instant_adapter.start(run, sink)
    assert(run ~= nil, 'run required')
    assert(sink ~= nil, 'sink required')
    sink:append(
        run.id,
        'model.completed',
        { outcome = 'completed' },
        { source = 'gauntlet_instant' }
    )
    return { adapter = 'gauntlet_instant', run_id = run.id, closed = false }
end

---@param handle table
---@return boolean
function instant_adapter.cancel(handle)
    assert(handle ~= nil, 'handle required')
    return true
end

---@param handle table
function instant_adapter.close(handle)
    assert(handle ~= nil, 'handle required')
    handle.closed = true
end

---Fake adapter that NEVER completes on its own: a run started with it stays
---`running` until the driver explicitly finishes/cancels it. Used for the
---parent, because supervisor.create refuses 'parent run is already terminal'
---on spawn (supervisor.lua), and the instant adapter's run self-finishes on
---the first supervisor tick (drain_completions).
local holding_adapter = { name = 'gauntlet_holding' }

function holding_adapter.probe()
    return {
        available = true,
        streaming = false,
        cancellation = true,
        resume = false,
        permissions = false,
        artifacts = false,
        remote = false,
        tools = false,
    }
end

---@param run table
---@param sink table
function holding_adapter.start(run, sink)
    assert(run ~= nil, 'run required')
    assert(sink ~= nil, 'sink required')
    -- Deliberately no sink append: no model.completed event, no auto-finish.
    return { adapter = 'gauntlet_holding', run_id = run.id, closed = false }
end

---@param handle table
---@return boolean
function holding_adapter.cancel(handle)
    assert(handle ~= nil, 'handle required')
    return true
end

---@param handle table
function holding_adapter.close(handle)
    assert(handle ~= nil, 'handle required')
    handle.closed = true
end

---Wire up the harness and the fake adapter. Returns handles.
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
    local harness = require('ai.harness')
    local ok, err = harness.setup({})
    if not ok then
        return nil, 'harness.setup failed: ' .. tostring(err)
    end
    local st = harness._state
    if st == nil then
        return nil, 'harness internal state unavailable after setup'
    end
    local registry = require('ai.harness.registry')
    local reg_ok, reg_err = registry.register_adapter(st.registry, 'gauntlet_instant', instant_adapter)
    if not reg_ok then
        return nil, 'fake adapter registration failed: ' .. tostring(reg_err)
    end
    local hold_ok, hold_err = registry.register_adapter(st.registry, 'gauntlet_holding', holding_adapter)
    if not hold_ok then
        return nil, 'holding adapter registration failed: ' .. tostring(hold_err)
    end
    return {
        harness = harness,
        sup = st.supervisor,
        sink = st.sink,
        supervisor = require('ai.harness.supervisor'),
        types = require('ai.harness.types'),
    }
end

---@param handles table
---@param parent_id string
---@return table spec
local function child_spec(handles, parent_id)
    return {
        workflow = 'gauntlet-fanout',
        goal = 'fan-out probe child of ' .. parent_id,
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = 'gauntlet_instant',
        timeout_ms = 30000,
    }
end

---@param handles table
---@return string? run_id
---@return string? err
local function start_parent(handles)
    -- gauntlet_holding: the parent must stay `running` while children
    -- spawn; the instant adapter would self-complete on the first tick,
    -- and create() rejects 'parent run is already terminal'.
    return handles.harness.run({
        workflow = 'gauntlet-fanout',
        goal = 'fan-out probe parent',
        workspace = vim.env.GAUNTLET_WORK_DIR,
        adapter = 'gauntlet_holding',
        timeout_ms = 60000,
    })
end

---@param handles table
---@param run_id string
---@return string state
local function run_state(handles, run_id)
    local run = handles.supervisor.get(handles.sup, run_id)
    if run == nil then
        return 'missing'
    end
    return run.state
end

---Drive the supervisor clock until every id in `ids` is terminal.
---@param handles table
---@param ids string[]
---@param timeout_ms integer
---@return boolean all_terminal
local function wait_all_terminal(handles, ids, timeout_ms)
    local deadline = vim.uv.hrtime() + (timeout_ms * 1000000)
    while true do
        handles.supervisor.tick(handles.sup, handles.types.now_ns())
        local pending = 0
        for _, id in ipairs(ids) do
            if not handles.types.is_terminal(run_state(handles, id)) then
                pending = pending + 1
            end
        end
        if pending == 0 then
            return true
        end
        if vim.uv.hrtime() >= deadline then
            return false
        end
        vim.wait(WAIT_POLL_MS)
    end
end

---default (V): small N spawns and completes; parent finishes after children.
local function scenario_default(handles)
    local parent_id, parent_err = start_parent(handles)
    if parent_id == nil then
        return fail('parent', 'harness.run failed: ' .. tostring(parent_err))
    end
    ev('parent run: ' .. parent_id)
    local children = {}
    for i = 1, SMALL_N do
        local child, err =
            handles.supervisor.spawn_child(handles.sup, parent_id, child_spec(handles, parent_id))
        if child == nil then
            return fail('spawn', 'spawn_child ' .. i .. ' failed: ' .. tostring(err))
        end
        children[#children + 1] = child.id
        local start_ok, start_err =
            handles.supervisor.start_run(handles.sup, child.id, 'gauntlet_instant')
        if not start_ok then
            return fail('spawn', 'start_run for child ' .. i .. ' failed: ' .. tostring(start_err))
        end
    end
    ev('spawned children: ' .. #children)
    local parent = handles.supervisor.get(handles.sup, parent_id)
    if #parent.children ~= SMALL_N then
        return fail('spawn', 'parent.children has ' .. #parent.children .. ', want ' .. SMALL_N)
    end
    ev('parent.children linked: ' .. SMALL_N .. ' (structured ownership)')
    for _, id in ipairs(children) do
        local child = handles.supervisor.get(handles.sup, id)
        if child.parent_id ~= parent_id then
            return fail('spawn', 'child ' .. id .. ' has wrong parent_id')
        end
        if child.root_id ~= parent.root_id then
            return fail('spawn', 'child ' .. id .. ' has wrong root_id')
        end
    end
    ev('parent_id + root_id propagated to every child')
    if not wait_all_terminal(handles, children, COMPLETE_WAIT_MS) then
        return fail('complete', 'children did not all reach terminal state')
    end
    ev('all ' .. SMALL_N .. ' children completed')
    -- A parent cannot finish while it owns live children (supervisor.finish
    -- refuses); with children terminal, the parent may finish.
    local fin_ok, fin_err = handles.supervisor.finish(handles.sup, parent_id, 'completed', 'gauntlet')
    if not fin_ok then
        return fail('complete', 'parent finish failed: ' .. tostring(fin_err))
    end
    if run_state(handles, parent_id) ~= 'completed' then
        return fail('complete', 'parent not completed')
    end
    ev('parent finished after its children: small-N fan-out spawns and completes')
    return pass()
end

---attacker-n (A): N=10000 spawn attempts against the real spawn path. The
---supervisor's total-run cap (runs_max = 256) must hold: attempts past the
---cap fail with the explicit 'supervisor run bound exceeded' error — no
---hang, no unbounded growth.
local function scenario_attacker_n(handles)
    local parent_id, parent_err = start_parent(handles)
    if parent_id == nil then
        return fail('parent', 'harness.run failed: ' .. tostring(parent_err))
    end
    local t0 = vim.uv.hrtime()
    local succeeded = 0
    local rejected = 0
    local wrong_error = 0
    for _ = 1, ATTACKER_N do
        local child, err =
            handles.supervisor.spawn_child(handles.sup, parent_id, child_spec(handles, parent_id))
        if child ~= nil then
            succeeded = succeeded + 1
        else
            rejected = rejected + 1
            if tostring(err) ~= 'supervisor run bound exceeded' then
                wrong_error = wrong_error + 1
                if wrong_error <= 3 then
                    ev('unexpected spawn error: ' .. tostring(err))
                end
            end
        end
    end
    local elapsed_ms = (vim.uv.hrtime() - t0) / 1000000
    ev('spawn attempts: ' .. ATTACKER_N)
    ev('spawned ok: ' .. succeeded .. '; rejected: ' .. rejected)
    ev('spawn loop wall time: ' .. string.format('%.1f', elapsed_ms) .. ' ms (no hang)')
    -- The parent run holds 1 slot of the 256-run cap.
    local want_ok = handles.sup.runs_max - 1
    if succeeded ~= want_ok then
        return fail(
            'bound',
            'expected ' .. want_ok .. ' successful spawns before the cap, got ' .. succeeded
        )
    end
    ev('cap held at runs_max=' .. handles.sup.runs_max .. ' total runs (parent + ' .. succeeded .. ' children)')
    if rejected ~= ATTACKER_N - want_ok then
        return fail('bound', 'rejection count mismatch: ' .. rejected)
    end
    if wrong_error ~= 0 then
        return fail('bound', wrong_error .. ' rejections carried the wrong error')
    end
    ev("every over-bound spawn failed with the explicit error 'supervisor run bound exceeded'")
    if handles.sup.run_count ~= handles.sup.runs_max then
        return fail('bound', 'run_count=' .. tostring(handles.sup.run_count) .. ' after the storm')
    end
    ev('run_count pinned at the cap: no unbounded growth, no OOM vector')
    -- Live-child accounting: every created child is still live (never
    -- started), so live count == created count <= cap.
    local live = 0
    for _, id in
        ipairs(handles.supervisor.get(handles.sup, parent_id).children)
    do
        if not handles.types.is_terminal(run_state(handles, id)) then
            live = live + 1
        end
    end
    if live ~= succeeded then
        return fail('bound', 'live children ' .. live .. ' ~= created ' .. succeeded)
    end
    ev('live children: ' .. live .. ' (never exceeds the total-run cap)')
    return pass()
end

---recursive (A): fork-bomb shape — every run spawns BRANCHING children,
---breadth-first, until the supervisor refuses. The cascade must terminate:
---total runs pinned at the cap, every refusal explicit. Documents the real
---mechanism: there is no depth bound; the total-run cap is the fork-bomb
---stop, so a linear chain could still reach depth 255.
local function scenario_recursive(handles)
    local parent_id, parent_err = start_parent(handles)
    if parent_id == nil then
        return fail('parent', 'harness.run failed: ' .. tostring(parent_err))
    end
    local t0 = vim.uv.hrtime()
    local frontier = { parent_id }
    local created = 0
    local refused = 0
    local wrong_error = 0
    local max_depth = 0
    local depth_of = { [parent_id] = 0 }
    while #frontier > 0 do
        local next_frontier = {}
        for _, pid in ipairs(frontier) do
            for _ = 1, BRANCHING do
                local child, err = handles.supervisor.spawn_child(
                    handles.sup,
                    pid,
                    child_spec(handles, pid)
                )
                if child ~= nil then
                    created = created + 1
                    depth_of[child.id] = depth_of[pid] + 1
                    if depth_of[child.id] > max_depth then
                        max_depth = depth_of[child.id]
                    end
                    next_frontier[#next_frontier + 1] = child.id
                else
                    refused = refused + 1
                    if tostring(err) ~= 'supervisor run bound exceeded' then
                        wrong_error = wrong_error + 1
                    end
                end
            end
        end
        frontier = next_frontier
    end
    local elapsed_ms = (vim.uv.hrtime() - t0) / 1000000
    local total = created + 1 -- +1 for the parent
    ev('branching factor: ' .. BRANCHING .. '; cascade wall time: ' .. string.format('%.1f', elapsed_ms) .. ' ms')
    ev('runs created by cascade: ' .. created .. ' (total ' .. total .. ' incl. parent)')
    ev('refused with bound error: ' .. refused)
    ev('max spawn depth reached: ' .. max_depth)
    if total ~= handles.sup.runs_max then
        return fail('recursion', 'cascade stopped at ' .. total .. ' runs, cap is ' .. handles.sup.runs_max)
    end
    ev('cascade terminated exactly at the total-run cap: no fork bomb')
    if refused == 0 then
        return fail('recursion', 'cascade ended with zero refusals: bound never engaged')
    end
    if wrong_error ~= 0 then
        return fail('recursion', wrong_error .. ' refusals carried the wrong error')
    end
    ev("every refusal was the explicit 'supervisor run bound exceeded' error")
    ev('documented: no explicit depth bound exists — depth is bounded only by the total-run cap')
    return pass()
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'default'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return fail('bootstrap', boot_err)
    end
    ev('scenario=' .. scenario)
    if scenario == 'default' then
        return scenario_default(handles)
    elseif scenario == 'attacker-n' then
        return scenario_attacker_n(handles)
    elseif scenario == 'recursive' then
        return scenario_recursive(handles)
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
