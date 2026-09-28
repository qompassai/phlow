-- task-17 driver: harness-driven edit -> check -> fix loop.
--
-- Simulates what a harness-driven agent does when it edits a Lua file:
--   (1) edit   write the target file into GAUNTLET_WORK_DIR
--   (2) check  run `luac -p` on it through job control (vim.system) and
--              parse the syntax error out of stderr
--   (3) fix    apply the scenario's fix, re-check, and repeat up to
--              FIX_ATTEMPTS_MAX times
--   (4) verify the loop passes only when luac exits 0
--
-- Two safety properties are structural, not just asserted:
--   * the checker never executes the file — `luac -p` parses only, so a
--     hostile or broken edit cannot run code through this loop;
--   * fixes are bounded — after FIX_ATTEMPTS_MAX unsuccessful attempts the
--     loop stops and reports honest failure instead of looping forever.
--
-- Scenarios via GAUNTLET_SCENARIO (default "default"):
--   default     one syntax error -> one fix -> clean
--   bad-fix     the first fix is itself broken (a *different* error); the
--               loop must catch the new error and keep going -> clean
--   no-converge every fix is broken (the error oscillates between two
--               messages); the loop must stop bounded and fail honestly
--
-- Progress renders as operator-visible stage lines on stdout, so the
-- operator sees what ran, what luac said, and what to do next. The single
-- JSON verdict line is printed last. Writes nothing outside
-- GAUNTLET_WORK_DIR.

local TARGET = 'target.lua'
local FIX_ATTEMPTS_MAX = 3
local LUAC_WAIT_MS = 10000
local ERR_CHARS_MAX = 512
local EVIDENCE_MAX = 64

-- The agent's first edit: a missing closing paren on the print call.
local BROKEN = table.concat({
    'local function greet(name)',
    '  return "hello, " .. name',
    'end',
    '',
    'print(greet("matt")',
    '',
}, '\n')

-- The correct fix.
local FIXED = table.concat({
    'local function greet(name)',
    '  return "hello, " .. name',
    'end',
    '',
    'print(greet("matt"))',
    '',
}, '\n')

-- A *different* syntax error: a fix that trades one error for another.
local BROKEN_V2 = table.concat({
    'local function greet(name)',
    '  return "hello, " .. name',
    'end',
    '',
    'print(greet("matt"))',
    'return +',
    '',
}, '\n')

---Fix plans per scenario: what each fix attempt writes.
---@type table<string, string[]>
local FIX_PLANS = {
    default = { FIXED },
    ['bad-fix'] = { BROKEN_V2, FIXED },
    -- Oscillates: v2 error, then the original error returns, then v2 again.
    -- A loop without a bound would spin here forever.
    ['no-converge'] = { BROKEN_V2, BROKEN, BROKEN_V2 },
}

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

---Operator-visible progress line: goes to stdout and to the verdict
---evidence, so the operator and the report see the same transcript.
---@param line string
local function show(line)
    io.stdout:write('[task-17] ' .. line .. '\n')
    ev('[task-17] ' .. line)
end

local function fail(where, how)
    return { id = 'task-17', outcome = 'fail', where = where, how = how, evidence = evidence }
end

local function pass()
    return { id = 'task-17', outcome = 'pass', evidence = evidence }
end

---@param path string
---@param content string
---@return string? err
local function write_file(path, content)
    local handle, open_err = io.open(path, 'w')
    if handle == nil then
        return 'cannot open ' .. path .. ': ' .. tostring(open_err)
    end
    handle:write(content)
    handle:close()
    return nil
end

---@param content string
---@return integer
local function count_lines(content)
    local n = 0
    for _ in content:gmatch('\n') do
        n = n + 1
    end
    return n
end

---@param stderr string
---@return integer? line
---@return string msg
local function parse_luac_error(stderr)
    local first = stderr:match('[^\n]*')
    local line, msg = first:match(':(%d+):%s*(.-)%s*$')
    if line == nil then
        return nil, first:sub(1, ERR_CHARS_MAX)
    end
    return tonumber(line), msg:sub(1, ERR_CHARS_MAX)
end

