-- task-55 driver: dissent escalation recon probe.
--
-- The design asks for dissent escalation on the multi-model answer
-- comparison path: two heterogeneous models answer the same prompt; on
-- agreement the run proceeds, on disagreement the conflict is escalated
-- to a human with both answers quoted (never a silent majority), and a
-- model that is systematically wrong on a class of questions trips a
-- dissent-rate alert and is quarantined from the comparison set pending
-- review — the quarantine threshold a named constant.
--
-- Recon probe: the driver inspects the REAL comparison surfaces — the
-- six harness adapters (rose, phlow, a2a, herd, mcp, acp: the modules
-- that return answers to prompts), ai.harness.verdict, and
-- ai.security (the one place "escalation"/"quarantine" vocabulary
-- exists) — reading exported function tables for answer-comparison /
-- dissent / escalation machinery. It makes no network calls and spawns
-- no workers.
--
-- Honest result: no comparison path exists. Adapters start workers and
-- return handles; nothing feeds two models' answers into a comparator;
-- escalation/quarantine in ai.security concern suspicious FILES (move to
-- an isolated directory), not model answers; there is no escalation
-- record, no dissent-rate counter, and no quarantine-threshold
-- constant for models. Distinct from task-19 (the council VOTES and a
-- tie resolves to Revise): this design refuses to resolve machine-side
-- at all — and there is no machine-side path to refuse with.
--
-- Fail-closed: if comparison machinery ever appears, the driver reports
-- where="recon" (premise changed) instead of the seam absence.
--
-- Scenarios via GAUNTLET_SCENARIO (default "adapters"):
--   adapters              probe the 6 harness adapters' export tables
--   verdict-and-security  probe ai.harness.verdict + ai.security
--   no-escalation-record  document: no escalation record type, no
--                         dissent counter, no quarantine constant
--   fail-closed-recon     union scan; hits -> where="recon"
--
-- Prints exactly one JSON verdict line to stdout and always exits 0; the
-- verdict carries the outcome, not the exit code. Writes nothing outside
-- GAUNTLET_WORK_DIR. Never modifies the diver repo.

local EVIDENCE_MAX = 64
local TASK_ID = 'task-55'

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function fail(where, how)
    return { id = TASK_ID, outcome = 'fail', where = where, how = how, evidence = evidence }
end

---The modules that return answers to prompts (harness adapters), the
---per-run verifier, and the one module with escalation/quarantine
---vocabulary (file security scanning — the control sample).
local ADAPTER_MODULES = {
    'ai.harness.adapters.rose',
    'ai.harness.adapters.phlow',
    'ai.harness.adapters.a2a',
    'ai.harness.adapters.herd',
    'ai.harness.adapters.mcp',
    'ai.harness.adapters.acp',
}
local VERDICT_MODULES = { 'ai.harness.verdict' }
---The control sample: the one module with real escalation/quarantine
---vocabulary (file security scanning). Probed for evidence ONLY — its
---hits are classified unrelated and never counted as comparison-path
---hits. Without this separation, ai.security.quarantine would
---false-positive the whole probe into where="recon".
local CONTROL_MODULES = { 'ai.security' }

---Name fragments indicating answer-comparison / dissent machinery.
---Matched case-insensitively against exported function names.
local COMPARISON_NEEDLES = {
    'compar',
    'dissent',
    'disagree',
    'escalat',
    'quarantine',
    'consensus',
    'adjudicat',
}

---@return boolean? ok
---@return string? err
local function bootstrap()
    local diver_lua_dir = vim.env.DIVER_LUA_DIR
    if type(diver_lua_dir) ~= 'string' or diver_lua_dir == '' then
        return nil, 'DIVER_LUA_DIR is not set'
    end
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    if type(work_dir) ~= 'string' or work_dir == '' then
        return nil, 'GAUNTLET_WORK_DIR is not set'
    end
    vim.opt.runtimepath:append(diver_lua_dir)
    return true
end

