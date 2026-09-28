-- task_71.lua -- gauntlet driver: delegation depth attribution.
--
-- The design asks for depth of the LIVE spawn tree attributed by the
-- supervisor from its own run ancestry — never self-reported by the
-- child. Pass criteria: depth is always supervisor-computed; no
-- child-asserted depth value is trusted anywhere; rejections name the
-- ancestry chain; the bound is a named constant, not a magic number.
--
-- Seam mapping (verified, not invented): diver's supervisor
-- (lua/ai/harness/supervisor.lua) keeps a run TREE — every run carries
-- `parent_id`/`root_id`, `M.create` links `parent.children`, and a
-- parent id must name an existing non-terminal run. But NOTHING in the
-- spawn path computes depth, enforces a depth bound, or verifies that a
-- spawn request's asserted parent is the true spawner:
--   * `M.create`/`M.spawn_child` never walk the parent_id chain;
--   * no depth constant exists (the only spawn bound is the total-run
--     cap `runs_max = 256`);
--   * `spec.extensions` passes through unread — a child asserting
--     `claimed_depth = 0` is never read, never rejected;
--   * any existing non-terminal run id is accepted as `parent_id` —
--     a foreign parent (not the true spawner) attaches silently.
-- The design's expected result here is the documented hole: depth is
-- computable from the supervisor's own tree, but the supervisor never
-- computes it, so the attribution-integrity attack (lie about where
-- you sit in the tree) has no check to defeat.
--
-- This driver plays the harness: it builds run trees with the REAL
-- supervisor (mock sink + mock registry; no adapters needed — runs are
-- never started) and walks `parent_id` ancestry with an explicit
-- iterative, capped walk (no recursion over attacker-controlled depth).
-- Every scenario fails at "seam" with mechanism evidence — 2
-- validation facets (chain depth is supervisor-computable 0/1/2; no
-- bound stops a deep chain), 2 adversarial (forged depth claim +
-- foreign parent accepted; no depth machinery anywhere in the spawn
-- path).
--
-- Scenarios via GAUNTLET_SCENARIO (default "chain-depth"):
--   chain-depth    V: parent->child->grandchild; the driver's ancestry
--                  walk yields depths 0/1/2 from the supervisor's own
--                  run table. The DATA for supervisor-side attribution
--                  exists; the supervisor itself never computes it.
--   depth-unbounded V: a linear chain of 30 spawns; every spawn
--                  succeeds. No `delegation_depth_exceeded` error, no
--                  named depth constant — depth is bounded only by the
--                  total-run cap.
--   forged-depth   A: the child asserts `extensions.claimed_depth = 0`
--                  (the lie) and, separately, a spawn names a FOREIGN
--                  parent (an unrelated live run, not the true
--                  spawner). Both succeed: the claim is never read,
--                  the foreign parent never verified.
--   no-depth-error A: source scan of the loaded supervisor.lua finds
--                  zero 'depth' mentions; a chain to the total-run cap
--                  is refused only with 'supervisor run bound
--                  exceeded' — no rejection ever names an ancestry
--                  chain.
--
-- Prints exactly one JSON verdict line to stdout and always exits 0.
-- Writes nothing outside GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64
local ANCESTRY_WALK_MAX = 1024
local DEEP_CHAIN = 30
local TASK_ID = 'task-71'

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

---Mock sink honoring the append shape the supervisor uses.
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

---@param handles table
---@param goal string
---@return table spec
local function base_spec(handles, goal)
    return {
        workflow = 'gauntlet-depth',
        goal = goal,
        workspace = vim.env.GAUNTLET_WORK_DIR,
        timeout_ms = 60000,
    }
end

---Iterative, capped walk of the supervisor's own parent_id chain.
---This is the attribution the design demands the SUPERVISOR perform;
---here the gauntlet performs it externally to show the data exists
---while the supervisor never computes it.
---@param handles table
---@param run_id string
---@return integer? depth
---@return string? err
local function ancestry_depth(handles, run_id)
    local seen = {}
    local current = run_id
    local depth = 0
    for _ = 1, ANCESTRY_WALK_MAX do
        if seen[current] then
            return nil, 'ancestry cycle at ' .. tostring(current)
        end
        seen[current] = true
        local run = handles.supervisor.get(handles.sup, current)
        if run == nil then
            return nil, 'unknown run ' .. tostring(current)
        end
        if run.parent_id == nil then
            return depth
        end
        depth = depth + 1
        current = run.parent_id
    end
    return nil, 'ancestry walk exceeded cap ' .. ANCESTRY_WALK_MAX
end

---@param handles table
---@return string? root_id
---@return string? err
local function make_root(handles)
    local run, err = handles.supervisor.create(handles.sup, base_spec(handles, 'depth probe root'))
    if run == nil then
        return nil, err
    end
    return run.id
