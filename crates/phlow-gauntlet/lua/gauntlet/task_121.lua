-- task-121 driver: deny-all default + explicit opt-in acceptance probe
-- (diver Phase-2 Decision 1).
--
-- The decision: diver ships `policy = { default = 'deny', rules = {} }`;
-- `policy_example.lua` (allow observe + local_reversible) is shipped
-- BESIDE it, never loaded implicitly — enabling it is one explicit line,
-- because `local_reversible` is currently the adapter's unverified claim.
--
-- Diver probed: c84352cc850d507df477706b9166b6541ebe9e1c (main; no Phase-2
-- branch exists).
--
-- Scenarios via GAUNTLET_SCENARIO (default "default"):
--   default               fresh setup({}) yields default='deny', rules={} (V)
--   decide-fail-closed    decide denies: empty rules, nil policy
--                         ('no policy configured'), malformed request,
--                         unknown risk class (V)
--   no-implicit-example   regression guard: the setup path never requires
--                         the example module implicitly (static source scan
--                         + runtime package.loaded + empty rules) (A)
--   opt-in-absent         the documented one-line opt-in has no target:
--                         ai.harness.policy_example does not exist (A, gap)
--
-- The first three scenarios pass today (characterization + guard). The
-- fourth records the exact gap; that record IS the Phase-2 acceptance
-- artifact.
--
-- Prints exactly one JSON verdict line to stdout and always exits 0.
-- Writes nothing outside GAUNTLET_WORK_DIR. Never modifies the diver repo.

local DIVER_SHA = 'c84352cc850d507df477706b9166b6541ebe9e1c'
local EVIDENCE_MAX = 64

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function gap(how)
    return { id = 'task-121', outcome = 'fail', where = 'policy-example-absent', how = how, evidence = evidence }
end

local function scenario_pass(note)
    ev(note)
    return { id = 'task-121', outcome = 'pass', evidence = evidence }
end

local function driver_fail(where, how)
    return { id = 'task-121', outcome = 'fail', where = where, how = how, evidence = evidence }
end

---Resolve DIVER_LUA_DIR to the runtimepath root and the lua/ dir beneath
---it. Accepts either the rtp root itself (holding lua/ai/harness/init.lua)
---or the lua/ dir directly (holding ai/harness/init.lua): the rtp entry
---must be the directory that *contains* lua/, or require() never fires.
local function resolve_diver_dirs(diver_lua_dir)
    local function is_file(path)
        local fh = io.open(path, 'r')
        if fh == nil then
            return false
        end
        fh:close()
        return true
    end
    if is_file(diver_lua_dir .. '/lua/ai/harness/init.lua') then
        return diver_lua_dir, diver_lua_dir .. '/lua'
    end
    if is_file(diver_lua_dir .. '/ai/harness/init.lua') then
        local parent = diver_lua_dir:gsub('/+$', ''):gsub('/[^/]+$', '')
        if parent == '' then
            parent = '/'
        end
        return parent, diver_lua_dir:gsub('/+$', '')
    end
    return nil, nil
end

---Wire up the harness. Returns handles or (nil, err).
local function bootstrap()
    local diver_lua_dir = vim.env.DIVER_LUA_DIR
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    if type(diver_lua_dir) ~= 'string' or diver_lua_dir == '' then
        return nil, 'DIVER_LUA_DIR is not set'
    end
    if type(work_dir) ~= 'string' or work_dir == '' then
        return nil, 'GAUNTLET_WORK_DIR is not set'
    end
    local rtp_root, lua_dir = resolve_diver_dirs(diver_lua_dir)
    if rtp_root == nil then
        return nil, 'DIVER_LUA_DIR has no ai/harness/init.lua under <dir>/lua or <dir>: ' .. diver_lua_dir
    end
    vim.opt.runtimepath:append(rtp_root)
    local harness_ok, harness = pcall(require, 'ai.harness')
    if not harness_ok then
        return nil, "require('ai.harness') failed: " .. tostring(harness)
    end
    local ok, err = harness.setup({})
    if not ok then
        return nil, 'harness.setup failed: ' .. tostring(err)
    end
    local st = harness._state
    if st == nil then
        return nil, 'harness internal state unavailable after setup'
    end
    return {
        harness = harness,
        policy = st.policy,
        policy_mod = require('ai.harness.policy'),
        harness_dir = lua_dir .. '/ai/harness/',
    }
end

---Read a whole file from the harness dir, or return nil.
local function read_harness_file(handles, name)
    local fh = io.open(handles.harness_dir .. name, 'r')
    if fh == nil then
        return nil
    end
    local text = fh:read('*a')
    fh:close()
    return text
end

---default (V): fresh setup({}) yields default='deny' with exactly no rules.
local function scenario_default(handles)
    local policy = handles.policy
    if type(policy) ~= 'table' then
        return driver_fail('setup', 'post-setup policy is not a table')
    end
    ev("policy.default = '" .. tostring(policy.default) .. "'")
    if policy.default ~= 'deny' then
        return driver_fail('default', "fresh setup policy.default is '" .. tostring(policy.default) .. "', want 'deny'")
    end
    if type(policy.rules) ~= 'table' or next(policy.rules) ~= nil then
        return driver_fail('default', 'fresh setup policy.rules is not exactly {}')
    end
    ev('policy.rules is exactly {} — no rules ship enabled')
    ev('source: init.lua M.setup ("Fail closed: without an explicit policy every tool request denies")')
    ev('source: policy.lua M.new — config.default is \'deny\' unless explicitly \'allow\'')
    return scenario_pass("fresh setup({}) yields default='deny', rules={}")
