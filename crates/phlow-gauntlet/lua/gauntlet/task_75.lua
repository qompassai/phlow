-- task_75.lua -- gauntlet driver: delegation cycle detection.
--
-- The design asks that cycles in the delegation graph be rejected with
-- a typed error — a depth bound alone cannot catch them (a 2-node
-- cycle never gets deep; it spins). Pass criteria: every spawn checks
-- membership in the requester's ancestry set; rejections name the full
-- cycle; no deadlock or livelock under cyclic requests; the check is
-- O(depth), documented.
--
-- Seam mapping (verified, not invented): diver's spawn path
-- (supervisor.spawn_child -> supervisor.create, lua/ai/harness/) has
-- NO ancestry-membership check. Strict graph cycles are structurally
-- unrepresentable — a parent_id must name an already-existing run and
-- ids are minted fresh at create(), so no run can be its own ancestor
-- — but the property holds by construction, not by check: no per-spawn
-- walk exists, no `delegation_cycle` error exists (zero 'cycle'
-- mentions in supervisor.lua), and the design's "escalation loop" (a
-- child delegating back UP to an ancestor under a different task name
-- — detected by identity, not by name) is ACCEPTED silently. A spawn
-- may also name any unrelated live run as parent_id (caller-asserted,
-- never verified against the spawner), so the ancestry set the design
-- wants checked is never even consulted.
-- The design's expected result here is the documented hole. Diver-owned
-- (flagged, never fixed on gauntlet authority).
--
-- This driver plays the harness with the REAL supervisor (mock sink +
-- mock registry; runs are never started) and runs the O(depth)
-- ancestry walk the design demands — iterative, capped, no recursion
-- over attacker-controlled depth — externally, under headless live
-- nvim. Every scenario fails at "seam" with mechanism evidence — 2
-- validation facets (a linear chain works with sound ancestry walks;
-- the walk itself is validated on a branching tree), 2 adversarial
-- (the escalation loop is accepted; no cycle machinery exists
-- anywhere in the spawn path).
--
-- Scenarios via GAUNTLET_SCENARIO (default "linear-chain"):
--   linear-chain      V: chain of 5 via spawn_child; the capped walk
--                       yields depths 0..4; no false rejections.
--   walk-sound        V: a branching tree (root, 2 children, 3
--                       grandchildren); the walk reports exact ancestry
--                       sets for every node — the O(depth) check the
--                       design wants, demonstrated externally.
--   escalation-accepted A: B (child of A) spawns C with parent_id = A
--                       (an ancestor, not the direct spawner) under a
--                       different workflow name — accepted silently.
--                       The design wants this rejected by identity.
--   no-cycle-machinery A: source scan of the loaded supervisor.lua:
--                       zero 'cycle' mentions, no delegation_cycle
--                       string; plus a spawn naming an unrelated live
--                       run as parent is accepted — the ancestry set is
--                       never consulted at spawn.
--
-- Prints exactly one JSON verdict line to stdout and always exits 0.
-- Writes nothing outside GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64
local ANCESTRY_WALK_MAX = 1024
local CHAIN_LEN = 5
local TASK_ID = 'task-75'

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
---@param workflow string?
---@return table spec
local function base_spec(goal, workflow)
    return {
        workflow = workflow or 'gauntlet-cycle',
        goal = goal,
        workspace = vim.env.GAUNTLET_WORK_DIR,
        timeout_ms = 60000,
    }
end

---The O(depth) ancestry walk the design demands at every spawn:
---iterative, capped, returns the full ancestor id set. The gauntlet
---runs it externally under live nvim; the supervisor never runs it.
---The returned set holds every id on the parent_id chain EXCLUDING
---the starting node itself (a node is not its own ancestor).
---@param handles table
---@param run_id string
---@return table? ancestors set of ancestor ids
---@return string? err
local function ancestry_set(handles, run_id)
    local seen = {}
    local current = run_id
    for _ = 1, ANCESTRY_WALK_MAX do
        local run = handles.supervisor.get(handles.sup, current)
        if run == nil then
            return nil, 'unknown run ' .. tostring(current)
        end
        if run.parent_id == nil then
            return seen -- reached the root: `seen` is exactly the ancestor set
        end
        current = run.parent_id
        if seen[current] then
            return nil, 'ancestry cycle at ' .. tostring(current)
        end
        seen[current] = true
    end
    return nil, 'ancestry walk exceeded cap ' .. ANCESTRY_WALK_MAX
end

---@param handles table
---@param parent_id string?
---@param goal string
---@param workflow string?
---@return table? run
---@return string? err
local function spawn(handles, parent_id, goal, workflow)
    local spec = base_spec(goal, workflow)
    if parent_id == nil then
        return handles.supervisor.create(handles.sup, spec)
    end
    return handles.supervisor.spawn_child(handles.sup, parent_id, spec)
end

---@param set table
---@return integer count
local function set_size(set)
    local n = 0
    for _ in pairs(set) do
        n = n + 1
    end
    return n