end

---@param handles table
---@param parent_id string
---@param goal string
---@param extra table?
---@return table? child
---@return string? err
local function spawn(handles, parent_id, goal, extra)
    local spec = base_spec(handles, goal)
    if extra ~= nil then
        for k, v in pairs(extra) do
            spec[k] = v
        end
    end
    return handles.supervisor.spawn_child(handles.sup, parent_id, spec)
end

---chain-depth (V): the tree carries supervisor-computable depth 0/1/2.
---Documents that the DATA for supervisor-side attribution exists while
---the supervisor never computes it (no depth field on any run).
local function scenario_chain_depth(handles)
    local root_id, err = make_root(handles)
    if root_id == nil then
        return fail('spawn', 'root create failed: ' .. tostring(err))
    end
    local child, cerr = spawn(handles, root_id, 'depth probe child')
    if child == nil then
        return fail('spawn', 'child spawn failed: ' .. tostring(cerr))
    end
    local grandchild, gerr = spawn(handles, child.id, 'depth probe grandchild')
    if grandchild == nil then
        return fail('spawn', 'grandchild spawn failed: ' .. tostring(gerr))
    end
    local depths = {}
    for i, id in ipairs({ root_id, child.id, grandchild.id }) do
        local d, derr = ancestry_depth(handles, id)
        if d == nil then
            return fail('attribution', 'ancestry walk failed: ' .. tostring(derr))
        end
        depths[i] = d
    end
    ev('parent_id chain root->child->grandchild walks to depths '
        .. depths[1] .. '/' .. depths[2] .. '/' .. depths[3] .. ' (want 0/1/2)')
    if depths[1] ~= 0 or depths[2] ~= 1 or depths[3] ~= 2 then
        return fail('attribution', 'ancestry walk gave wrong depths')
    end
    local run = handles.supervisor.get(handles.sup, grandchild.id)
    if run.depth ~= nil then
        return fail('attribution', 'run carries a supervisor-computed depth field: unexpected')
    end
    ev('no run carries a supervisor-computed depth field: the walk above '
        .. 'was performed by the gauntlet, not by supervisor.create/spawn_child')
    return fail(
        'seam',
        'depth is computable from the supervisor\'s own parent_id tree (0/1/2 verified), '
            .. 'but supervisor.create/spawn_child never compute it, enforce no bound on it, '
            .. 'and store no depth on the run: supervisor-side depth attribution does not exist.'
    )
end

---depth-unbounded (V): a linear chain far past any sane delegation
---bound spawns without resistance — the only bound is the total-run cap.
local function scenario_depth_unbounded(handles)
    local parent_id, err = make_root(handles)
    if parent_id == nil then
        return fail('spawn', 'root create failed: ' .. tostring(err))
    end
    for i = 1, DEEP_CHAIN do
        local child, cerr = spawn(handles, parent_id, 'deep chain link ' .. i)
        if child == nil then
            return fail(
                'bound',
                'chain stopped at depth ' .. i .. ' with: ' .. tostring(cerr)
                    .. ' — a depth bound stopped it, contrary to the finding'
            )
        end
        parent_id = child.id
    end
    local depth, derr = ancestry_depth(handles, parent_id)
    if depth == nil then
        return fail('attribution', 'ancestry walk failed: ' .. tostring(derr))
    end
    ev('linear chain of ' .. DEEP_CHAIN .. ' spawns succeeded; tip depth = ' .. depth)
    ev('no spawn failed with a depth error: no delegation_depth_exceeded exists; '
        .. 'no named depth constant exists (only runs_max = ' .. handles.sup.runs_max .. ')')
    return fail(
        'seam',
        'a ' .. DEEP_CHAIN .. '-deep delegation chain spawns with zero resistance: '
            .. 'the design\'s "bound enforced at spawn time" is absent; the only spawn bound '
            .. 'is the total-run cap (runs_max = ' .. handles.sup.runs_max .. ').'
    )
end