end

---decide-fail-closed (V): decide denies on empty rules, nil policy,
---malformed requests, and unknown risk classes.
local function scenario_decide(handles)
    local decide = handles.policy_mod.decide
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    local req = { risk = 'observe', tool = 'fs.read', workspace = work_dir }

    local d1 = decide(handles.policy, req)
    if d1.decision ~= 'deny' then
        return driver_fail('decide', 'decide with empty rules allowed: ' .. vim.inspect(d1.decision))
    end
    ev("decide(policy, observe request) -> deny ('" .. d1.reason .. "')")

    local d2 = decide(nil, req)
    if d2.decision ~= 'deny' or d2.reason ~= 'no policy configured' then
        return driver_fail(
            'decide',
            'decide(nil policy) did not fail closed with reason \'no policy configured\': '
                .. vim.inspect(d2)
        )
    end
    ev("decide(nil, request) -> deny ('no policy configured')")

    local d3 = decide(handles.policy, 'not a table')
    if d3.decision ~= 'deny' or d3.reason ~= 'malformed request' then
        return driver_fail('decide', 'decide with non-table request did not deny: ' .. vim.inspect(d3))
    end
    ev("decide(policy, 'not a table') -> deny ('malformed request')")

    local d4 = decide(handles.policy, { risk = 'bogus', tool = 'x', workspace = work_dir })
    if d4.decision ~= 'deny' or d4.reason ~= 'unknown risk class' then
        return driver_fail('decide', 'decide with unknown risk class did not deny: ' .. vim.inspect(d4))
    end
    ev("decide(policy, risk='bogus') -> deny ('unknown risk class')")

    ev('source: policy.lua M.decide — nil policy / malformed request deny before any rule matching')
    return scenario_pass('decide fail-closes on nil policy, malformed requests, unknown risk')
end

---no-implicit-example (A): regression guard — the setup path must never
---require the example module implicitly. Static source scan plus runtime
---checks; a future "helpful" auto-enable fails this test by construction.
local function scenario_no_implicit(handles)
    for _, name in ipairs({ 'init.lua', 'supervisor.lua', 'policy.lua' }) do
        local text = read_harness_file(handles, name)
        if text == nil then
            return driver_fail('scan', 'cannot read harness source file: ' .. name)
        end
        if text:find('policy_example', 1, true) ~= nil then
            return driver_fail(
                'implicit-example',
                name .. ' references policy_example — the example may be loading implicitly'
            )
        end
        ev('static: ' .. name .. ' has no policy_example reference')
    end
    if package.loaded['ai.harness.policy_example'] ~= nil then
        return driver_fail('implicit-example', 'ai.harness.policy_example is in package.loaded after setup({})')
    end
    ev('runtime: package.loaded has no ai.harness.policy_example after setup({})')
    if next(handles.policy.rules) ~= nil then
        return driver_fail('implicit-example', 'post-setup policy.rules is non-empty: rules were injected')
    end
    ev('runtime: post-setup policy.rules is exactly {} — no rules injected')
    return scenario_pass('setup path never loads the example module implicitly')
end

---opt-in-absent (A): the documented one-line opt-in has no target module.
---Records the Phase-2 gap with the exact acceptance criterion.
local function scenario_opt_in(handles)
    local ok = pcall(require, 'ai.harness.policy_example')
    if ok then
        return scenario_pass('ai.harness.policy_example exists and loads (Decision 1 shipped)')
    end
    ev('GAP today: require("ai.harness.policy_example") fails — module not found')
    ev('file evidence: no policy_example* file anywhere under diver lua/ (repo-wide find, 2026-09-28)')
    ev('file evidence: lua/ai/harness/ holds init/policy/supervisor/... but no policy_example.lua')
    local how = 'decision-1 gap: Phase-2 Decision 1 ships ai.harness.policy_example beside the '
        .. "deny-all default ({ default='deny', rules={{risk='observe',decision='allow'},"
        .. "{risk='local_reversible',decision='allow'}} } — spec lines 155-164), enabled by the "
        .. "documented one line setup({ policy = require('ai.harness.policy_example') }). The module "
        .. 'does not exist, so the opt-in cannot be exercised. Phase-2 acceptance: the documented '
        .. 'line loads the module, decide allows observe requests, and still denies network '
        .. "(remote adapters classify 'network', matching no allow rule)."
    return gap(how)
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'default'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return driver_fail('bootstrap', boot_err)
    end
    ev('diver probed: ' .. DIVER_SHA .. ' (main; no Phase-2 branch exists)')
    ev('scenario=' .. scenario)
    if scenario == 'default' then
        return scenario_default(handles)
    elseif scenario == 'decide-fail-closed' then
        return scenario_decide(handles)
    elseif scenario == 'no-implicit-example' then
        return scenario_no_implicit(handles)
    elseif scenario == 'opt-in-absent' then
        return scenario_opt_in(handles)
    end
    return driver_fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
end

local ok, verdict = pcall(main)
if not ok then
    ev('lua error: ' .. tostring(verdict))
    verdict = driver_fail('lua-driver', 'unhandled error: ' .. tostring(verdict))
end
-- verdict on the real stdout: in `nvim --headless -l`, Lua print() goes
-- to stderr, but the Rust runner parses the verdict from stdout.
io.stdout:write(vim.json.encode(verdict) .. '\n')
