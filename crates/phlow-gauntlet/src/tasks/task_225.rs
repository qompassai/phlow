//! task-225: approval permission delta.
//!
//! Honest scope: Approval.request/get must expose a computed permission_delta with sorted
//! added/removed arrays. The before/after request fields and output contract are desired
//! extensions; the real queue currently drops them.
//! Fixtures use installed diver-fixed modules without changing either Neovim config.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-225";
/// Desired invariant.
pub const NAME: &str = "approval permission delta";
/// Runs the actual fixed-config Neovim modules.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "empty_to_empty",
    "unchanged_permission",
    "addition_visible",
    "removal_visible",
];

const PROBES: [&str; 4] = [
    r#"
local request = req()
request.permissions_before = {}
request.permissions_after = {}

local queue = a.new()
local id, err = a.request(queue, "delta-run", request)
assert(id, err)
local record = a.get(queue, id)
return vim.deep_equal(record.permission_delta, { added = {}, removed = {} })
"#,
    r#"
local request = req()
request.permissions_before = { "fs.read" }
request.permissions_after = { "fs.read" }

local queue = a.new()
local id, err = a.request(queue, "delta-run", request)
assert(id, err)
local record = a.get(queue, id)
return vim.deep_equal(record.permission_delta, { added = {}, removed = {} })
"#,
    r#"
local request = req()
request.permissions_before = {}
request.permissions_after = { "fs.write" }

local queue = a.new()
local id, err = a.request(queue, "delta-run", request)
assert(id, err)
local record = a.get(queue, id)
return vim.deep_equal(record.permission_delta, { added = { "fs.write" }, removed = {} })
"#,
    r#"
local request = req()
request.permissions_before = { "fs.write" }
request.permissions_after = {}

local queue = a.new()
local id, err = a.request(queue, "delta-run", request)
assert(id, err)
local record = a.get(queue, id)
return vim.deep_equal(record.permission_delta, { added = {}, removed = { "fs.write" } })
"#,
];

/// Run all four cases with bounded subprocess execution and per-case evidence.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    super::task_225::run_probes(ctx, ID, &CASES, &PROBES)
}

#[cfg(test)]
mod tests {
    use super::{CASES, PROBES};

    #[test]
    fn empty_to_empty() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn unchanged_permission() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn addition_visible() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn removal_visible() {
        let result = super::super::task_225::probe(std::path::Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}

// Shared fixtures contain no authorization or delta implementation.
const PRELUDE: &str = r#"
local a = require("ai.harness.approval")
local p = require("ai.harness.policy")
local events = require("ai.harness.events")
local function req()
	return { tool = "fs.write", risk = "local_reversible", paths = { "/work/a" }, workspace = "/work" }
end
local function rule_for(decision)
	return { risk = "local_reversible", decision = decision, tools = { "fs.write" }, paths = { "/work/a" } }
end
local function policy(decision)
	local state, err = p.new({ default = "deny", rules = { rule_for(decision) } })
	assert(state, err)
	return state
end
local function pending(request)
	local queue = a.new()
	local id, err = a.request(queue, "run", request or req())
	assert(id, err)
	return queue, id
end
"#;

/// Execute one real-module probe; false is an assertion failure, errors are driver failures.
/// Keep fixed-config cache/state and a copy of its package lock in an owned temp directory:
/// normal startup otherwise tries to write outside the sandbox. Config/modules stay unchanged.
pub(super) fn probe(nvim: &std::path::Path, body: &str) -> Result<bool, String> {
    use std::process::{Command, Stdio};
    use std::time::Duration;
    const SCRIPT_BYTES_MAX: usize = 16_384;
    if PRELUDE.len() + body.len() > SCRIPT_BYTES_MAX {
        return Err("static probe exceeds script budget".to_owned());
    }
    let fixture = super::task_233::Fixture::new()?;
    let lock_source = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or("HOME missing for fixed-config lockfile")?
        .join(".config/diver-fixed/nvim-pack-lock.json");
    if std::fs::metadata(&lock_source)
        .map_err(super::task_233::err)?
        .len()
        > 1_048_576
    {
        return Err("fixed-config lockfile exceeds one MiB".to_owned());
    }
    let lock_copy = fixture.path.join("nvim-pack-lock.json");
    std::fs::copy(lock_source, &lock_copy).map_err(super::task_233::err)?;
    let script = format!("{PRELUDE}\n{body}");
    let command = "lua local f = loadstring(vim.env.PHLOW_DELTA_PROBE); \
        if not f then vim.cmd('cquit 4') else local ok, result = pcall(f); \
        if not ok then vim.cmd('cquit 4') elseif result ~= true then \
        vim.cmd('cquit 3') else vim.cmd('qa!') end end";
    let child = Command::new(nvim)
        .args([
            "--headless",
            "-i",
            "NONE",
            "--cmd",
            "lua vim.o.packlockfile = vim.env.PHLOW_PROBE_LOCK",
            "-c",
            command,
        ])
        .env("PHLOW_PROBE_LOCK", lock_copy)
        .env("XDG_CACHE_HOME", fixture.path.join("cache"))
        .env("XDG_STATE_HOME", fixture.path.join("state"))
        .env("NVIM_APPNAME", "diver-fixed")
        .env("PHLOW_DELTA_PROBE", script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("fixed-config Neovim spawn: {error}"))?;
    match crate::wait_for_child(child, Duration::from_secs(15))? {
        crate::WaitOutcome::TimedOut => Err("fixed-config probe exceeded 15 seconds".to_owned()),
        crate::WaitOutcome::Finished { status, output } => {
            output?;
            match status.code() {
                Some(0) => Ok(true),
                Some(3) => Ok(false),
                code => Err(format!("probe fixture/runtime error: exit {code:?}")),
            }
        }
    }
}

pub(super) fn run_probes(
    ctx: &Ctx,
    id: &'static str,
    cases: &[&str; 4],
    probes: &[&str; 4],
) -> TaskOutcome {
    let results = cases
        .iter()
        .zip(probes)
        .map(|(case, body)| (*case, probe(&ctx.nvim_bin, body)));
    super::task_233::collect_outcome(id, results)
}
