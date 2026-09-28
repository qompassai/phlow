-- task_56.lua -- gauntlet driver: approval timeout defaults deny.
--
-- Drives the REAL harness approval-expiry seam (the task-03 pattern):
--   * ai.harness.approval -- request / decide / get / sweep_expired.
--     Requests expire to denied-equivalent ('expired'), never to approved;
--     only pending approvals can be decided.
--   * ai.harness.supervisor.tick -- drives sweep_expired, the expiry path
--     the design names.
--
-- Scenarios (GAUNTLET_SCENARIO):
--   responds-in-time            (V) approver approves before the deadline
--                               -> approved, the gated tool proceeds.
--   terminal-within-deadline    (V) nobody responds; the request reaches a
--                               terminal state by deadline + epsilon, never
--                               stuck pending.
--   timeout-never-grants        (A) deadline passes with no response ->
--                               NOT granted, NOT left pending; the gate
--                               blocks the tool. Hunts the classic
--                               default-allow-on-timeout bug.
--   late-approval-cannot-resurrect (A) decide('approved') after expiry ->
--                               rejected ('already expired'); the record
--                               stays expired.
--
-- The mock approver is assigned but never responds: it sleeps past the
-- deadline by simply never calling approval.decide, exactly the design's
-- "approver is assigned but never responds" dimension. There is no live
-- human backend in the sandbox; the stand-in acts ONLY through the real
-- approval.decide API and is named in every verdict's evidence.
--
-- HONEST NAMING NOTE: the design's pass criteria say "denied", and the
-- module header comment says "Requests expire to denied, never to
-- approved" — but the code's terminal state is named 'expired'
-- (type: 'pending'|'approved'|'denied'|'expired'). 'expired' is the
-- deny-equivalent: terminal, never transitions to 'approved', decide()
-- rejects it, and the gate treats anything ~= 'approved' as blocked. The
-- driver asserts the SECURITY property (default-deny on timeout) and
-- records the doc/code naming gap in evidence; it does not invent a
-- 'denied' state the module never produces.
--
-- Prints exactly one JSON verdict line and always exits 0.

local EVIDENCE_LINES_MAX = 64
local APPROVAL_TIMEOUT_MS = 1000
local MARKER_NAME = 'tool-executed.txt'
local MARKER_TEXT = 'task-56: approved tool call executed\n'
local STAND_IN = 'task-56:stand-in-approver'

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
    local v = { id = 'task-56', outcome = outcome, evidence = evidence }
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

---@param sup table real harness supervisor (owns .approvals)
---@param mods table {approval=, supervisor=}
---@param run_id string
---@param opts table {decide_before_tick: 'approved'|'denied'|nil}
---@return table result {approval_id=, record=, expired_by_tick=}
local function request_approval(sup, mods, run_id, opts)
    local approval_mod = mods.approval
    local approval_id, req_err = approval_mod.request(sup.approvals, run_id, {
        risk = 'local_reversible',
        tool = 'fs.write',
        paths = { 'marker' },
    }, { timeout_ms = APPROVAL_TIMEOUT_MS })
    if approval_id == nil then
        fail('request', 'approval.request failed: ' .. tostring(req_err))
    end
    note('approval.request: id=' .. approval_id .. ' state=pending')
    if opts.decide_before_tick ~= nil then
        local ok, decide_err = approval_mod.decide(
            sup.approvals, approval_id, opts.decide_before_tick, STAND_IN)
        if not ok then
            fail('decide', 'stand-in decide failed: ' .. tostring(decide_err))
        end
        note('stand-in approver decided ' .. opts.decide_before_tick .. ' before deadline')
    else
        note('mock approver assigned but silent: never calls decide (sleeps past deadline)')
    end
    local record = approval_mod.get(sup.approvals, approval_id)
    return { approval_id = approval_id, record = record }
end