---@param luac string
---@param path string
---@return boolean clean
---@return integer? line
---@return string msg
local function luac_check(luac, path)
    local proc = vim.system({ luac, '-p', path }, { text = true })
    local done = proc:wait(LUAC_WAIT_MS)
    if done == nil then
        return false, nil, 'luac timed out after ' .. LUAC_WAIT_MS .. ' ms'
    end
    if done.code == 0 then
        return true, nil, ''
    end
    local line, msg = parse_luac_error(done.stderr or '')
    return false, line, msg
end

---@param h table
---@return table? verdict
local function run_loop(h)
    -- Stage 1: the agent's edit.
    local write_err = write_file(h.target, BROKEN)
    if write_err ~= nil then
        return fail('edit', write_err)
    end
    show('scenario=' .. h.scenario)
    show('stage=edit file=' .. TARGET .. ' lines=' .. count_lines(BROKEN))

    local checks = 0
    local fixes = 0

    ---Run one check; render the outcome for the operator.
    ---@return boolean clean
    ---@return integer? line
    ---@return string msg
    local function check(attempt)
        checks = checks + 1
        show('stage=check attempt=' .. attempt .. ' cmd="luac -p ' .. TARGET .. '"')
        local clean, line, msg = luac_check(h.luac, h.target)
        if clean then
            show('luac-clean file=' .. TARGET)
        else
            show(
                'luac-error file=' .. TARGET
                    .. ' line=' .. tostring(line)
                    .. ' msg=' .. msg
            )
            show('hint: fix the error above, then re-run luac')
        end
        return clean, line, msg
    end

    -- Stage 2: the first check must surface the syntax error.
    local clean, line, msg = check(1)
    if clean then
        return fail('check', 'luac reported the broken file clean: the check is unsound')
    end
    if line == nil or msg == '' then
        return fail('check', 'luac error did not name a line and message')
    end

    -- Stages 3..n: fix and re-check, bounded.
    local plan = FIX_PLANS[h.scenario]
    for attempt = 1, FIX_ATTEMPTS_MAX do
        local fix_content = plan[attempt]
        if fix_content == nil then
            return fail(
                'fix',
                'fix plan for scenario ' .. h.scenario .. ' ran out at attempt ' .. attempt
            )
        end
        fixes = fixes + 1
        write_err = write_file(h.target, fix_content)
        if write_err ~= nil then
            return fail('fix', write_err)
        end
        show('stage=fix attempt=' .. attempt .. ' file=' .. TARGET .. ' lines=' .. count_lines(fix_content))
        clean, line, msg = check(attempt + 1)
        if clean then
            show('result: pass checks=' .. checks .. ' fixes=' .. fixes)
            return pass()
        end
        -- Not clean: the loop saw the *new* error and keeps going instead
        -- of trusting the fix. The transcript above already shows it.
    end

    -- Bounded stop: the fixes never converged. Honest failure carries the
    -- last luac error so the operator knows exactly where it stands.
    return fail(
        'fix',
        'fix did not converge after '
            .. FIX_ATTEMPTS_MAX
            .. ' attempts; last luac error: '
            .. TARGET
            .. ':'
            .. tostring(line)
            .. ': '
            .. msg
    )
end

---@return table? h
---@return string? err
local function bootstrap()
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    if type(work_dir) ~= 'string' or work_dir == '' then
        return nil, 'GAUNTLET_WORK_DIR is not set'
    end
    local luac = vim.env.GAUNTLET_LUAC_BIN
    if type(luac) ~= 'string' or luac == '' then
        luac = 'luac'
    end
    if vim.fn.executable(luac) ~= 1 then
        return nil, 'luac not executable: ' .. luac
    end
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'default'
    end
    if FIX_PLANS[scenario] == nil then
        return nil, 'unknown GAUNTLET_SCENARIO: ' .. scenario
    end
    show('luac-bin=' .. luac)
    return {
        luac = luac,
        scenario = scenario,
        target = work_dir .. '/' .. TARGET,
    }
end

local function main()
    local h, boot_err = bootstrap()
    if h == nil then
        return fail('bootstrap', boot_err)
    end
    return run_loop(h)
end

local ok, verdict = pcall(main)
if not ok then
    ev('lua error: ' .. tostring(verdict))
    verdict = fail('lua-driver', 'unhandled error: ' .. tostring(verdict))
end
-- Verdict on the real stdout: in `nvim --headless -l`, Lua print() goes to
-- stderr, but the Rust runner parses the verdict from stdout.
io.stdout:write(vim.json.encode(verdict) .. '\n')
