-- task-37 driver: confused deputy on diver's tool-authorization check.
--
-- Recon probe: the design asks that the authorization check consider
-- the REQUESTER CHAIN, not just the immediate tool. Tool A (low
-- privilege) returns output instructing the agent to invoke tool B
-- (high privilege) "on its behalf": the check must deny (or
-- re-approve) the invocation when it is caused by A's output, and the
-- denial must name the broken chain. A two-hop laundering A->C->B must
-- still be detected.
--
-- This driver exercises the REAL authorization module —
-- ai.harness.policy (M.new / M.decide), the single authorization
-- decision point per its own header — with mock tools A (chatterbox),
-- B (privileged), and C (laundering hop). It makes no network calls
-- and spawns no workers.
--
-- Honest result: the check considers only the IMMEDIATE request.
-- AiHarnessToolRequest carries risk/tool/argv/paths/endpoints/
-- workspace — no requester-chain field (no chain, principal,
-- delegated_by, on_behalf_of, or caused_by). policy.decide's
-- rule_matches consults only risk/tools/paths/endpoints. An extra
-- `caused_by` field on the request table is silently ignored: the
-- deputy-caused invocation of B decides IDENTICALLY to a direct,
-- properly-approved invocation of B. The design's "denial names the
-- broken chain" is impossible — there is no chain to name. (Observed
-- too: nothing in the harness calls policy.decide per tool
-- invocation — the supervisor stores opts.policy but never consults
-- it — so even the immediate-request check is currently unwired; the
-- probe documents the decision function itself.)
--
-- Fail-closed: if a chain-caused request ever decides differently
-- from the identical direct request (chain tracking wired in), the
-- probe reports where="recon" instead.
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
    return { id = 'task-37', outcome = 'fail', where = where, how = how, evidence = evidence }
end

---@return table? policy_mod
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
    local ok, policy_mod = pcall(require, 'ai.harness.policy')
    if not ok then
        return nil, 'require ai.harness.policy failed: ' .. tostring(policy_mod)
    end
    return policy_mod
end

---Build the request the deputy submits for tool B: the immediate tool
---request, plus the (ignored) provenance of who caused it.
---@param extra? table
---@return table
local function request_for_b(extra)
    local req = { risk = 'irreversible', tool = 'vault.export', workspace = '/work' }
    if extra ~= nil then
        for k, v in pairs(extra) do
            req[k] = v
        end
    end
    return req
end

local function main()
    local policy_mod, boot_err = bootstrap()
    if boot_err ~= nil then
        return fail('bootstrap', boot_err)
    end
    ev('ai.harness.policy loaded from DIVER_LUA_DIR')

    -- Mock tools: A = notes.read (low privilege, the chatterbox),
    -- C = cache.write (mid privilege, the laundering hop),
    -- B = vault.export (high privilege, the deputy's target).
    local policy, perr = policy_mod.new({
        default = 'deny',
        rules = {
            { risk = 'observe', decision = 'allow', tools = { 'notes.read' } },
            { risk = 'local_reversible', decision = 'allow', tools = { 'cache.write' } },
            { risk = 'irreversible', decision = 'approval', tools = { 'vault.export' } },
        },
    })
    if policy == nil then
        return fail('lua-driver', 'policy.new failed: ' .. tostring(perr))
    end
    ev('policy: notes.read -> allow, cache.write -> allow, vault.export -> approval (default deny)')

    -- V1 (default scenario): the agent invokes B directly with proper
    -- approval. The immediate-request check answers 'approval'.
    local direct = policy_mod.decide(policy, request_for_b())
    if direct.decision ~= 'approval' then
        return fail(
            'lua-driver',
            'direct invocation of vault.export decided "' .. tostring(direct.decision) .. '", want "approval"'
        )
    end
    ev('V1: direct invocation of vault.export -> decision=approval (reason: ' .. direct.reason .. ') — the immediate-request check works')

    -- A1 (adversarial): tool A's output instructs the agent to invoke B
    -- "on its behalf". The deputy submits the request for B. The design
    -- requires the check to consider the requester chain (A -> B) and
    -- deny or re-approve, naming the broken chain.
    local caused = policy_mod.decide(policy, request_for_b({ caused_by = 'notes.read' }))
    ev('A1: deputy-caused invocation of vault.export (caused_by=notes.read) -> decision=' .. caused.decision .. ' (reason: ' .. caused.reason .. ')')
    local identical = vim.json.encode(direct) == vim.json.encode(caused)
    ev('A1: deputy-caused decision is byte-identical to the direct decision: ' .. tostring(identical) .. ' — the extra provenance field is silently ignored')
    if not identical then
        return fail(
            'recon',
            'a chain-caused request now decides differently from the identical direct request: requester-chain tracking appears wired into policy.decide; probe premise changed'
        )
    end
    ev('A1: AiHarnessToolRequest carries risk/tool/argv/paths/endpoints/workspace only — no chain, principal, delegated_by, on_behalf_of, or caused_by field; rule_matches consults only risk/tools/paths/endpoints (policy.lua)')

    -- A2 (adversarial): A launders the request through two hops
    -- (A -> C -> B). The chain must still be detected.
    local laundered = policy_mod.decide(policy, request_for_b({ caused_by = 'cache.write', origin = 'notes.read' }))
    ev('A2: two-hop laundered invocation of vault.export (notes.read -> cache.write -> vault.export) -> decision=' .. laundered.decision .. ' (reason: ' .. laundered.reason .. ')')
    local identical2 = vim.json.encode(direct) == vim.json.encode(laundered)
    ev('A2: laundered decision is byte-identical to the direct decision: ' .. tostring(identical2) .. ' — two-hop laundering is equally invisible to the check')
    if not identical2 then
        return fail(
            'recon',
            'a two-hop-laundered request now decides differently from the direct request: chain tracking appears wired in; probe premise changed'
        )
    end

    ev('wiring: no harness module calls policy.decide per tool invocation (supervisor.lua stores opts.policy but never consults it) — even the immediate-request check is currently unwired; the probe documents policy.decide itself')
    return fail(
        'seam',
        'seam absent: the authorization check has no requester-chain concept — AiHarnessToolRequest carries no chain/principal/delegation field and policy.decide ignores extra provenance fields, '
            .. 'so a high-privilege invocation caused by a low-privilege tool\'s output decides identically to a direct, properly-approved invocation. '
            .. 'The design\'s "denial names the broken chain" is impossible: there is no chain to name. '
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