local function main()
    local diver_dir = vim.env.DIVER_LUA_DIR
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    local scenario = vim.env.GAUNTLET_SCENARIO or 'responds-in-time'
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
    local sup = harness._state and harness._state.supervisor
    if sup == nil then
        fail('setup', 'harness supervisor unavailable after setup')
    end
    local mods = {
        approval = require('ai.harness.approval'),
        supervisor = require('ai.harness.supervisor'),
    }

    local run, run_err = mods.supervisor.create(sup, {
        workflow = 'gauntlet',
        goal = 'task-56: approval timeout defaults deny',
        workspace = work_dir,
        budget = { turn = 50, tool_call = 200, token = 200000, time_ms = 3600000, byte = 10000000 },
    })
    if run == nil then
        fail('run', 'supervisor.create failed: ' .. tostring(run_err))
    end
    note('run created: id=' .. run.id .. ' state=' .. run.state)

    local marker_path = work_dir .. '/' .. MARKER_NAME

    if scenario == 'responds-in-time' then
        -- V: the approver answers before the deadline -> approved, tool proceeds.
        local req = request_approval(sup, mods, run.id, { decide_before_tick = 'approved' })
        mods.supervisor.tick(sup, req.record.deadline_ns + 1)
        local record = mods.approval.get(sup.approvals, req.approval_id)
        note('tick past deadline: state=' .. record.state)
        if record.state ~= 'approved' then
            fail('responds-in-time', 'timely approval did not hold: state=' .. record.state)
        end
        local exec_ok, exec_err = execute_tool(work_dir)
        if not exec_ok then
            fail('responds-in-time', 'tool execution failed: ' .. tostring(exec_err))
        end
        if executed and file_exists(marker_path) and record.decided_by == STAND_IN then
            verdict('pass')
        else
            fail('responds-in-time', 'approved call did not proceed as required')
        end
    elseif scenario == 'terminal-within-deadline' then
        -- V: nobody responds; the request reaches a terminal state by
        -- deadline + epsilon — never stuck pending. The epsilon boundary
        -- is pinned: still pending at deadline - 1, terminal at deadline.
        local req = request_approval(sup, mods, run.id, {})
        mods.supervisor.tick(sup, req.record.deadline_ns - 1)
        local before = mods.approval.get(sup.approvals, req.approval_id)
        note('tick(deadline - 1ns): state=' .. before.state)
        if before.state ~= 'pending' then
            fail('terminal-within-deadline',
                'request left pending before the deadline: state=' .. before.state)
        end
        mods.supervisor.tick(sup, req.record.deadline_ns)
        local after = mods.approval.get(sup.approvals, req.approval_id)
        note('tick(deadline): state=' .. after.state)
        local terminal = after.state == 'approved'
            or after.state == 'denied'
            or after.state == 'expired'
        if after.state ~= 'pending' and terminal then
            note('naming: module header says "expire to denied"; code state is \'expired\' '
                .. '(deny-equivalent: terminal, never approved, decide() rejects it)')
            verdict('pass')
        else
            fail('terminal-within-deadline',
                'request not terminal at deadline: state=' .. after.state)
        end
    elseif scenario == 'timeout-never-grants' then
        -- A: the deadline passes with no response. The classic bug this
        -- hunts is default-allow on timeout. Required: NOT granted, NOT
        -- left pending forever, and the gate blocks the tool.
        local req = request_approval(sup, mods, run.id, {})
        mods.supervisor.tick(sup, req.record.deadline_ns + 1)
        local record = mods.approval.get(sup.approvals, req.approval_id)
        note('tick past deadline: state=' .. record.state)
        if record.state == 'approved' then
            fail('timeout-never-grants', 'TIMEOUT GRANTED THE REQUEST: default-allow bug present')
        end
        if record.state == 'pending' then
            fail('timeout-never-grants', 'request left pending forever past the deadline')
        end
        -- The gate: anything ~= 'approved' blocks the tool (positive proof).
        if record.state ~= 'approved' then
            note('gate: state ~= approved -> tool call BLOCKED')
        end
        if executed or file_exists(marker_path) then
            fail('timeout-never-grants', 'tool executed without approval')
        end
        note('executed=' .. tostring(executed) .. ' marker_present='
            .. tostring(file_exists(marker_path)) .. ' (positive proof the tool never ran)')
        verdict('pass')
    elseif scenario == 'late-approval-cannot-resurrect' then
        -- A: the approver's response arrives AFTER the expiry. The late
        -- 'approved' must be rejected explicitly; the record stays expired.
        local req = request_approval(sup, mods, run.id, {})
        mods.supervisor.tick(sup, req.record.deadline_ns + 1)
        local expired = mods.approval.get(sup.approvals, req.approval_id)
        note('tick past deadline: state=' .. expired.state)
        if expired.state == 'pending' or expired.state == 'approved' then
            fail('late-approval-cannot-resurrect',
                'precondition failed: state=' .. expired.state)
        end
        local decide_ok, decide_err = mods.approval.decide(
            sup.approvals, req.approval_id, 'approved', STAND_IN)
        -- decide() follows the Lua convention (true | nil, err): normalize
        -- to an explicit boolean for the evidence line.
        note('late decide(approved): ok=' .. tostring(decide_ok == true)
            .. ' err=' .. tostring(decide_err))
        if decide_ok then
            fail('late-approval-cannot-resurrect',
                'late approval RESURRECTED the request: decide() succeeded after expiry')
        end
        local after = mods.approval.get(sup.approvals, req.approval_id)
        if after.state ~= expired.state then
            fail('late-approval-cannot-resurrect',
                'record state changed after rejected late decide: ' .. after.state)
        end
        if not tostring(decide_err):find('already', 1, true) then
            fail('late-approval-cannot-resurrect',
                'rejection is not an explicit already-decided rejection: '
                    .. tostring(decide_err))
        end
        note('late approval rejected explicitly ("' .. tostring(decide_err)
            .. '"); record stays ' .. after.state .. '; tool never ran (executed='
            .. tostring(executed) .. ')')
        verdict('pass')
    else
        fail('scenario', 'unknown GAUNTLET_SCENARIO: ' .. tostring(scenario))
    end
end

local ok, err = pcall(main)
if not ok then
    fail('panic', 'driver raised: ' .. tostring(err))
end
