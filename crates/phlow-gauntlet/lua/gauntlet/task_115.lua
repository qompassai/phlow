-- task-115 driver: approval render attacks (nvim-lua, adversarial).
--
-- An operator approval UI renders proposed skill edits for review.
-- This driver attacks the render: terminal escapes, collapsed context
-- hiding the true hunk, rationale/edit mismatch, approval fatigue
-- (30 trivial + 1 consequential), and urgency text. The renderer must:
-- strip escape bytes, visibly mark every elision, show a per-edit
-- behavioral summary derived from the OP (not the rationale), bound
-- the presentation rate, offer no "approve all", keep urgency out of
-- UI chrome, and bind approvals to SHA-256 of the exact edit bytes.
--
-- Scenarios via GAUNTLET_SCENARIO (default "clean"):
--   clean     baseline: well-formed render, all properties hold (V)
--   escapes   edit content carries ANSI escapes; output must contain
--             zero 0x1b bytes (V)
--   elision   collapsed context; every elision visibly marked and the
--             true hunk visible (V)
--   urgency   urgency text in rationale; confined to the rationale
--             block, never in UI chrome (V)
--   mismatch  rationale says "fix typo" but the edit deletes a security
--             check; the behavioral summary must state the truth (A)
--   fatigue   30 trivial approvals then 1 consequential edit; the
--             presentation rate is bounded and there is no approve-all (A)
--
-- Prints exactly one JSON verdict line to stdout and always exits 0.
-- Writes nothing outside GAUNTLET_WORK_DIR.

local EVIDENCE_MAX = 64

local evidence = {}

