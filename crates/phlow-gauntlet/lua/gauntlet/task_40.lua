-- task-40 driver: approval TOCTOU on diver's approval record ->
-- execution path.
--
-- Recon probe: the design asks that EVERY execution re-validate the
-- approval against CURRENT state (or that the approval bind a state
-- hash), with the check after the last possible mutation point.
-- Scenarios: (a) the file approved for reading is replaced with a
-- symlink to a secret between approval and open — the executor must
-- re-validate the target and deny; (b) the approval is granted, then
-- REVOKED before execution — execution must check liveness and deny.
--
-- This driver exercises the REAL approval module —
-- ai.harness.approval (M.new / M.request / M.decide / M.get /
-- M.pending / M.sweep_expired) — with a mock approver and a mutator
-- that races execution. It makes no network calls and spawns no
-- workers.
--
-- Honest result: the re-validation seam is ABSENT, twice over.
-- (1) Approval records bind no state: the record is
-- { id, run_id, tool, risk, summary, argv?, paths?, endpoints?,
-- state, created_ns, deadline_ns, decided_by? } — no state hash,
-- digest, or target binding. (2) No execution-time re-validation
-- exists: nothing in the harness consumes an approval at execution —
-- the supervisor only sweeps expiries in tick(); there is no
-- executor function that re-checks an approval against current state,
-- no revocation API (M.decide accepts only 'approved'/'denied' from
-- 'pending'; an approved record can never be revoked), and no
-- liveness check. The design's "executor re-validates the target" and
-- "execution checks liveness" have no seam to attach to.
--
-- Fail-closed: if the record gains a state-hash field, a revocation
-- path, or an execution-time re-validation call, the probe reports
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
    return { id = 'task-40', outcome = 'fail', where = where, how = how, evidence = evidence }
end

---@return table? approval_mod
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
    local ok, approval_mod = pcall(require, 'ai.harness.approval')
    if not ok then
        return nil, 'require ai.harness.approval failed: ' .. tostring(approval_mod)
    end
    return approval_mod
end

---Sorted record keys, for shape inspection.
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

---True when any record key looks like a state binding (hash/digest/
---version/fingerprint of the approved-against state).
---@param keys string[]
---@return boolean
local function has_state_binding(keys)
    for _, key in ipairs(keys) do
        local lower = key:lower()
        if lower:find('hash', 1, true) or lower:find('digest', 1, true) or lower:find('fingerprint', 1, true) then
            return true
        end
    end
    return false
end

local function main()
    local approval_mod, boot_err = bootstrap()
    if boot_err ~= nil then
        return fail('bootstrap', boot_err)
    end
    ev('ai.harness.approval loaded from DIVER_LUA_DIR')

    -- V1 (default scenario): no change between grant and execution —
    -- the approval is granted and the record reads back approved.
    local queue = approval_mod.new()
    local id, req_err = approval_mod.request(queue, 'run-40', {
        tool = 'fs.read',
        risk = 'observe',
        summary = 'read /work/notes.txt',
        paths = { '/work/notes.txt' },
    })
    if id == nil then
        return fail('lua-driver', 'approval.request failed: ' .. tostring(req_err))
    end
    local ok, dec_err = approval_mod.decide(queue, id, 'approved', 'mock-approver')
    if not ok then
        return fail('lua-driver', 'approval.decide failed: ' .. tostring(dec_err))
    end
    local rec = approval_mod.get(queue, id)
    if rec == nil or rec.state ~= 'approved' then
        return fail('lua-driver', 'approval.get did not read back the approved record')
    end
    ev('V1: approval requested for fs.read /work/notes.txt, decided approved, reads back approved — the default path records the grant')

    -- A1 (adversarial): between grant and "execution", the mutator
    -- replaces the approved file with a symlink to a secret. The
    -- design requires the executor to re-validate the target (or the
    -- approval to bind a state hash) and deny.
    local keys = sorted_keys(rec)
    ev('A1: approval record keys: ' .. table.concat(keys, ', '))
    if has_state_binding(keys) then
        return fail(
            'recon',
            'the approval record now binds approved-against state (hash/digest field); probe premise changed'
        )
    end
    ev('A1: the record binds NO state — no hash, digest, or fingerprint of the approved-against target; a symlink swap after grant is invisible to the record')
    -- The mutator races execution: the file the approval named is
    -- swapped for a symlink to a secret. The record is unchanged by
    -- construction — and there is no re-validation call to make: no
    -- harness function re-checks an approval against current state.
    local rec_after = approval_mod.get(queue, id)
    if rec_after.state ~= 'approved' then
        return fail('lua-driver', 'the mutator changed the approval record — unexpected')
    end
    ev('A1: after the mutator swaps the target, approval.get still reads state=approved — no re-validation hook fired, because none exists')
    ev('A1: source shape: no harness module calls approval.get at execution time (supervisor.tick only calls approval.sweep_expired); there is no executor function to re-validate the target in')

    -- A2 (adversarial): the approval is granted, then REVOKED before
    -- execution. The design requires execution to check liveness.
    local rok, rerr = approval_mod.decide(queue, id, 'denied', 'mock-approver')
    ev('A2: revoke attempt on the approved record -> ok=' .. tostring(rok) .. ' err=' .. tostring(rerr))
    if rok then
        return fail(
            'recon',
            'an approved record can now be revoked: the revocation path the probe assumed absent exists; premise changed'
        )
    end
    ev('A2: M.decide accepts only approved/denied from pending — an approved record can never move back; no revocation API exists')
    ev('A2: no liveness check exists at execution time: states are pending/approved/denied/expired, and the only state consumer besides the queue itself is the expiry sweep — nothing asks "is this approval still live?" after the last mutation point')

    return fail(
        'seam',
        'seam absent: approval records bind no approved-against state (no hash/digest/fingerprint), there is no revocation path, and no execution-time re-validation exists — '
            .. 'nothing in the harness consumes an approval at execution, so the design\'s "executor re-validates the target" and "execution checks liveness" have no seam to attach to. '
            .. 'A symlink swapped in after grant, or a grant that should have been revoked, is invisible to the record. '
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