---forged-depth (A): the attribution-integrity attack. The child asserts
---depth 0 in extensions (the lie) — never read, never rejected — and a
---spawn names a FOREIGN parent (an unrelated live run, not the true
---spawner) — accepted silently. The supervisor trusts the asserted
---parent_id; it verifies nothing about the spawner.
local function scenario_forged_depth(handles)
    local root_id, err = make_root(handles)
    if root_id == nil then
        return fail('spawn', 'root create failed: ' .. tostring(err))
    end
    local child, cerr = spawn(handles, root_id, 'forging child', {
        extensions = { claimed_depth = 0 },
    })
    if child == nil then
        return fail('spawn', 'child spawn failed: ' .. tostring(cerr))
    end
    local grandchild, gerr = spawn(handles, child.id, 'grandchild of liar', {
        extensions = { claimed_depth = 0 },
    })
    if grandchild == nil then
        return fail('spawn', 'grandchild spawn failed: ' .. tostring(gerr))
    end
    local got = handles.supervisor.get(handles.sup, grandchild.id)
    local true_depth, derr = ancestry_depth(handles, grandchild.id)
    if true_depth == nil then
        return fail('attribution', 'ancestry walk failed: ' .. tostring(derr))
    end
    ev('child asserted extensions.claimed_depth = 0; supervisor-computed ancestry depth = '
        .. true_depth)
    ev('the claim sits unread in run.extensions: create/validate_run_spec never inspect it')
    if got.extensions.claimed_depth ~= 0 then
        return fail('spawn', 'extensions did not round-trip the forged claim')
    end
    -- Foreign parent: an unrelated live run adopts the child. The
    -- "spawner" here is the driver acting for `child`, but the asserted
    -- parent is a run the spawner has no relationship to.
    local stranger, serr = make_root(handles)
    if stranger == nil then
        return fail('spawn', 'stranger root create failed: ' .. tostring(serr))
    end
    local spec = base_spec(handles, 'adopted by a stranger')
    spec.parent_id = stranger
    local adopted, aerr = handles.supervisor.create(handles.sup, spec)
    if adopted == nil then
        return fail(
            'attribution',
            'foreign parent_id was REJECTED (' .. tostring(aerr) .. '): '
                .. 'the supervisor verifies the spawner after all — finding refuted'
        )
    end
    ev('spawn asserting parent_id = unrelated live run ' .. stranger .. ' SUCCEEDED: '
        .. 'adopted run ' .. adopted.id)
    ev('create() checks only "parent exists and is non-terminal" — it never verifies '
        .. 'that the asserted parent is the true spawner')
    return fail(
        'seam',
        'attribution is caller-asserted, never supervisor-verified: a forged depth-0 claim '
            .. 'passes through extensions unread (true depth ' .. true_depth .. '), and a '
            .. 'foreign parent_id is accepted silently. The design\'s "supervisor recomputes '
            .. 'depth from its own run tree and rejects the lie" has no implementation.'
    )
end

---no-depth-error (A): sweep for depth machinery in the spawn path.
---The loaded supervisor.lua is source-scanned for 'depth' (zero hits),
---and a chain to the cap is refused only by the total-run bound — no
---rejection ever names an ancestry chain.
local function scenario_no_depth_error(handles)
    local found = vim.api.nvim_get_runtime_file('lua/ai/harness/supervisor.lua', false)
    if #found == 0 then
        return fail('seam', 'could not resolve the loaded supervisor.lua path')
    end
    local text = table.concat(vim.fn.readfile(found[1]), '\n'):lower()
    local hits = 0
    for _ in text:gmatch('depth') do
        hits = hits + 1
    end
    ev('source scan of loaded supervisor.lua (' .. found[1] .. '): "depth" mentions = ' .. hits)
    if hits ~= 0 then
        return fail(
            'seam',
            'supervisor.lua mentions depth ' .. hits .. 'x: depth machinery may exist — finding refuted'
        )
    end
    local parent_id, err = make_root(handles)
    if parent_id == nil then
        return fail('spawn', 'root create failed: ' .. tostring(err))
    end
    local refused_with = nil
    local made = 0
    while true do
        local child, cerr = spawn(handles, parent_id, 'cap probe ' .. made)
        if child == nil then
            refused_with = tostring(cerr)
            break
        end
        made = made + 1
        parent_id = child.id
    end
    ev('chain spawns before refusal: ' .. made .. '; refusal error: ' .. tostring(refused_with))
    if refused_with ~= 'supervisor run bound exceeded' then
        return fail('bound', 'unexpected refusal error: ' .. tostring(refused_with))
    end
    ev('no refusal named an ancestry chain; no delegation_depth_exceeded string exists')
    return fail(
        'seam',
        'the spawn path contains no depth machinery at all (0 "depth" mentions in '
            .. 'supervisor.lua); the only refusal is the total-run cap. The design\'s typed '
            .. 'delegation_depth_exceeded naming the full ancestry chain is absent.'
    )
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'chain-depth'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return fail('bootstrap', boot_err)
    end
    ev('scenario=' .. scenario)
    if scenario == 'chain-depth' then
        return scenario_chain_depth(handles)
    elseif scenario == 'depth-unbounded' then
        return scenario_depth_unbounded(handles)
    elseif scenario == 'forged-depth' then
        return scenario_forged_depth(handles)
    elseif scenario == 'no-depth-error' then
        return scenario_no_depth_error(handles)
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