end

---linear-chain (V): the design's default — a linear chain works; the
---walk reports depths 0..4 with no false cycle positives.
local function scenario_linear_chain(handles)
    local ids = {}
    local parent_id = nil
    for i = 1, CHAIN_LEN do
        local run, err = spawn(handles, parent_id, 'chain link ' .. i)
        if run == nil then
            return fail('spawn', 'chain link ' .. i .. ' failed: ' .. tostring(err))
        end
        ids[#ids + 1] = run.id
        parent_id = run.id
    end
    for i, id in ipairs(ids) do
        local ancestors, aerr = ancestry_set(handles, id)
        if ancestors == nil then
            return fail('walk', 'ancestry walk failed at link ' .. i .. ': ' .. tostring(aerr))
        end
        if set_size(ancestors) ~= i - 1 then
            return fail(
                'walk',
                'link ' .. i .. ' has ' .. set_size(ancestors) .. ' ancestors, want ' .. (i - 1)
            )
        end
    end
    ev('linear chain of ' .. CHAIN_LEN .. ' spawns: ancestry sets are 0,1,2,3,4 — no false positives')
    return fail(
        'seam',
        'the chain works and the O(depth) ancestry walk is sound (0..4 verified externally), '
            .. 'but the walk is the GAUNTLET\'s, not the supervisor\'s: no spawn ever checks '
            .. 'membership in the requester\'s ancestry set, so the design\'s per-spawn cycle '
            .. 'check does not exist.'
    )
end

---walk-sound (V): validate the walk itself on a branching tree — the
---exact machinery the design wants at every spawn, shown correct.
local function scenario_walk_sound(handles)
    local root, rerr = spawn(handles, nil, 'branch root')
    if root == nil then
        return fail('spawn', 'root failed: ' .. tostring(rerr))
    end
    local left, lerr = spawn(handles, root.id, 'left child')
    if left == nil then
        return fail('spawn', 'left child failed: ' .. tostring(lerr))
    end
    local right, rerr2 = spawn(handles, root.id, 'right child')
    if right == nil then
        return fail('spawn', 'right child failed: ' .. tostring(rerr2))
    end
    local ll, llerr = spawn(handles, left.id, 'left-left grandchild')
    if ll == nil then
        return fail('spawn', 'left-left failed: ' .. tostring(llerr))
    end
    local lr, lrerr = spawn(handles, left.id, 'left-right grandchild')
    if lr == nil then
        return fail('spawn', 'left-right failed: ' .. tostring(lrerr))
    end
    local rchild, rcerr = spawn(handles, right.id, 'right grandchild')
    if rchild == nil then
        return fail('spawn', 'right grandchild failed: ' .. tostring(rcerr))
    end
    local cases = {
        { id = root.id, want = {} },
        { id = left.id, want = { root.id } },
        { id = right.id, want = { root.id } },
        { id = ll.id, want = { root.id, left.id } },
        { id = lr.id, want = { root.id, left.id } },
        { id = rchild.id, want = { root.id, right.id } },
    }
    for _, case in ipairs(cases) do
        local ancestors, aerr = ancestry_set(handles, case.id)
        if ancestors == nil then
            return fail('walk', 'walk failed: ' .. tostring(aerr))
        end
        if set_size(ancestors) ~= #case.want then
            return fail('walk', 'wrong ancestor count for ' .. case.id)
        end
        for _, want_id in ipairs(case.want) do
            if not ancestors[want_id] then
                return fail('walk', 'missing ancestor ' .. want_id .. ' for ' .. case.id)
            end
        end
    end
    ev('branching tree: 6 nodes, every ancestry set exact (root={}, children={root}, '
        .. 'grandchildren={root,parent})')
    ev('the walk is O(depth), iterative, capped at ' .. ANCESTRY_WALK_MAX .. ' — the check the '
        .. 'design demands, validated under live nvim; the supervisor never runs it')
    return fail(
        'seam',
        'the ancestry walk is sound on a branching tree, but it exists only in the '
            .. 'gauntlet: supervisor.create/spawn_child perform no ancestry-membership check, '
            .. 'so a cyclic or upward delegation request meets no check at all.'
    )
end

---escalation-accepted (A): the design's "escalation loop" — a child
---delegating back UP to an ancestor under a different task name,
---detected by identity not by name. Diver accepts it silently.
local function scenario_escalation_accepted(handles)
    local agent_a, aerr = spawn(handles, nil, 'agent A root')
    if agent_a == nil then
        return fail('spawn', 'agent A failed: ' .. tostring(aerr))
    end
    local agent_b, berr = spawn(handles, agent_a.id, 'agent B (child of A)')
    if agent_b == nil then
        return fail('spawn', 'agent B failed: ' .. tostring(berr))
    end
    -- B delegates back up to its ancestor A under a DIFFERENT task
    -- name. By identity this is an upward delegation loop; the design
    -- wants `delegation_cycle` naming the cycle.
    local spec = base_spec('escalated task', 'totally-different-workflow')
    spec.parent_id = agent_a.id
    local agent_c, cerr = handles.supervisor.create(handles.sup, spec)
    if agent_c == nil then
        return fail(
            'cycle',
            'upward delegation was REJECTED (' .. tostring(cerr) .. '): '
                .. 'the check exists — finding refuted'
        )
    end
    local ancestors, aerr2 = ancestry_set(handles, agent_c.id)
    if ancestors == nil then
        return fail('walk', 'walk failed: ' .. tostring(aerr2))
    end
    ev('B (child of A) delegated back up: new run ' .. agent_c.id .. ' has parent '
        .. agent_a.id .. ' (the ANCESTOR) under workflow "totally-different-workflow"')
    ev('accepted silently: no delegation_cycle error, no ancestry-membership check — '
        .. 'identity-based detection does not exist')
    if not ancestors[agent_a.id] then
        return fail('walk', 'ancestor not in ancestry set: unexpected')
    end
    return fail(
        'seam',
        'the escalation loop is accepted: a child delegated back up to its ancestor under '
            .. 'a different workflow name with no rejection. The design\'s identity-based '
            .. 'delegation_cycle detection has no implementation in the spawn path.'
    )
end

---no-cycle-machinery (A): the spawn path contains no cycle machinery
---at all — and the ancestry set is never consulted, since any
---unrelated live run is accepted as parent_id.
local function scenario_no_cycle_machinery(handles)
    local found = vim.api.nvim_get_runtime_file('lua/ai/harness/supervisor.lua', false)
    if #found == 0 then
        return fail('seam', 'could not resolve the loaded supervisor.lua path')
    end
    local text = table.concat(vim.fn.readfile(found[1]), '\n'):lower()
    local hits = 0
    for _ in text:gmatch('cycle') do
        hits = hits + 1
    end
    local _, lifecycle_hits = text:gsub('lifecycle', '')
    ev('source scan of loaded supervisor.lua: "cycle" substring mentions = ' .. hits
        .. ' ("lifecycle" occurrences = ' .. lifecycle_hits .. ')')
    if hits ~= lifecycle_hits then
        return fail(
            'seam',
            '"cycle" appears outside the word "lifecycle": cycle machinery may exist — '
                .. 'finding refuted'
        )
    end
    ev('the only "cycle" mention is inside the word "lifecycle" (header comment): '
        .. 'no cycle DETECTION machinery — no ancestry walk, no membership check')
    if text:find('delegation_cycle', 1, true) ~= nil then
        return fail('seam', 'delegation_cycle string exists: the typed error exists — finding refuted')
    end
    ev('no delegation_cycle error string anywhere in the spawn path')
    -- The ancestry set is never consulted: an unrelated live run is
    -- accepted as parent without any membership check.
    local root_a, aerr = spawn(handles, nil, 'unrelated root A')
    if root_a == nil then
        return fail('spawn', 'root A failed: ' .. tostring(aerr))
    end
    local root_b, berr = spawn(handles, nil, 'unrelated root B')
    if root_b == nil then
        return fail('spawn', 'root B failed: ' .. tostring(berr))
    end
    local spec = base_spec('attached anywhere')
    spec.parent_id = root_b.id
    local adopted, derr = handles.supervisor.create(handles.sup, spec)
    if adopted == nil then
        return fail(
            'spawn',
            'unrelated parent_id REJECTED (' .. tostring(derr) .. '): '
                .. 'some check exists — finding refuted'
        )
    end
    ev('spawn naming unrelated live run ' .. root_b.id .. ' as parent: accepted as '
        .. adopted.id .. ' — parent_id is caller-asserted, never checked against any ancestry set')
    ev('strict cycles are structurally unrepresentable (parent must pre-exist; ids are fresh), '
        .. 'but that is construction, not a check — and the design\'s upward/escalation cases '
        .. 'ARE representable and unrejected')
    return fail(
        'seam',
        'no cycle machinery exists in the spawn path (the only "cycle" substring is inside '
            .. 'the word "lifecycle" in the header comment; no delegation_cycle '
            .. 'error): no per-spawn ancestry-membership check, no O(depth) walk, no typed '
            .. 'rejection naming a cycle. Strict cycles are unrepresentable by construction '
            .. '(fresh ids), but the design\'s checkable cases — escalation loops, upward '
            .. 'delegation — meet no check.'
    )
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'linear-chain'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return fail('bootstrap', boot_err)
    end
    ev('scenario=' .. scenario)
    if scenario == 'linear-chain' then
        return scenario_linear_chain(handles)
    elseif scenario == 'walk-sound' then
        return scenario_walk_sound(handles)
    elseif scenario == 'escalation-accepted' then
        return scenario_escalation_accepted(handles)
    elseif scenario == 'no-cycle-machinery' then
        return scenario_no_cycle_machinery(handles)
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
