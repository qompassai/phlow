//! task-209: structured denial records.
//!
//! Honest scope: The policy decision is the closest denial seam. A denial must retain the
//! denied tool and scope as typed data; no event bus emission is claimed.
//! Fixtures use the installed diver-fixed modules, never mocks or live config edits.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-209";
/// Desired permission invariant.
pub const NAME: &str = "structured denial records";
/// Real policy and approval modules in fixed-config headless Neovim.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "default_denial_is_typed",
    "explicit_denial_has_reason",
    "denial_retains_tool",
    "denial_retains_exact_paths",
];

const PROBES: [&str; 4] = [
    r#"
local d = fixture.p.decide(fixture.p.new({}), fixture.req())
return type(d) == "table" and d.decision == "deny" and d.risk == "local_reversible"
"#,
    r#"
local d = fixture.p.decide(fixture.policy("deny"), fixture.req())
return d.decision == "deny" and type(d.reason) == "string" and #d.reason > 0
"#,
    r#"
local d = fixture.p.decide(nil, fixture.req())
return d.decision == "deny" and d.tool == "fs.write"
"#,
    r#"
local d = fixture.p.decide(nil, fixture.req())
return vim.deep_equal(d.paths, { "/work/a" })
"#,
];

/// Run four bounded real-seam probes; retain all case outcomes, including failures.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_probes(ctx, ID, &CASES, &PROBES)
}

#[cfg(test)]
mod tests {
    use super::{CASES, PROBES};
    use crate::tasks::task_209::probe;
    use std::path::Path;

    #[test]
    fn default_denial_is_typed() {
        let result = probe(Path::new("nvim"), PROBES[0]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[0]);
    }

    #[test]
    fn explicit_denial_has_reason() {
        let result = probe(Path::new("nvim"), PROBES[1]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[1]);
    }

    #[test]
    fn denial_retains_tool() {
        let result = probe(Path::new("nvim"), PROBES[2]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[2]);
    }

    #[test]
    fn denial_retains_exact_paths() {
        let result = probe(Path::new("nvim"), PROBES[3]);
        assert!(matches!(result, Ok(true)), "{}: {result:?}", CASES[3]);
    }
}

// Only fixture construction and subprocess/report mechanics are shared. All
// authorization, admission, decisions and reads call the installed real modules.
const PRELUDE: &str = r#"
local fixture = {}
fixture.p = require("ai.harness.policy")
fixture.a = require("ai.harness.approval")
function fixture.req()
	return { tool = "fs.write", risk = "local_reversible", paths = { "/work/a" }, workspace = "/work" }
end
function fixture.rule(decision)
	return { risk = "local_reversible", decision = decision, tools = { "fs.write" }, paths = { "/work/a" } }
end
function fixture.policy(decision)
	local result, err = fixture.p.new({ default = "deny", rules = { fixture.rule(decision) } })
	assert(result ~= nil, err)
	return result
end
function fixture.pending(request)
	local queue = fixture.a.new()
	local id, err = fixture.a.request(queue, "run", request or fixture.req())
	assert(id ~= nil, err)
	return queue, id
end
function fixture.split_policy()
	local first = fixture.rule("allow")
	local second = fixture.rule("allow")
	second.paths = { "/work/b" }
	local result, err = fixture.p.new({ default = "deny", rules = { first, second } })
	assert(result ~= nil, err)
	return result
end
"#;

/// Execute one static Lua fixture with a 15-second deadline and no output capture.
/// Exit 3 means invariant failure; exit 4 means fixture/runtime failure. Neither
/// errors nor timeouts are counted as successful denial. No shell is involved.
pub(super) fn probe(nvim: &std::path::Path, body: &str) -> Result<bool, String> {
    use std::process::{Command, Stdio};
    use std::time::Duration;

    const SCRIPT_BYTES_MAX: usize = 8192;
    if body.len() + PRELUDE.len() > SCRIPT_BYTES_MAX {
        return Err("probe exceeds static script budget".to_owned());
    }
    let script = format!("{PRELUDE}\n{body}");
    let command = "lua local f = loadstring(vim.env.PHLOW_APPROVAL_PROBE); \
        if not f then vim.cmd('cquit 4') else local ok, result = pcall(f); \
        if not ok then vim.cmd('cquit 4') elseif result ~= true then \
        vim.cmd('cquit 3') else vim.cmd('qa!') end end";
    let child = Command::new(nvim)
        .args(["--headless", "-i", "NONE", "-c", command])
        .env("NVIM_APPNAME", "diver-fixed")
        .env("PHLOW_APPROVAL_PROBE", script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot spawn fixed-config Neovim: {error}"))?;
    match crate::wait_for_child(child, Duration::from_secs(15))? {
        crate::WaitOutcome::TimedOut => Err("Neovim probe timed out".to_owned()),
        crate::WaitOutcome::Finished { status, output } => {
            output?;
            match status.code() {
                Some(0) => Ok(true),
                Some(3) => Ok(false),
                code => Err(format!("Neovim fixture/runtime failure: exit {code:?}")),
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
    let mut evidence = Vec::with_capacity(4);
    let mut failures = Vec::with_capacity(4);
    for (case, body) in cases.iter().zip(probes) {
        match probe(&ctx.nvim_bin, body) {
            Ok(true) => evidence.push(format!("{case}: pass")),
            Ok(false) => {
                evidence.push(format!("{case}: fail"));
                failures.push(format!("{case}: desired invariant did not hold"));
            }
            Err(error) => {
                evidence.push(format!("{case}: driver error"));
                failures.push(format!("{case}: {error}"));
            }
        }
    }
    let evidence = crate::bound_evidence(evidence);
    if failures.is_empty() {
        TaskOutcome::Pass { evidence }
    } else {
        TaskOutcome::Fail {
            where_: id.to_owned(),
            how: failures.join("; "),
            evidence,
        }
    }
}
