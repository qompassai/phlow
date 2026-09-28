-- task-41 driver: authority attenuation across delegation depth.
--
-- Recon probe: the design asks that a parent run's tool grants
-- attenuate down the delegation chain — the effective tool set of any
-- run is the intersection of its ancestors' grants, checked at every
-- delegation, with denials naming the missing grant. Scenarios: (a)
-- parent with tools {read} delegates to a child; the child uses {read}
-- (allowed); (b) the child requests {write} (denied — child authority
-- is a subset of the parent's); (c) the child delegates to a
-- grandchild requesting {write} (attenuation is transitive — denied at
-- depth 2).
--
-- This driver exercises the REAL delegation path — ai.harness
-- supervisor M.create / M.spawn_child with parent_id — and inspects
-- the REAL run tables for any authority/grant field, plus the policy
-- module's rule shape. It makes no network calls and spawns no
-- workers.
--
-- Honest result: the attenuation seam is ABSENT. Runs carry no
-- authority: the run table built in supervisor.create is
-- { id, parent_id, root_id, workflow, adapter, workspace, state,
-- attempt, created_ns, deadline_ns, budget, extensions, acceptance,
-- generation, children } — no tools grant, no authority set, no
-- privilege list. spawn_child copies the spec through M.create with
-- no intersection step. The policy module's rules are
-- supervisor-global (decision per tool request), never attached to a
-- run, so there is nothing for a child to inherit or intersect.
--
-- Fail-closed: if a run ever gains a tools/authority/grants field, or
-- spawn_child intersects it with the parent's, the probe reports
-- where="recon" instead.
--
-- Prints exactly one JSON verdict line to stdout and always exits 0.
-- Writes nothing outside GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function fail(where, how)
    return { id = 'task-41', outcome = 'fail', where = where, how = how, evidence = evidence }
end

---@return table? mods  -- { supervisor=..., types=..., policy=... }
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
    local ok_sup, supervisor = pcall(require, 'ai.harness.supervisor')
    if not ok_sup then
        return nil, 'require ai.harness.supervisor failed: ' .. tostring(supervisor)
    end
    local ok_ev, events = pcall(require, 'ai.harness.events')
    if not ok_ev then
        return nil, 'require ai.harness.events failed: ' .. tostring(events)
    end
    local ok_reg, registry = pcall(require, 'ai.harness.registry')
    if not ok_reg then
        return nil, 'require ai.harness.registry failed: ' .. tostring(registry)
    end
    local ok_pol, policy = pcall(require, 'ai.harness.policy')
    if not ok_pol then
        return nil, 'require ai.harness.policy failed: ' .. tostring(policy)
    end
    return { supervisor = supervisor, events = events, registry = registry, policy = policy }
end

---Sorted keys of a table, for shape inspection.
---@param t table
---@return string[]
local function sorted_keys(t)
    local keys = {}
    for k in pairs(t) do
        keys[#keys + 1] = tostring(k)
    end
    table.sort(keys)
    return keys
end

---True when any run key looks like an authority/grant field.
---@param keys string[]
---@return boolean
local function has_authority_field(keys)
    for _, key in ipairs(keys) do
        local lower = key:lower()
        if lower:find('tool', 1, true) or lower:find('grant', 1, true)
            or lower:find('authorit', 1, true) or lower:find('privilege', 1, true)
        then
            return true
        end
    end
    return false
end

---@param mods table
---@return table? sup
---@return string? err
local function new_supervisor(mods)
    local sink = mods.events.new_sink()
    local reg = mods.registry.new()
    return mods.supervisor.new({ registry = reg, sink = sink })
end

---@param mods table
---@param sup table
---@param spec table
---@return table? run
---@return string? err
local function create_run(mods, sup, spec)
    return mods.supervisor.create(sup, spec)
end

local function main()
    local mods, boot_err = bootstrap()
    if boot_err ~= nil then
        return fail('bootstrap', boot_err)
    end
    ev('ai.harness.supervisor/events/registry/policy loaded from DIVER_LUA_DIR')

    local sup, sup_err = new_supervisor(mods)
    if sup == nil then
        return fail('lua-driver', 'supervisor.new failed: ' .. tostring(sup_err))
    end

    -- V1 (default scenario): delegation works — parent -> child ->
    -- grandchild, parent_id linked at each depth.
    local parent, perr = create_run(mods, sup, {
        workflow = 'gauntlet-41',
        goal = 'probe authority attenuation',
        workspace = '/tmp/gauntlet-41',
    })
    if parent == nil then
        return fail('lua-driver', 'parent run create failed: ' .. tostring(perr))
    end
    local child, cerr = mods.supervisor.spawn_child(sup, parent.id, {
        workflow = 'gauntlet-41',
        goal = 'probe authority attenuation',
        workspace = '/tmp/gauntlet-41',
    })
    if child == nil then
        return fail('lua-driver', 'child spawn failed: ' .. tostring(cerr))
    end
    local grandchild, gerr = mods.supervisor.spawn_child(sup, child.id, {
        workflow = 'gauntlet-41',
        goal = 'probe authority attenuation',
        workspace = '/tmp/gauntlet-41',
    })
    if grandchild == nil then
        return fail('lua-driver', 'grandchild spawn failed: ' .. tostring(gerr))
    end
    ev('V1: delegation chain built — parent -> child -> grandchild, parent_id linked at each depth; the delegation path the design targets exists')

    -- A1 (adversarial): the child "requests {write}" while the parent
    -- "has {read}". Attenuation needs per-run grants to intersect —
    -- inspect the run tables for any authority field.
    local parent_keys = sorted_keys(parent)
    local child_keys = sorted_keys(child)
    local grandchild_keys = sorted_keys(grandchild)
    ev('A1: parent run keys: ' .. table.concat(parent_keys, ', '))
    ev('A1: child run keys: ' .. table.concat(child_keys, ', '))
    ev('A1: grandchild run keys: ' .. table.concat(grandchild_keys, ', '))
    if has_authority_field(parent_keys) or has_authority_field(child_keys)
        or has_authority_field(grandchild_keys)
    then
        return fail(
            'recon',
            'a run table now carries an authority/grant/tools field; the probe premise changed'
        )
    end
    ev('A1: no run carries an authority field — no tools grant, no grant set, no privilege list; '
        .. 'the child\'s "{write}" request has nothing to be denied against, and the "{read}" '
        .. 'default has nothing to be allowed by — the effective tool set is undefined, not attenuated')

    -- A2 (adversarial): attenuation must be transitive — checked at
    -- EVERY delegation. spawn_child copies the spec through M.create
    -- with no intersection step; policy rules are supervisor-global,
    -- never attached to a run.
    local pol = mods.policy.new({
        rules = {
            { risk = 'observe', decision = 'allow', tools = { 'fs.read' } },
        },
    })
    if pol == nil then
        return fail('lua-driver', 'policy.new failed')
    end
    ev('A2: a policy rule grants {fs.read} at the supervisor level — the policy module is a supervisor-global collaborator (supervisor.new opts.policy), never a per-run field')
    ev('A2: spawn_child sets spec.parent_id and calls M.create; M.create validates the spec, resolves the parent for root_id/children bookkeeping, and builds the run table — '
        .. 'no step reads the parent\'s authority (there is none) or intersects it with the child\'s')
    ev('A2: with no per-run grants, "child authority ⊆ parent authority" and transitivity to depth 2 are unexpressible — the denial that should name the missing grant cannot be produced')

    return fail(
        'seam',
        'seam absent: diver\'s harness has delegation (parent_id, spawn_child) but no per-run authority — '
            .. 'run tables carry no tools grant, no authority set, no privilege list, and spawn_child performs no intersection step; '
            .. 'policy rules are supervisor-global, never attached to a run. '
            .. 'The design\'s "effective tool set is the intersection of ancestors\' grants" has no grants to intersect, '
            .. 'so neither the {read}-allowed default, the {write}-denied adversarial case, nor transitive denial at depth 2 can be expressed. '
            .. 'Diver-owned finding: flagged, not fixed on gauntlet authority.'
    )
end

local ok, verdict = pcall(main)
if not ok then
    ev('lua error: ' .. tostring(verdict))
    verdict = fail('lua-driver', 'unhandled error: ' .. tostring(verdict))
end
-- verdict on the real stdout: in `nvim --headless -l`, Lua print() goes
-- to stderr, but the Rust runner parses the verdict from stdout.
io.stdout:write(vim.json.encode(verdict) .. '\n')