---Collect the exported function names of a module table.
---@param mod table
---@return string[] names
local function exported_names(mod)
    local names = {}
    for key, value in pairs(mod) do
        if type(value) == 'function' and type(key) == 'string' then
            names[#names + 1] = key
        end
    end
    table.sort(names)
    return names
end

---Require one module and scan its exported function names for the
---comparison needles. Records evidence either way.
---@param path string
---@return string[] hits
local function probe_module(path)
    local ok, mod = pcall(require, path)
    if not ok then
        ev('note: ' .. path .. ': require failed: ' .. tostring(mod):sub(1, 120))
        return {}
    end
    if type(mod) ~= 'table' then
        ev('note: ' .. path .. ': module is not a table')
        return {}
    end
    local names = exported_names(mod)
    ev('probed ' .. path .. ': ' .. #names .. ' exported functions (' .. table.concat(names, ', ') .. ')')
    local hits = {}
    for _, name in ipairs(names) do
        local lower = name:lower()
        for _, needle in ipairs(COMPARISON_NEEDLES) do
            if lower:find(needle, 1, true) then
                hits[#hits + 1] = path .. '.' .. name
                break
            end
        end
    end
    return hits
end

---Facet: the adapters start workers; none compares two models' answers.
---@return string[] hits
local function facet_adapters()
    ev('facet adapters: the modules that return answers to prompts')
    local hits = {}
    for _, path in ipairs(ADAPTER_MODULES) do
        for _, hit in ipairs(probe_module(path)) do
            hits[#hits + 1] = hit
        end
    end
    ev('adapters expose start/probe/cancel/close-style worker lifecycles — they hand answers back, they never compare two models\' answers to the same prompt')
    ev('no compare/dissent/disagree entry point on any adapter: disagreement between models has no code path to travel')
    return hits
end

---Facet: the verifier grades one run; the security module's escalation /
---quarantine is about files, not model answers (the control sample that
---proves the vocabulary scan is not blind).
---@return string[] hits
local function facet_verdict_and_security()
    ev('facet verdict-and-security: the per-run verifier and the escalation-vocabulary control sample')
    local hits = {}
    for _, path in ipairs(VERDICT_MODULES) do
        for _, hit in ipairs(probe_module(path)) do
            hits[#hits + 1] = hit
        end
    end
    for _, path in ipairs(CONTROL_MODULES) do
        local control_hits = probe_module(path)
        if #control_hits > 0 then
            ev('control sample ' .. path .. ': needles matched (' .. table.concat(control_hits, ', ') .. ') — classified UNRELATED: file-scanning escalation/quarantine, not model-answer dissent')
        else
            ev('control sample ' .. path .. ': no needle matches (scan premise changed)')
        end
    end
    ev('ai.harness.verdict.evaluate: grades ONE run against acceptance criteria — no second model, no comparison')
    ev('ai.security escalation/quarantine: composite security verdicts on scanned FILES (quarantine moves a suspicious file to an isolated directory) — unrelated to model-answer dissent')
    ev('control sample: the needles DO match real vocabulary in ai.security, so zero hits on the comparison path means the path is absent, not the scan broken')
    return hits
end

---Facet: document the three named artifacts the design requires and
---their absence — the escalation record, the dissent-rate counter, and
---the quarantine-threshold constant.
---@return string[] hits always empty; the facet documents, it does not find
local function facet_no_escalation_record()
    ev('facet no-escalation-record: the design\'s three required artifacts')
    ev('1. escalation record quoting both answers + the human-review gate: no such record type exists on any probed module — there is nothing for a human to review because disagreement is never recorded')
    ev('2. dissent-rate counter per model: no counter, no rate, no alert — a systematically-wrong model has no tripwire')
    ev('3. quarantine threshold as a named constant: no constant — a compromised model cannot be quarantined from a comparison set that does not exist')
    ev('pass criterion check: "no action is taken on a dissented answer without human review" holds vacuously (no dissent is ever detected), which is exactly the failure mode the design forbids — silent non-detection, not silent majority')
    return {}
end

---Facet: fail-closed union scan over every probed module.
---@return string[] hits
local function facet_fail_closed()
    ev('facet fail-closed-recon: union scan over adapters + verdict (+ security as control sample)')
    local hits = {}
    for _, path in ipairs(ADAPTER_MODULES) do
        for _, hit in ipairs(probe_module(path)) do
            hits[#hits + 1] = hit
        end
    end
    for _, path in ipairs(VERDICT_MODULES) do
        for _, hit in ipairs(probe_module(path)) do
            hits[#hits + 1] = hit
        end
    end
    for _, path in ipairs(CONTROL_MODULES) do
        local control_hits = probe_module(path)
        if #control_hits > 0 then
            ev('control sample ' .. path .. ': needles matched (' .. table.concat(control_hits, ', ') .. ') — classified UNRELATED: file-scanning escalation/quarantine, not model-answer dissent')
        end
    end
    return hits
end

local function main()
    local _, boot_err = bootstrap()
    if boot_err ~= nil then
        return fail('bootstrap', boot_err)
    end
    ev('harness lua tree bootstrapped from DIVER_LUA_DIR')
    ev('comparison needles: ' .. table.concat(COMPARISON_NEEDLES, ', '))
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'adapters'
    end
    ev('scenario=' .. scenario)
    local hits
    if scenario == 'adapters' then
        hits = facet_adapters()
    elseif scenario == 'verdict-and-security' then
        hits = facet_verdict_and_security()
    elseif scenario == 'no-escalation-record' then
        hits = facet_no_escalation_record()
    elseif scenario == 'fail-closed-recon' then
        hits = facet_fail_closed()
    else
        return fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. scenario)
    end
    if #hits > 0 then
        return fail(
            'recon',
            'comparison machinery now exists (' .. table.concat(hits, ', ') .. '); probe outdated'
        )
    end
    ev('zero comparison-API hits on the answer path: no compare, no dissent, no escalation, no model quarantine')
    return fail(
        'seam',
        'seam absent: diver has no multi-model answer comparison path — adapters return answers, nothing compares two models\' answers to the same prompt, '
            .. 'and no escalation record, dissent-rate counter, or quarantine-threshold constant exists for models (ai.security\'s escalation/quarantine is file scanning, unrelated). '
            .. 'The design\'s "escalate dissent to a human, never silent majority" has no seam to assert against; open design gap (diver-owned).'
    )
end

local ok, verdict = pcall(main)
if not ok then
    ev('lua error: ' .. tostring(verdict))
    verdict = fail('lua-driver', 'unhandled error: ' .. tostring(verdict))
end
-- Verdict on the real stdout: in `nvim --headless -l`, Lua print() goes
-- to stderr, but the Rust runner parses the verdict from stdout.
io.stdout:write(vim.json.encode(verdict) .. '\n')
