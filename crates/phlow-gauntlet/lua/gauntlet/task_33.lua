-- task-33 driver: checkpoint durability for diver's ai.harness.store.
--
-- Recon probe: the design asks for checkpoint durability across SUPERVISOR
-- death (SIGKILL): the run checkpoints at step boundaries, the supervisor
-- dies, a new supervisor restores from the last complete checkpoint and
-- completes the run. Kill-during-write must fall back to the last COMPLETE
-- checkpoint (atomic write or write-then-rename), and a checkpoint schema
-- change must be rejected with a typed error, never resumed corrupt.
--
-- This driver exercises the REAL checkpoint module — ai.harness.store
-- (M.checkpoint / M.get_checkpoint / M.save_run) — with real run tables.
-- It makes no network calls and spawns no workers.
--
-- Honest result: the checkpoint API works, but persistence is IN-MEMORY
-- ONLY. store.lua's own header documents it: "Phase 1 store is in-memory
-- with deep copies at the boundary ... SQLite backing is Phase 5 work."
-- A SIGKILLed supervisor takes its store with it: a new supervisor
-- process constructs a fresh M.new() — empty tables — so there is no
-- durable checkpoint to restore. Kill-during-write atomicity is vacuous
-- (Lua table assignment cannot tear, but nothing reaches disk: the task
-- scratch dir stays empty after checkpointing), and checkpoint records
-- are { label, at_ns, state } with NO schema version, so get_checkpoint
-- performs no schema validation and a schema change could not be
-- rejected with a typed error. The supervisor accepts the store as a
-- collaborator (supervisor.new({ store = ... })) but never calls
-- checkpoint itself (source scan of supervisor.lua: the only 'store'
-- reference is the constructor assignment).
--
-- Fail-closed: if checkpoints ever become durable (a checkpoint file
-- appears in GAUNTLET_WORK_DIR, or get_checkpoint validates a schema
-- version), the driver reports where="recon" (premise changed) instead.
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
    return { id = 'task-33', outcome = 'fail', where = where, how = how, evidence = evidence }
end

---@return table? store_mod
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
    local ok, store_mod = pcall(require, 'ai.harness.store')
    if not ok then
        return nil, 'require ai.harness.store failed: ' .. tostring(store_mod)
    end
    return store_mod
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

---List entries in a directory (empty table when missing/unreadable).
---@param dir string
---@return string[]
local function dir_entries(dir)
    if vim.fn.isdirectory(dir) ~= 1 then
        return {}
    end
    local ok, entries = pcall(vim.fn.readdir, dir)
    if not ok or type(entries) ~= 'table' then
        return {}
    end
    return entries
end

local function main()
    local store_mod, boot_err = bootstrap()
    if boot_err ~= nil then
        return fail('bootstrap', boot_err)
    end
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    ev('ai.harness.store loaded from DIVER_LUA_DIR')

    -- V1: checkpoint round-trip inside one live store. A run checkpoints
    -- at two step boundaries; both checkpoints restore with matching
    -- state. This is the half of the seam that exists.
    local store = store_mod.new()
    local run = { id = 'run-33', state = 'running', step = 1 }
    local ok, err = store_mod.save_run(store, run)
    if not ok then
        return fail('lua-driver', 'save_run failed: ' .. tostring(err))
    end
    ok, err = store_mod.checkpoint(store, 'run-33', 'step-1')
    if not ok then
        return fail('lua-driver', 'checkpoint step-1 failed: ' .. tostring(err))
    end
    run.step = 2
    ok, err = store_mod.save_run(store, run)
    if not ok then
        return fail('lua-driver', 'save_run (step 2) failed: ' .. tostring(err))
    end
    ok, err = store_mod.checkpoint(store, 'run-33', 'step-2')
    if not ok then
        return fail('lua-driver', 'checkpoint step-2 failed: ' .. tostring(err))
    end
    local cp1 = store_mod.get_checkpoint(store, 'run-33', 'step-1')
    local cp2 = store_mod.get_checkpoint(store, 'run-33', 'step-2')
    if cp1 == nil or cp2 == nil then
        return fail('lua-driver', 'get_checkpoint lost a checkpoint inside the live store')
    end
    if cp1.state.step ~= 1 or cp2.state.step ~= 2 then
        return fail(
            'lua-driver',
            'checkpoint state mismatch: step-1=' .. tostring(cp1.state.step) .. ' step-2=' .. tostring(cp2.state.step)
        )
    end
    ev('V1: checkpoint round-trips in-memory — step-1 and step-2 restore with matching state')
    ev('V1: checkpoint record keys: ' .. table.concat(sorted_keys(cp1), ', '))

    -- A2 (run before the kill simulation so the live store is intact):
    -- checkpoint records carry no schema version. Inspect the keys.
    local has_version = false
    for _, key in ipairs(sorted_keys(cp1)) do
        local lower = key:lower()
        if lower:find('version', 1, true) or lower:find('schema', 1, true) then
            has_version = true
        end
    end
    if has_version then
        return fail(
            'recon',
            'checkpoint records now carry a schema version; probe premise changed'
        )
    end
    ev('A2: checkpoint record is { label, at_ns, state } — no schema version field')
    ev('A2: get_checkpoint performs no schema validation: a schema-changed checkpoint could not be rejected with a typed error')

    -- A1: kill-during-write atomicity is vacuous — and durability is
    -- zero. Checkpointing writes no disk artifact: the task scratch dir
    -- stays empty. If a checkpoint file ever appears, the premise
    -- changed (durability exists) and the probe says so.
    local entries = dir_entries(work_dir)
    if #entries > 0 then
        return fail(
            'recon',
            'checkpointing wrote disk artifacts (' .. table.concat(entries, ', ') .. '); probe premise changed'
        )
    end
    ev('A1: after 2 checkpoints, GAUNTLET_WORK_DIR holds 0 files — checkpoint writes nothing to disk')
    ev('A1: kill-during-write atomicity is vacuous (in-memory table assignment cannot tear); durability is zero')

    -- V2: the design's default scenario — the supervisor dies (SIGKILL)
    -- and a NEW supervisor restores. A new supervisor process constructs
    -- a fresh store (M.new() = empty tables): the post-mortem view.
    -- get_checkpoint on the fresh store returns nil — restore is
    -- impossible because nothing survived the supervisor.
    local post_mortem = store_mod.new()
    local restored = store_mod.get_checkpoint(post_mortem, 'run-33', 'step-2')
    if restored ~= nil then
        return fail(
            'recon',
            'a fresh store restored a checkpoint from the dead supervisor; probe premise changed'
        )
    end
    ev('V2: post-mortem store (fresh M.new(), as a new supervisor process constructs) -> get_checkpoint returns nil')
    ev('V2: SIGKILL takes the checkpoints with the supervisor: no durable checkpoint exists to restore, so the run cannot be completed from a checkpoint')

    ev('module contract: store.lua header documents "Phase 1 store is in-memory ... SQLite backing is Phase 5 work"')
    ev('wiring: supervisor.new accepts store as a collaborator but never calls checkpoint (source scan: only the constructor assignment references store)')
    return fail(
        'seam',
        'seam absent: checkpoint persistence is in-memory only — a SIGKILLed supervisor loses every checkpoint, so the design\'s restore-and-complete scenario has no durable seam; '
            .. 'no disk writes exist (kill-during-write atomicity is vacuous, durability is zero); '
            .. 'checkpoint records carry no schema version, so a schema change could not be rejected with a typed error. '
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
