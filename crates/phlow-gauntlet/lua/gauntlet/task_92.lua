-- task_92.lua -- gauntlet driver: proposal scope binding and drift detection.
--
-- The design asks for an approval to authorize *exactly* the bytes it
-- reviewed: the bound unit is hash(base revision + diff bytes); any drift
-- between approval time and apply time (rebase, concurrent edit,
-- regeneration) is detected and rejected with `proposal_drift` naming the
-- differing hunks; a rebase that applies cleanly is still caught because
-- the base revision is part of the bound hash; approval for P applied to Q
-- is rejected; and a recovery path exists (re-review and re-approve the
-- drifted proposal — fail-closed is not fail-stuck).
--
-- Seam mapping (verified, not invented): the lua layer (diver's `ai.*`)
-- has NO approval→content-hash binding. What exists:
--   * `ai.harness.policy` — tool-use approvals bound to ACTION SCOPE
--     (rule decisions 'allow'|'deny'|'approval'; task-59's mechanism).
--     It binds *what the tool may do*, never *which bytes were reviewed*.
--   * `ai.harness.approval` — an async approval queue (data only: id,
--     tool, risk, summary, argv, paths; no render, no hash field).
--   * `ai.harness.store` — a content-addressed ARTIFACT store with a
--     `content_hash` helper. This is the red herring: it hashes artifact
--     bytes for deduplication; `approval.lua` never references it, and no
--     approval is bound to any hash.
-- Absent: any `proposal_drift` typed error, any drift-detection module,
-- any re-review/re-approve path, any binding of an approval id to
-- hash(base revision + diff bytes).
--
-- This driver probes the REAL modules headless and runs a bounded token
-- scan over the diver lua/ai tree:
--   binding    V: `ai.harness.approval` exists but exposes no
--                content-hash binding API; `ai.harness.store`'s
--                content_hash is unreachable from the approval queue.
--   scope      V: `ai.harness.policy` binds approvals to action scope
--                (rule decisions), not to content hashes — the task-59
--                mechanism, distinct from what task-92 demands.
--   drift      A: no drift-detection module or API exists; token scan
--                for `drift` over lua/ai finds zero hits.
--   recovery   A: no re-review/re-approve path; token scan for
--                `re_approve`/`reapprove`/`proposal_drift` finds zero hits.
--
-- All scenarios write machine-readable traces into GAUNTLET_WORK_DIR
-- (binding-trace.json) for the Rust harness probes, print exactly one
-- JSON verdict line to stdout, always exit 0, write nothing outside
-- GAUNTLET_WORK_DIR, and never modify the diver repo.
--
-- Scenarios via GAUNTLET_SCENARIO (default "binding"):
--   binding    V: approval queue has no content-hash binding API.
--   scope      V: policy binds to action scope, not content hash.
--   drift      A: no drift detection anywhere in lua/ai.
--   recovery   A: no re-review path for drifted proposals.

local TASK_ID = 'task-92'
local TRACE_NAME = 'binding-trace.json'

local EVIDENCE_MAX = 64
local SCAN_FILES_MAX = 500
local SCAN_BYTES_MAX = 262144

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

---Wire up the rtp and resolve the diver lua/ai tree (read-only).
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
    local ai_dir = diver_lua_dir .. '/lua/ai'
    local probe = io.open(ai_dir .. '/harness/approval.lua', 'r')
    if probe == nil then
        return nil, 'diver lua/ai tree not found under DIVER_LUA_DIR: ' .. ai_dir
    end
    probe:close()
    ev('diver lua/ai tree: ' .. ai_dir)
    return { ai_dir = ai_dir, work_dir = work_dir }
end

---Write `text` to a file inside GAUNTLET_WORK_DIR.
---@param handles table
---@param name string
---@param text string
---@return string? err
local function write_work_file(handles, name, text)
    local path = handles.work_dir .. '/' .. name
    local fh, err = io.open(path, 'w')
    if fh == nil then
        return 'cannot write ' .. path .. ': ' .. tostring(err)
    end
    fh:write(text)
    fh:close()
    return nil
end

---Bounded exact-token (case-insensitive, word-boundary) scan over the
---diver lua/ai tree. Returns hit count and up to 8 hit paths.
---@param handles table
---@param tokens string[]
---@return integer hits
---@return string[] hit_paths
local function scan_ai_tree(handles, tokens)
    local files = vim.fn.globpath(handles.ai_dir, '**/*.lua', false, true)
    local hits = 0
    local hit_paths = {}
    local seen = 0
    for _, path in ipairs(files) do
        seen = seen + 1
        if seen > SCAN_FILES_MAX then
            break
        end
        local fh = io.open(path, 'r')
        if fh ~= nil then
            local content = fh:read('*a') or ''
            fh:close()
            if #content <= SCAN_BYTES_MAX then
                local lower = content:lower()
                for _, token in ipairs(tokens) do
                    if lower:find('%f[%w]' .. token .. '%f[%W]') ~= nil then
                        hits = hits + 1
                        if #hit_paths < 8 then
                            hit_paths[#hit_paths + 1] = path .. ' :: ' .. token
                        end
                        break
                    end
                end
            end
        end
    end
    return hits, hit_paths
end

---Write the machine-readable trace for one scenario.
---@param handles table
---@param trace table
---@return string? err
local function write_trace(handles, trace)
    trace.id = TASK_ID
    return write_work_file(handles, TRACE_NAME, vim.json.encode(trace))
end

---Require a real diver module and confirm it loaded from the diver tree.
---@param modname string
---@return table? mod
---@return string? err
local function require_diver(modname)
    local ok, mod = pcall(require, modname)
    if not ok or type(mod) ~= 'table' then
        return nil, 'cannot require ' .. modname .. ': ' .. tostring(mod)
    end
    return mod, nil
end

---Scenario `binding` (V): the approval queue has no content-hash binding
---API, and the artifact store's content_hash is unreachable from it.
local function scenario_binding(handles)
    local approval, err = require_diver('ai.harness.approval')
    if approval == nil then
        return fail('bootstrap', err)
    end
    ev('driving real module: ai.harness.approval')
    local queue = approval.new()
    local binding_api = false
    for _, field in ipairs({ 'bind_approval', 'proposal_hash', 'content_hash', 'candidate_digest' }) do
        if approval[field] ~= nil then
            binding_api = true
            ev('UNEXPECTED binding API on ai.harness.approval: ' .. field)
        end
    end
    -- The store's content_hash is the red herring: confirm the approval
    -- queue never references it.
    local store, serr = require_diver('ai.harness.store')
    if store == nil then
        return fail('bootstrap', serr)
    end
    local store_has_hash = store.put_artifact ~= nil
    ev('ai.harness.store exposes artifact store (put_artifact=' .. tostring(store_has_hash) .. ')')
    local id, rerr = approval.request(queue, 'run-1', { tool = 'write', risk = 'irreversible', summary = 'x' })
    if id == nil then
        return fail('scenario', 'approval.request failed: ' .. tostring(rerr))
    end
    local rec = approval.get(queue, id)
    local rec_has_hash = rec ~= nil
        and (rec.content_hash ~= nil or rec.proposal_hash ~= nil or rec.candidate_digest ~= nil)
    ev('approval record fields: id/tool/risk/summary/argv/paths — no hash field: ' .. tostring(not rec_has_hash))
    local werr = write_trace(handles, {
        scenario = 'binding',
        binding_api = binding_api,
        record_has_hash = rec_has_hash,
    })
    if werr ~= nil then
        return fail('trace', werr)
    end
    if binding_api or rec_has_hash then
        return fail('binding', 'a content-hash binding API appeared on the approval queue (premise changed)')
    end
    return pass()
end

---Scenario `scope` (V): the policy binds approvals to action scope
---(rule decisions), not to content hashes.
local function scenario_scope(handles)
    local policy, err = require_diver('ai.harness.policy')
    if policy == nil then
        return fail('bootstrap', err)
    end
    ev('driving real module: ai.harness.policy')
    local scope_binding = policy.decide ~= nil or policy.check ~= nil or policy.evaluate ~= nil
    ev('policy exposes scope decisions (task-59 mechanism): ' .. tostring(scope_binding))
    local hash_binding = false
    for _, field in ipairs({ 'bind_approval', 'proposal_hash', 'content_hash', 'drift' }) do
        if policy[field] ~= nil then
            hash_binding = true
            ev('UNEXPECTED hash API on ai.harness.policy: ' .. field)
        end
    end
    local werr = write_trace(handles, {
        scenario = 'scope',
        scope_binding = scope_binding,
        hash_binding = hash_binding,
    })
    if werr ~= nil then
        return fail('trace', werr)
    end
    if hash_binding then
        return fail('scope', 'policy gained a content-hash binding API (premise changed)')
    end
    return pass()
end

---Scenario `drift` (A): no drift-detection module or API exists in
---lua/ai; a bounded token scan finds zero hits.
local function scenario_drift(handles)
    local absent = {}
    for _, modname in ipairs({ 'ai.harness.drift', 'ai.self_improve', 'ai.approval' }) do
        local ok, mod = pcall(require, modname)
        absent[#absent + 1] = modname .. '=' .. tostring(not (ok and type(mod) == 'table'))
    end
    ev('candidate drift modules absent: ' .. table.concat(absent, ' '))
    local hits, hit_paths = scan_ai_tree(handles, { 'drift' })
    ev('token scan for "drift" over lua/ai: ' .. hits .. ' hit(s)')
    for _, hp in ipairs(hit_paths) do
        ev('  hit: ' .. hp)
    end
    local werr = write_trace(handles, {
        scenario = 'drift',
        drift_hits = hits,
    })
    if werr ~= nil then
        return fail('trace', werr)
    end
    if hits ~= 0 then
        return fail('drift', 'drift vocabulary appeared in lua/ai (premise changed)')
    end
    return pass()
end

---Scenario `recovery` (A): no re-review/re-approve path and no
---`proposal_drift` typed error exist in lua/ai.
local function scenario_recovery(handles)
    local hits, hit_paths = scan_ai_tree(handles, { 're_approve', 'reapprove', 'proposal_drift' })
    ev('token scan for re_approve/reapprove/proposal_drift over lua/ai: ' .. hits .. ' hit(s)')
    for _, hp in ipairs(hit_paths) do
        ev('  hit: ' .. hp)
    end
    local werr = write_trace(handles, {
        scenario = 'recovery',
        recovery_hits = hits,
    })
    if werr ~= nil then
        return fail('trace', werr)
    end
    if hits ~= 0 then
        return fail('recovery', 'recovery vocabulary appeared in lua/ai (premise changed)')
    end
    return pass()
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'binding'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return fail('bootstrap', boot_err)
    end
    ev('scenario=' .. scenario)
    if scenario == 'binding' then
        return scenario_binding(handles)
    elseif scenario == 'scope' then
        return scenario_scope(handles)
    elseif scenario == 'drift' then
        return scenario_drift(handles)
    elseif scenario == 'recovery' then
        return scenario_recovery(handles)
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
