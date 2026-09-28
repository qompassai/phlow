-- task_03.lua -- gauntlet driver: approval gate blocks unapproved tool use.
--
-- Drives the REAL harness enforcement seam:
--   * ai.harness.policy.decide(policy, request) -- the one authorization
--     decision point (policy.lua: "The only path from a model proposal to a
--     side effect").
--   * ai.harness.approval -- the async approval queue (request / decide /
--     pending / sweep_expired). The harness supervisor drives expiry on
--     tick(): requests expire to denied, never to approved.
--
-- DECLARED GAP: the harness ships no tool executor, so nothing inside the
-- repo wires a tool proposal -> policy.decide -> approval.request -> wait ->
-- execute (policy.decide and approval.request have zero callers in the
-- harness). There is also no live human UI/backend in this sandbox, so the
-- "approver" is a test-side stand-in that acts ONLY through the real
-- approval.decide(queue, id, decision, by) API on the real queue. The gate
-- itself follows policy.lua's contract exactly; the stand-in is named in
-- every verdict's evidence.
--
-- Scenarios (GAUNTLET_SCENARIO):
--   default      approver grants  -> tool call must proceed
--   no-approver  nobody decides   -> call BLOCKED, approval expires to denied
--   denied       approver denies  -> call BLOCKED, denial recorded exactly
--   unknown-tool tool matches no rule -> policy denies, no approval requested
--
-- The "tool" is a marker-file write confined to GAUNTLET_WORK_DIR.
-- Non-execution is proved positively (executed flag false AND marker file
-- absent), never by "we didn't see it happen".
--
-- Prints exactly one JSON verdict line and always exits 0.

local EVIDENCE_LINES_MAX = 64
local APPROVAL_TIMEOUT_MS = 1000
local MARKER_NAME = 'tool-executed.txt'
local MARKER_TEXT = 'task-03: approved tool call executed\n'
local STAND_IN = 'task-03:stand-in-approver'

local evidence = {}

---@param line string
local function note(line)
    if #evidence < EVIDENCE_LINES_MAX then
        evidence[#evidence + 1] = line
    end
end

---@param outcome string 'pass'|'fail'
---@param where_? string
---@param how? string
local function verdict(outcome, where_, how)
    local v = { id = 'task-03', outcome = outcome, evidence = evidence }
    if where_ ~= nil then
        v.where = where_
    end
    if how ~= nil then
        v.how = how
    end
    io.write(vim.json.encode(v) .. '\n')
    io.flush()
    os.exit(0)
end

---@param where_ string
---@param how string
local function fail(where_, how)
    note('FAIL where=' .. where_ .. ' how=' .. how)
    verdict('fail', where_, how)
end

-- The gated "tool": writes a marker file inside the work dir only.
-- Returns ok, err. The executed flag is the positive proof of execution.
local executed = false

---@param work_dir string
---@return boolean ok
---@return string? err
local function execute_tool(work_dir)
    local path = work_dir .. '/' .. MARKER_NAME
    local fh, open_err = io.open(path, 'w')
    if fh == nil then
        return false, 'cannot open marker file: ' .. tostring(open_err)
    end
    fh:write(MARKER_TEXT)
    fh:close()
    executed = true
    return true
end

---@param path string
---@return boolean
local function file_exists(path)
    local fh = io.open(path, 'r')
    if fh == nil then
        return false
    end
    fh:close()
    return true
end

---@param path string
---@return string?
local function read_file(path)
    local fh = io.open(path, 'r')
    if fh == nil then
        return nil
    end
    local text = fh:read('*a')
    fh:close()
    return text
end

-- The approval gate: the only path from proposal to side effect.
-- `approver` is nil (nobody decides) or a function(queue, approval_id) that
-- grants/denies through the real approval.decide API.
---@param sup table real harness supervisor (owns .policy and .approvals)
---@param mods table {policy=, approval=, supervisor=}
---@param run_id string
---@param work_dir string
---@param request table AiHarnessToolRequest
---@param approver? function
---@return table result
local function gate(sup, mods, run_id, work_dir, request, approver)
    local policy_mod = mods.policy
    local approval_mod = mods.approval
    local supervisor_mod = mods.supervisor

    -- 1. The enforcement point: policy decides.
    local decision = policy_mod.decide(sup.policy, request)
    note(string.format(
        'policy.decide: decision=%s reason=%s risk=%s',
        tostring(decision.decision),
        tostring(decision.reason),
        tostring(decision.risk)
    ))
    if decision.decision == 'deny' then
        return { allowed = false, phase = 'policy-deny', decision = decision }
    end
    if decision.decision == 'allow' then
        local ok, err = execute_tool(work_dir)
        if not ok then
            return { allowed = false, phase = 'execute-failed', err = err }
        end
        return { allowed = true, phase = 'policy-allow', decision = decision }
    end

    -- 2. Approval required: enqueue on the real harness approval queue.
    local approval_id, req_err = approval_mod.request(sup.approvals, run_id, request, {
        timeout_ms = APPROVAL_TIMEOUT_MS,
    })
    if approval_id == nil then
        return { allowed = false, phase = 'approval-request-failed', err = req_err }
    end
    note('approval.request: id=' .. approval_id .. ' state=pending (run waits)')

    -- 3. The stand-in human acts through the real decide API (or nobody does).
    if approver ~= nil then
        local ok, decide_err = approver(sup.approvals, approval_id)
        if not ok then
            return { allowed = false, phase = 'approver-error', err = decide_err }
        end
    end
    local record = approval_mod.get(sup.approvals, approval_id)

    -- 4. While the approval is pending the run does not proceed: nothing
    --    executes. Drive the real supervisor tick past the deadline; the
    --    harness expires pending approvals to denied, never to approved.
    if record.state == 'pending' then
        note('no approver: approval still pending; tool call does not proceed')
        supervisor_mod.tick(sup, record.deadline_ns + 1)
        record = approval_mod.get(sup.approvals, approval_id)
        note('supervisor.tick past deadline: approval state=' .. record.state)
    end

    if record.state == 'approved' then
        local ok, err = execute_tool(work_dir)
        if not ok then
            return { allowed = false, phase = 'execute-failed', err = err }
        end
        note('approval granted by ' .. tostring(record.decided_by) .. '; tool executed')
        return { allowed = true, phase = 'approval-grant', record = record }
    end
    note('tool call BLOCKED: approval state=' .. record.state
        .. ' decided_by=' .. tostring(record.decided_by))
    return { allowed = false, phase = 'approval-' .. record.state, record = record }
end

local function main()
    local diver_dir = vim.env.DIVER_LUA_DIR
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    local scenario = vim.env.GAUNTLET_SCENARIO or 'default'
    if type(diver_dir) ~= 'string' or diver_dir == '' then
        fail('env', 'DIVER_LUA_DIR missing or empty')
    end
    if type(work_dir) ~= 'string' or work_dir == '' then
        fail('env', 'GAUNTLET_WORK_DIR missing or empty')
    end
    note('scenario=' .. scenario .. ' work_dir=' .. work_dir)
    note('stand-in approver declared: ' .. STAND_IN
        .. ' (no live human backend in sandbox; acts only via approval.decide)')

    vim.opt.runtimepath:append(diver_dir)
    local harness = require('ai.harness')

    -- Fail-closed policy installed through the public setup() API, exactly
    -- as a real deployment would: only fs.write may proceed, and only with
    -- human approval.
    local ok, setup_err = harness.setup({
        policy = {
            default = 'deny',
            rules = {
                { risk = 'local_reversible', tools = { 'fs.write' }, decision = 'approval' },
            },
        },
    })
    if not ok then
        fail('setup', 'harness.setup failed: ' .. tostring(setup_err))
    end
    -- The approval queue lives on the real supervisor; the public harness
    -- API exposes no approval surface, so the driver reaches the managed
    -- supervisor directly (declared seam, not a second implementation).
    local sup = harness._state and harness._state.supervisor
    if sup == nil then
        fail('setup', 'harness supervisor unavailable after setup')
    end
    local mods = {
        policy = require('ai.harness.policy'),
        approval = require('ai.harness.approval'),
        supervisor = require('ai.harness.supervisor'),
    }

    -- A real harness run to own the approval request (stays 'created': no
    -- adapter is started, no backend exists in the sandbox).
    local run, run_err = mods.supervisor.create(sup, {
        workflow = 'gauntlet',
        goal = 'task-03: approval gate blocks unapproved tool use',
        workspace = work_dir,
        -- generous wall clock so the expiry-driving tick() cannot trip the
        -- budget; the budget is not what is under test here.
        budget = { turn = 50, tool_call = 200, token = 200000, time_ms = 3600000, byte = 10000000 },
    })
    if run == nil then
        fail('run', 'supervisor.create failed: ' .. tostring(run_err))
    end
    note('run created: id=' .. run.id .. ' state=' .. run.state)

    local marker_path = work_dir .. '/' .. MARKER_NAME

    if scenario == 'default' then
        local result = gate(sup, mods, run.id, work_dir, {
            risk = 'local_reversible',
            tool = 'fs.write',
            paths = { marker_path },
            workspace = work_dir,
        }, function(queue, approval_id)
            return mods.approval.decide(queue, approval_id, 'approved', STAND_IN)
        end)
        local content = read_file(marker_path)
        local pass = result.allowed
            and executed
            and content == MARKER_TEXT
            and result.record ~= nil
            and result.record.state == 'approved'
        note('executed=' .. tostring(executed)
            .. ' marker_present=' .. tostring(file_exists(marker_path)))
        if pass then
            verdict('pass')
        else
            fail('default', 'approved call did not proceed as required')
        end
    elseif scenario == 'no-approver' then
        local result = gate(sup, mods, run.id, work_dir, {
            risk = 'local_reversible',
            tool = 'fs.write',
            paths = { marker_path },
            workspace = work_dir,
        }, nil)
        local rec = result.record
        local pass = (not result.allowed)
            and (not executed)
            and (not file_exists(marker_path))
            and rec ~= nil
            and (rec.state == 'expired' or rec.state == 'pending')
        note('executed=' .. tostring(executed)
            .. ' marker_present=' .. tostring(file_exists(marker_path))
            .. ' (positive proof the tool never ran)')
        if pass then
            verdict('pass')
        else
            fail('no-approver', 'unapproved call was not blocked as required')
        end
    elseif scenario == 'denied' then
        local result = gate(sup, mods, run.id, work_dir, {
            risk = 'local_reversible',
            tool = 'fs.write',
            paths = { marker_path },
            workspace = work_dir,
        }, function(queue, approval_id)
            return mods.approval.decide(queue, approval_id, 'denied', STAND_IN)
        end)
        local rec = result.record
        local pass = (not result.allowed)
            and (not executed)
            and (not file_exists(marker_path))
            and rec ~= nil
            and rec.state == 'denied'
            and rec.decided_by == STAND_IN
        note('resulting run state: run.state=' .. tostring(mods.supervisor.get(sup, run.id).state)
            .. ' approval.state=' .. tostring(rec and rec.state)
            .. ' decided_by=' .. tostring(rec and rec.decided_by))
        note('executed=' .. tostring(executed)
            .. ' marker_present=' .. tostring(file_exists(marker_path)))
        if pass then
            verdict('pass')
        else
            fail('denied', 'denied approval did not block the call as required')
        end
    elseif scenario == 'unknown-tool' then
        local pending_before = #mods.approval.pending(sup.approvals)
        local result = gate(sup, mods, run.id, work_dir, {
            risk = 'process',
            tool = 'fs.exec',
            argv = { 'id' },
            workspace = work_dir,
        }, function(queue, approval_id)
            return mods.approval.decide(queue, approval_id, 'approved', STAND_IN)
        end)
        local pending_after = #mods.approval.pending(sup.approvals)
        local decision = result.decision
        local pass = (not result.allowed)
            and result.phase == 'policy-deny'
            and decision ~= nil
            and decision.decision == 'deny'
            and decision.reason == 'no rule matched'
            and pending_after == pending_before
            and (not executed)
        note('pending approvals before=' .. pending_before .. ' after=' .. pending_after
            .. ' (no approval requested for unknown tool)')
        if pass then
            verdict('pass')
        else
            fail('unknown-tool', 'unknown tool was not rejected cleanly')
        end
    else
        fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. tostring(scenario))
    end
end

local ok, err = pcall(main)
if not ok then
    fail('panic', 'driver raised: ' .. tostring(err))
end
