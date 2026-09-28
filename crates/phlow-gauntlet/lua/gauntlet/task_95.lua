-- task_95.lua -- gauntlet driver: approval render integrity (WYSIWYG).
--
-- The design asks for the approval UI to render the diff from the exact
-- bytes the approval will bind to (what you see is what you sign):
-- terminal escape sequences in diffs are stripped/neutralized with a
-- warning; every elision gets a visible marker while the approval binds
-- the full bytes regardless of what's shown; and a semantic-change
-- summary (added/removed identifiers, changed control flow) is always
-- shown beside the author's rationale.
--
-- Seam mapping (verified, not invented): the lua layer has NO proposal
-- approval render UI. `ai.harness.approval` is a data-only queue (id,
-- tool, risk, summary, argv, paths, endpoints; state pending/approved/
-- denied/expired) — its own header says "The single approval surface
-- renders from this queue", but no render function exists in lua/ai:
-- the surface lives outside the lua tree. No module renders a proposal
-- diff, neutralizes terminal escapes, marks elisions, or builds a
-- semantic-change summary. The approval record carries a free-text
-- `summary`, never diff bytes, so there is nothing for a WYSIWYG check
-- to bind to.
--
-- This driver probes the REAL approval queue headless and runs bounded
-- token scans over the diver lua/ai tree:
--   ui         V: the approval queue has no render API; the record
--                carries no diff bytes.
--   diff       V: no diff-render pipeline exists in the approval path
--                (token scan: diff_render/render_diff/approval_ui).
--   escapes    A: no escape-neutralization exists (token scan:
--                strip_escapes/neutralize/sanitize_render).
--   elision    A: no elision marking exists (token scan:
--                elision/collapsed_hunk/hunk_marker).
--
-- All scenarios write machine-readable traces into GAUNTLET_WORK_DIR
-- (render-trace.json) for the Rust harness probes, print exactly one
-- JSON verdict line to stdout, always exit 0, write nothing outside
-- GAUNTLET_WORK_DIR, and never modify the diver repo.
--
-- Scenarios via GAUNTLET_SCENARIO (default "ui"):
--   ui         V: approval queue has no render API.
--   diff       V: no diff-render pipeline in the approval path.
--   escapes    A: no escape neutralization.
--   elision    A: no elision marking.

local TASK_ID = 'task-95'
local TRACE_NAME = 'render-trace.json'

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

---Require a real diver module.
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

---Scenario `ui` (V): the approval queue is data-only — no render API —
---and the approval record carries no diff bytes.
local function scenario_ui(handles)
    local approval, err = require_diver('ai.harness.approval')
    if approval == nil then
        return fail('bootstrap', err)
    end
    ev('driving real module: ai.harness.approval')
    local render_api = false
    for _, field in ipairs({ 'render', 'render_diff', 'show', 'display', 'format_diff' }) do
        if approval[field] ~= nil then
            render_api = true
            ev('UNEXPECTED render API on ai.harness.approval: ' .. field)
        end
    end
    local queue = approval.new()
    local id, rerr = approval.request(queue, 'run-1', {
        tool = 'write',
        risk = 'irreversible',
        summary = 'whitespace only',
    })
    if id == nil then
        return fail('scenario', 'approval.request failed: ' .. tostring(rerr))
    end
    local rec = approval.get(queue, id)
    local has_diff = rec ~= nil and (rec.diff ~= nil or rec.diff_bytes ~= nil)
    ev('approval record carries summary text only; diff bytes present: ' .. tostring(has_diff))
    local werr = write_trace(handles, {
        scenario = 'ui',
        render_api = render_api,
        record_has_diff = has_diff,
    })
    if werr ~= nil then
        return fail('trace', werr)
    end
    if render_api or has_diff then
        return fail('ui', 'a render API or diff bytes appeared on the approval queue (premise changed)')
    end
    return pass()
end

---Scenario `diff` (V): no diff-render pipeline exists in the approval
---path anywhere in lua/ai.
local function scenario_diff(handles)
    local hits, hit_paths = scan_ai_tree(handles, { 'diff_render', 'render_diff', 'approval_ui' })
    ev('token scan for diff_render/render_diff/approval_ui over lua/ai: ' .. hits .. ' hit(s)')
    for _, hp in ipairs(hit_paths) do
        ev('  hit: ' .. hp)
    end
    local werr = write_trace(handles, {
        scenario = 'diff',
        render_hits = hits,
    })
    if werr ~= nil then
        return fail('trace', werr)
    end
    if hits ~= 0 then
        return fail('diff', 'render vocabulary appeared in lua/ai (premise changed)')
    end
    return pass()
end

---Scenario `escapes` (A): no escape-neutralization exists — there is no
---renderer whose output could hide lines or rewrite displayed text.
local function scenario_escapes(handles)
    local hits, hit_paths = scan_ai_tree(handles, { 'strip_escapes', 'neutralize', 'sanitize_render' })
    ev('token scan for strip_escapes/neutralize/sanitize_render over lua/ai: ' .. hits .. ' hit(s)')
    for _, hp in ipairs(hit_paths) do
        ev('  hit: ' .. hp)
    end
    local werr = write_trace(handles, {
        scenario = 'escapes',
        escape_hits = hits,
    })
    if werr ~= nil then
        return fail('trace', werr)
    end
    if hits ~= 0 then
        return fail('escapes', 'escape-handling vocabulary appeared in lua/ai (premise changed)')
    end
    return pass()
end

---Scenario `elision` (A): no elision marking exists — nothing collapses
---hunks, so no marker discipline can be verified.
local function scenario_elision(handles)
    local hits, hit_paths = scan_ai_tree(handles, { 'elision', 'collapsed_hunk', 'hunk_marker' })
    ev('token scan for elision/collapsed_hunk/hunk_marker over lua/ai: ' .. hits .. ' hit(s)')
    for _, hp in ipairs(hit_paths) do
        ev('  hit: ' .. hp)
    end
    local werr = write_trace(handles, {
        scenario = 'elision',
        elision_hits = hits,
    })
    if werr ~= nil then
        return fail('trace', werr)
    end
    if hits ~= 0 then
        return fail('elision', 'elision vocabulary appeared in lua/ai (premise changed)')
    end
    return pass()
end

local function main()
    local scenario = vim.env.GAUNTLET_SCENARIO
    if type(scenario) ~= 'string' or scenario == '' then
        scenario = 'ui'
    end
    local handles, boot_err = bootstrap()
    if handles == nil then
        return fail('bootstrap', boot_err)
    end
    ev('scenario=' .. scenario)
    if scenario == 'ui' then
        return scenario_ui(handles)
    elseif scenario == 'diff' then
        return scenario_diff(handles)
    elseif scenario == 'escapes' then
        return scenario_escapes(handles)
    elseif scenario == 'elision' then
        return scenario_elision(handles)
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