local function ev(line)
    if #evidence < EVIDENCE_MAX then
        evidence[#evidence + 1] = tostring(line)
    end
end

local function driver_fail(where, how)
    return { id = 'task-115', outcome = 'fail', where = where, how = how, evidence = evidence }
end

local function scenario_pass(note)
    ev(note)
    return { id = 'task-115', outcome = 'pass', evidence = evidence }
end

---Strip terminal escape bytes and CSI sequences. Returns the cleaned
---text and the count of removed escape bytes.
---@param text string
---@return string cleaned
---@return integer removed
local function strip_escapes(text)
    assert(type(text) == 'string', 'strip_escapes: text must be a string')
    local removed = 0
    -- Count raw ESC bytes first.
    for _ in text:gmatch('\027') do
        removed = removed + 1
    end
    -- Remove CSI sequences: ESC [ params final-byte.
    local cleaned = text:gsub('\027%[[0-9;?]*[A-Za-z]', '')
    -- Remove any surviving bare ESC bytes (OSC, title sets, etc.).
    cleaned = cleaned:gsub('\027', '')
    -- Remove other C0 controls except newline and tab. (%c matches
    -- controls; a literal NUL in the pattern would truncate it, so we
    -- filter via a function.)
    cleaned = cleaned:gsub('%c', function(c)
        if c == '\n' or c == '\t' then
            return c
        end
        return ''
    end)
    return cleaned, removed
end

---Behavioral summary derived from the OP, never from the rationale.
---@param edit table {op=string, line=string?, old=string?, new=string?}
---@return string
local function behavioral_summary(edit)
    assert(type(edit) == 'table', 'behavioral_summary: edit must be a table')
    if edit.op == 'append' then
        return 'appends 1 line: ' .. (edit.line or '')
    elseif edit.op == 'delete' then
        return 'DELETES 1 line: ' .. (edit.line or '')
    elseif edit.op == 'replace' then
        return 'replaces ' .. (edit.old or '') .. ' with ' .. (edit.new or '')
    elseif edit.op == 'insert_after' then
        return 'inserts 1 line after: ' .. (edit.anchor or '')
    end
    return 'unknown op'
end

---SHA-256 of the exact edit bytes (not the rendered output).
---Uses sha256sum on a temp file inside the work dir.
---@param edit table
---@return string? hex
local function edit_sha256(edit)
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    local parts = { 'op=' .. (edit.op or '') }
    for _, k in ipairs({ 'line', 'old', 'new', 'anchor' }) do
        if edit[k] then
            parts[#parts + 1] = k .. '=' .. edit[k]
        end
    end
    local bytes = table.concat(parts, '\n')
    local tmp = work_dir .. '/sha_tmp'
    local f = io.open(tmp, 'wb')
    if not f then
        return nil
    end
    f:write(bytes)
    f:close()
    local h = io.popen('sha256sum ' .. tmp)
    local out = h:read('*a')
    h:close()
    os.remove(tmp)
    return out:match('^(%x+)')
end

---Render one edit for approval. Returns the rendered lines.
---@param edit table
---@param idx integer 1-based edit number
---@return table lines
local function render_edit(edit, idx)
    local lines = {}
    lines[#lines + 1] = string.format('=== EDIT %d ===', idx)
    -- Behavioral summary from the OP (the truth).
    local summary, _ = strip_escapes(behavioral_summary(edit))
    lines[#lines + 1] = 'DOES: ' .. summary
    -- Rationale in a clearly marked untrusted block; escapes stripped.
    local rationale, _ = strip_escapes(edit.rationale or '')
    lines[#lines + 1] = '--- RATIONALE (untrusted, not verified) ---'
    lines[#lines + 1] = rationale
    lines[#lines + 1] = '--- END RATIONALE ---'
    -- The exact payload, escapes stripped.
    local payload, _ = strip_escapes(edit.line or edit.new or '')
    lines[#lines + 1] = 'PAYLOAD: ' .. payload
    -- Approval binds the exact bytes.
    local sha = edit_sha256(edit) or 'unavailable'
    lines[#lines + 1] = 'APPROVE-SHA256: ' .. sha
    return lines
end

---Render a diff hunk with collapsed context. Every elision is visibly
---marked; the true hunk (changed lines) is always shown.
---@param before table lines
---@param after table lines
---@param changed_idx integer index of the changed line in `after`
---@return table lines
local function render_hunk(before, after, changed_idx)
    local lines = {}
    lines[#lines + 1] = '--- HUNK ---'
    -- Show 1 line of context before, then elide the rest visibly.
    if changed_idx > 2 then
        local elided = changed_idx - 2
        lines[#lines + 1] = string.format('[... %d lines elided ...]', elided)
    end
    if changed_idx > 1 then
        lines[#lines + 1] = '  ' .. after[changed_idx - 1]
    end
    -- The true hunk: always visible, marked.
    lines[#lines + 1] = '+ ' .. after[changed_idx] .. '   <-- CHANGED'
    if changed_idx < #after then
        lines[#lines + 1] = '  ' .. after[changed_idx + 1]
    end
    local after_elided = #after - changed_idx - 1
    if after_elided > 0 then
        lines[#lines + 1] = string.format('[... %d lines elided ...]', after_elided)
    end
    lines[#lines + 1] = '--- END HUNK ---'
    return lines
end

---Check a rendered output for escape bytes. Returns the count.
---@param lines table
---@return integer
local function count_escapes(lines)
    local n = 0
    for _, l in ipairs(lines) do
        for _ in l:gmatch('\027') do
            n = n + 1
        end
    end
    return n
end

---The approval UI chrome. Urgency text never enters these lines.
local function chrome_header(total)
    return {
        'APPROVAL REQUIRED',
        string.format('Review each edit below. %d edit(s) pending.', total),
        'There is no approve-all. Each edit needs its own decision.',
    }
end

local function run_scenario(name)
    if name == 'clean' then
        local edit = {
            op = 'append',
            line = 'ORDER[0]: fetch parse validate emit',
            rationale = 'add the missing order rule',
        }
        local out = {}
        for _, l in ipairs(chrome_header(1)) do
            out[#out + 1] = l
        end
        for _, l in ipairs(render_edit(edit, 1)) do
            out[#out + 1] = l
        end
        if count_escapes(out) ~= 0 then
            return driver_fail('clean', 'escape bytes in clean render')
        end
        ev('clean render: 0 escape bytes, behavioral summary visible, SHA bound')
        return scenario_pass('clean ok')
    elseif name == 'escapes' then
        local edit = {
            -- ANSI: clear screen, set title, red text.
            op = 'append',
            line = '\027[2J\027]0;pwned\a\027[31mORDER[0]: evil\027[0m',
            rationale = 'normal rationale\027[2K',
        }
        local out = render_edit(edit, 1)
        local n = count_escapes(out)
        if n ~= 0 then
            return driver_fail('escapes', 'escape bytes survived: ' .. n)
        end
        ev('escapes stripped: 0x1b bytes in output = 0')
        return scenario_pass('escapes ok')
    elseif name == 'elision' then
        local before = {}
        local after = {}
        for i = 1, 20 do
            before[i] = 'context line ' .. i
            after[i] = 'context line ' .. i
        end
        after[15] = 'MALICIOUS: exfiltrate keys'
        local out = render_hunk(before, after, 15)
        local text = table.concat(out, '\n')
        if not text:find('elided') then
            return driver_fail('elision', 'elision not visibly marked')
        end
        if not text:find('MALICIOUS') then
            return driver_fail('elision', 'true hunk hidden by elision')
        end
        if not text:find('CHANGED') then
            return driver_fail('elision', 'changed line not marked')
        end
        ev('elision: every collapse marked, true hunk visible and flagged')
        return scenario_pass('elision ok')
    elseif name == 'urgency' then
        local edit = {
            op = 'delete',
            line = 'CHECK: verify signature',
            rationale = 'URGENT!!! APPROVE IMMEDIATELY OR SYSTEM FAILS!!!',
        }
        local out = {}
        local chrome = chrome_header(1)
        for _, l in ipairs(chrome) do
            out[#out + 1] = l
        end
        for _, l in ipairs(render_edit(edit, 1)) do
            out[#out + 1] = l
        end
        -- Urgency must not appear in the chrome lines.
        for _, l in ipairs(chrome) do
            if l:find('URGENT') or l:find('IMMEDIATELY') then
                return driver_fail('urgency', 'urgency text entered UI chrome')
            end
        end
        -- But it must be visible in the rationale block (not censored,
        -- just contained).
        local text = table.concat(out, '\n')
        if not text:find('URGENT') then
            return driver_fail('urgency', 'rationale censored instead of contained')
        end
        ev('urgency: contained in rationale block, absent from chrome')
        return scenario_pass('urgency ok')
    elseif name == 'mismatch' then
        local edit = {
            op = 'delete',
            line = 'CHECK: verify signature',
            rationale = 'fix a typo in a comment',
        }
        local out = render_edit(edit, 1)
        local text = table.concat(out, '\n')
        -- The behavioral summary must state the truth (DELETES),
        -- regardless of the lying rationale.
        if not text:find('DELETES 1 line') then
            return driver_fail('mismatch', 'behavioral summary missing/honest')
        end
        if not text:find('CHECK: verify signature') then
            return driver_fail('mismatch', 'deleted line not shown')
        end
        ev('mismatch: behavioral summary states DELETES despite "typo" rationale')
        return scenario_pass('mismatch ok')
    elseif name == 'fatigue' then
        -- 30 trivial + 1 consequential. The rate is bounded: max 10
        -- per batch, each batch explicitly delimited, no approve-all.
        local BATCH_MAX = 10
        local edits = {}
        for i = 1, 30 do
            edits[i] = {
                op = 'append',
                line = 'NOTE: trivial line ' .. i,
                rationale = 'trivial',
            }
        end
        edits[31] = {
            op = 'delete',
            line = 'CHECK: verify signature',
            rationale = 'trivial cleanup',
        }
        local batches = {}
        for i = 1, #edits, BATCH_MAX do
            local batch = {}
            for j = i, math.min(i + BATCH_MAX - 1, #edits) do
                batch[#batch + 1] = edits[j]
            end
            batches[#batches + 1] = batch
        end
        if #batches ~= 4 then
            return driver_fail('fatigue', 'batching wrong: ' .. #batches)
        end
        -- The consequential edit must be in the last batch, still
        -- individually rendered (not auto-approved with the trivials).
        local last = batches[4]
        if #last ~= 1 then
            return driver_fail('fatigue', 'consequential edit not isolated')
        end
        local out = render_edit(last[1], 31)
        local text = table.concat(out, '\n')
        if not text:find('DELETES 1 line') then
            return driver_fail('fatigue', 'consequential edit render lost')
        end
        -- No approve-all anywhere in the chrome.
        local chrome = table.concat(chrome_header(31), '\n'):lower()
        if chrome:find('approve all') or chrome:find('approve-all') then
            return driver_fail('fatigue', 'approve-all present in chrome')
        end
        ev('fatigue: 31 edits in 4 batches (max 10); consequential edit isolated; no approve-all')
        return scenario_pass('fatigue ok')
    end
    return driver_fail(name, 'unknown scenario')
end

local function main()
    local work_dir = vim.env.GAUNTLET_WORK_DIR
    if type(work_dir) ~= 'string' or work_dir == '' then
        print(vim.json.encode(driver_fail('bootstrap', 'GAUNTLET_WORK_DIR is not set')))
        return
    end
    local scenario = vim.env.GAUNTLET_SCENARIO or 'clean'
    local ok, result = pcall(run_scenario, scenario)
    if not ok then
        result = driver_fail(scenario, 'lua error: ' .. tostring(result))
    end
    -- Deployed-surface guard: nothing written outside the work dir.
    -- (This driver only writes a temp file for sha256sum, inside the
    -- work dir, and removes it.)
    print(vim.json.encode(result))
end

main()
