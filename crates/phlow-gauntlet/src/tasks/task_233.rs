//! task-233: canonical binary pin records.
//!
//! Honest scope: Runtime construction is the operator-config admission seam. Desired behavior
//! snapshots canonical executable identity there, and check reports expose canonical_path and
//! sha256. No pin record is fabricated by the test.

use crate::{Ctx, TaskKind, TaskOutcome};
use sha2::Digest;

/// Stable task identifier.
pub const ID: &str = "task-233";
/// Desired invariant.
pub const NAME: &str = "canonical binary pin records";
/// Executes real Rust runtime seams.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "absolute_binary_runs",
    "relative_alias_runs",
    "canonical_path_recorded",
    "digest_recorded",
];

/// Execute all cases, retaining assertion failures separately from fixture errors.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, CASES.iter().enumerate().map(|(i, c)| (*c, case(i))))
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => {
            let f = Fixture::new()?;
            let mut r = f.runtime(&["/usr/bin/true"], 1000, true)?;
            Ok(ok(&r.check(Some("probe"))))
        }
        1 => {
            let f = Fixture::new()?;
            f.executable(
                "tool",
                "#!/bin/sh
exit 0
",
            )?;
            let mut r = f.runtime(&["./tool"], 1000, true)?;
            Ok(ok(&r.check(Some("probe"))))
        }
        2 => {
            let f = Fixture::new()?;
            let mut r = f.runtime(&["/bin/true"], 1000, true)?;
            let report = r.check(Some("probe"));
            let path = std::fs::canonicalize("/bin/true").map_err(err)?;
            Ok(report["checks"][0]["canonical_path"].as_str() == path.to_str())
        }
        3 => {
            let f = Fixture::new()?;
            let mut r = f.runtime(&["/usr/bin/true"], 1000, true)?;
            let bytes = std::fs::read("/usr/bin/true").map_err(err)?;
            let digest = format!("{:x}", sha2::Sha256::digest(&bytes));
            Ok(r.check(Some("probe"))["checks"][0]["sha256"].as_str() == Some(digest.as_str()))
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

use phlow_runtime::{MsgpackTransport, ReqwestTransport, Runtime};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub(super) type RealRuntime = Runtime<ReqwestTransport, MsgpackTransport>;
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

/// Owns only a freshly created private directory, removed on every return path.
pub(super) struct Fixture {
    pub(super) path: PathBuf,
}

impl Fixture {
    pub(super) fn new() -> Result<Self, String> {
        for _ in 0..32 {
            let number = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("phlow-hardness-{}-{number}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(err(error)),
            }
        }
        Err("private fixture directory collision budget exhausted".to_owned())
    }

    pub(super) fn executable(&self, name: &str, content: &str) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        let path = self.path.join(name);
        std::fs::write(&path, content).map_err(err)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).map_err(err)
    }

    pub(super) fn runtime(
        &self,
        argv: &[&str],
        timeout_ms: u64,
        required: bool,
    ) -> Result<RealRuntime, String> {
        self.runtime_trusted(argv, timeout_ms, required, true)
    }

    pub(super) fn runtime_trusted(
        &self,
        argv: &[&str],
        timeout_ms: u64,
        required: bool,
        trusted: bool,
    ) -> Result<RealRuntime, String> {
        // JSON string arrays are valid TOML basic-string arrays for these fixtures.
        let cmd = serde_json::to_string(argv).map_err(err)?;
        let config_path = self.path.join("operator.toml");
        std::fs::write(
            &config_path,
            format!("[checks.probe]\ncmd = {cmd}\ntimeout = {timeout_ms}\nrequired = {required}\n"),
        )
        .map_err(err)?;
        let config = phlow_config::load_config(&phlow_config::LoadOptions {
            config_path: Some(config_path),
            workspace: Some(self.path.clone()),
            trusted,
            model: None,
        })
        .map_err(err)?;
        Runtime::new(
            config,
            ReqwestTransport::new().map_err(err)?,
            MsgpackTransport::new("/nonexistent/phlow-hardness-editor"),
            None,
        )
        .map_err(err)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.path) {
            eprintln!("gauntlet fixture cleanup failed: {error}");
        }
    }
}

pub(super) fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub(super) fn ok(report: &serde_json::Value) -> bool {
    report["checks"][0]["status"] == "ok"
}

pub(super) fn collect_outcome<'a>(
    id: &str,
    results: impl Iterator<Item = (&'a str, Result<bool, String>)>,
) -> TaskOutcome {
    let mut evidence = Vec::with_capacity(4);
    let mut failures = Vec::with_capacity(4);
    for (case, result) in results.take(4) {
        match result {
            Ok(true) => evidence.push(format!("{case}: pass")),
            Ok(false) => {
                evidence.push(format!("{case}: fail"));
                failures.push(format!("{case}: desired invariant did not hold"));
            }
            Err(error) => {
                evidence.push(format!("{case}: driver error: {error}"));
                failures.push(format!("{case}: fixture/runtime failure: {error}"));
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

pub(super) fn tamper(changed: bool, replace: bool, optional: bool) -> Result<bool, String> {
    let fixture = Fixture::new()?;
    let original = "#!/bin/sh\nexit 0\n";
    fixture.executable("tool", original)?;
    let mut runtime = fixture.runtime(&["./tool"], 1000, !optional)?;
    let content = if changed {
        "#!/bin/sh\nprintf ran > executed\nexit 0\n"
    } else {
        original
    };
    if replace {
        fixture.executable("replacement", content)?;
        std::fs::rename(fixture.path.join("replacement"), fixture.path.join("tool"))
            .map_err(err)?;
    } else if changed {
        fixture.executable("tool", content)?;
    }
    let report = runtime.check(Some("probe"));
    if changed {
        Ok(report["checks"][0]["status"] == "error" && !fixture.path.join("executed").exists())
    } else {
        Ok(ok(&report))
    }
}

pub(super) fn swap_link(changed: bool, identical_bytes: bool) -> Result<bool, String> {
    let fixture = Fixture::new()?;
    let original = "#!/bin/sh\nexit 0\n";
    fixture.executable("first", original)?;
    fixture.executable(
        "second",
        if identical_bytes {
            original
        } else {
            "#!/bin/sh\nprintf ran > executed\nexit 0\n"
        },
    )?;
    let link = fixture.path.join("tool");
    std::os::unix::fs::symlink("first", &link).map_err(err)?;
    let mut runtime = fixture.runtime(&["./tool"], 1000, true)?;
    std::fs::remove_file(&link).map_err(err)?;
    std::os::unix::fs::symlink(if changed { "second" } else { "first" }, &link).map_err(err)?;
    let report = runtime.check(Some("probe"));
    if changed {
        Ok(report["checks"][0]["status"] == "error" && !fixture.path.join("executed").exists())
    } else {
        Ok(ok(&report))
    }
}

/// Child-only PATH scenario; caller supplies the isolated process environment.
pub(super) fn path_child(mode: &str, directory: &std::path::Path) -> Result<bool, String> {
    let fixture = Fixture::new()?;
    let mut runtime = fixture.runtime(&["hardness-tool"], 1000, true)?;
    if mode.starts_with("shadow") {
        let source = directory.join(if mode == "shadow-same" {
            "same"
        } else {
            "different"
        });
        std::fs::copy(source, directory.join("early/hardness-tool")).map_err(err)?;
    }
    let report = runtime.check(Some("probe"));
    if mode.starts_with("shadow") {
        Ok(report["checks"][0]["status"] == "error" && !fixture.path.join("executed").exists())
    } else {
        Ok(ok(&report))
    }
}

pub(super) fn path_probe(shadow: bool, same: bool) -> Result<bool, String> {
    use std::process::{Command, Stdio};
    let fixture = Fixture::new()?;
    std::fs::create_dir(fixture.path.join("early")).map_err(err)?;
    std::fs::create_dir(fixture.path.join("late")).map_err(err)?;
    fixture.executable("late/hardness-tool", "#!/bin/sh\nexit 0\n")?;
    fixture.executable("same", "#!/bin/sh\nexit 0\n")?;
    fixture.executable("different", "#!/bin/sh\nprintf ran > executed\nexit 0\n")?;
    let current = std::env::current_exe().map_err(err)?;
    let binary = current
        .ancestors()
        .skip(1)
        .take(8)
        .map(|directory| directory.join("gauntlet"))
        .find(|candidate| candidate.is_file())
        .ok_or("built gauntlet not found beside test artifacts; cargo build first")?;
    let path = std::env::join_paths([fixture.path.join("early"), fixture.path.join("late")])
        .map_err(err)?;
    let child = Command::new(binary)
        .args([
            "run",
            "task-238",
            "--nvim-bin",
            "/usr/bin/nvim",
            "--diver-lua",
            "/home/phaedrus/.config/diver-fixed/lua",
        ])
        .env("PATH", path)
        .env("PHLOW_PIN_CHILD_DIR", &fixture.path)
        .env(
            "PHLOW_PIN_CHILD_MODE",
            if shadow {
                if same {
                    "shadow-same"
                } else {
                    "shadow-different"
                }
            } else {
                "stable"
            },
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(err)?;
    match crate::wait_for_child(child, Duration::from_secs(5))? {
        crate::WaitOutcome::TimedOut => Err("PATH child exceeded five seconds".to_owned()),
        crate::WaitOutcome::Finished { status, output } => {
            let output = output?;
            let output = String::from_utf8_lossy(&output.stdout);
            if status.code() == Some(0) {
                return Ok(true);
            }
            if status.code() == Some(1) && !output.contains("driver error") {
                return Ok(false);
            }
            Err(format!("PATH child failed: {status}: {output}"))
        }
    }
}

pub(super) fn socket_denied(family: &str) -> Result<bool, String> {
    // Do not credit Phlow for an outer sandbox's denial. A successful host
    // listener proves this environment supports unprivileged TCP sockets.
    let address = if family == "AF_INET6" {
        "[::1]:0"
    } else {
        "127.0.0.1:0"
    };
    let control = std::net::TcpListener::bind(address)
        .map_err(|error| format!("host socket control unavailable: {error}"))?;
    drop(control);
    let fixture = Fixture::new()?;
    // TCP socket creation is unprivileged without an enforcer. SOCK_RAW would
    // test the host's CAP_NET_RAW instead of Phlow's egress boundary.
    let script = format!(
        r#"import socket, errno, sys
try:
 s=socket.socket(socket.{family}, socket.SOCK_STREAM)
except OSError as e:
 sys.exit(0 if e.errno in (errno.EPERM, errno.EACCES) else 73)
else:
 s.close()
 sys.exit(42)
"#
    );
    let mut runtime = fixture.runtime(&["/usr/bin/python3", "-c", &script], 1000, true)?;
    let report = runtime.check(Some("probe"));
    if report["checks"][0]["returncode"] == 73 {
        return Err("socket family unavailable".to_owned());
    }
    Ok(ok(&report))
}

pub(super) fn storm(fires: usize, crash: bool) -> Result<bool, String> {
    use phlow_tuios::hooks::{HOOK_CONCURRENT_MAX, HookContext, HookEvent, HookManager};
    const OUTCOME_WAIT_SECS: u64 = 10;
    let fixture = Fixture::new()?;
    std::fs::write(fixture.path.join("counts"), "0 0").map_err(err)?;
    let script = r#"import fcntl, pathlib, sys, time, os
root = pathlib.Path(sys.argv[1])
def update(delta):
 with open(root / 'lock', 'a') as lock:
  fcntl.flock(lock, fcntl.LOCK_EX)
  live, peak = map(int, (root / 'counts').read_text().split())
  live += delta
  (root / 'counts').write_text(f'{live} {max(live, peak)}')
update(1)
time.sleep(0.2)
update(-1)
os._exit(7 if sys.argv[2] == 'crash' else 0)
"#;
    let manager = HookManager::new();
    manager
        .register(
            HookEvent::AfterAgentState,
            vec![
                "/usr/bin/python3".to_owned(),
                "-c".to_owned(),
                script.to_owned(),
                fixture.path.to_string_lossy().into_owned(),
                if crash { "crash" } else { "normal" }.to_owned(),
            ],
        )
        .map_err(err)?;
    for _ in 0..fires {
        manager.fire(HookEvent::AfterAgentState, &HookContext::default());
    }
    let started = Instant::now();
    let outcomes = loop {
        let outcomes = manager.outcomes();
        if outcomes.len() == fires {
            break outcomes;
        }
        if started.elapsed() > Duration::from_secs(OUTCOME_WAIT_SECS) {
            return Err(format!("only {} of {fires} hook outcomes", outcomes.len()));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let counts = std::fs::read_to_string(fixture.path.join("counts")).map_err(err)?;
    let counts: Vec<usize> = counts
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(err)?;
    if counts.len() != 2 {
        return Err("malformed measured counts".to_owned());
    }
    let dropped = outcomes.iter().filter(|outcome| outcome.dropped).count();
    Ok(counts[0] == 0
        && counts[1] > 0
        && counts[1] <= HOOK_CONCURRENT_MAX
        && (fires <= 4 || dropped > 0)
        && outcomes
            .iter()
            .all(|o| o.dropped || o.exit_code == Some(if crash { 7 } else { 0 })))
}

pub(super) fn reap(mode: &str) -> Result<bool, String> {
    const CHILDREN_MAX: usize = 8;
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..CHILDREN_MAX)
            .map(|_| scope.spawn(|| reap_one(mode)))
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| "reaping worker panicked".to_owned())?
            })
            .collect();
        let results = results.into_iter().collect::<Result<Vec<_>, String>>()?;
        Ok(results.into_iter().all(|passed| passed))
    })
}

fn reap_one(mode: &str) -> Result<bool, String> {
    let fixture = Fixture::new()?;
    let script = match mode {
        "normal" => "import os; print(os.getpid(), flush=True)",
        "nonzero" => "import os; print(os.getpid(), flush=True); raise SystemExit(7)",
        "timeout" => "import os,time; print(os.getpid(), flush=True); time.sleep(1)",
        "crash" => {
            "import os,signal; print(os.getpid(), flush=True); os.kill(os.getpid(),signal.SIGKILL)"
        }
        _ => return Err("unknown reap mode".to_owned()),
    };
    let mut runtime = fixture.runtime(&["/usr/bin/python3", "-c", script], 400, true)?;
    let report = runtime.check(Some("probe"));
    let check = &report["checks"][0];
    let pid: u32 = check["stdout"]
        .as_str()
        .ok_or("missing PID stdout")?
        .trim()
        .parse()
        .map_err(err)?;
    let expected = match mode {
        "normal" => "ok",
        "timeout" => "timeout",
        _ => "failed",
    };
    Ok(check["status"] == expected && !PathBuf::from(format!("/proc/{pid}")).exists())
}

pub(super) fn group(timeout: bool, descendants: usize) -> Result<bool, String> {
    let fixture = Fixture::new()?;
    let script = r#"import os, pathlib, sys, time
for index in range(int(sys.argv[1])):
 pid = os.fork()
 if pid == 0:
  pathlib.Path(f'ready-{index}').write_text(str(os.getpid()))
  time.sleep(0.7)
  pathlib.Path(f'late-{index}').write_text('alive')
  os._exit(0)
for index in range(int(sys.argv[1])):
 os.wait()
"#;
    let count = descendants.to_string();
    let mut runtime = fixture.runtime(
        &["/usr/bin/python3", "-c", script, &count],
        if timeout { 400 } else { 2000 },
        true,
    )?;
    let report = runtime.check(Some("probe"));
    std::thread::sleep(Duration::from_millis(800));
    let expected = if timeout { "timeout" } else { "ok" };
    for index in 0..descendants {
        if !fixture.path.join(format!("ready-{index}")).exists() {
            return Err("descendant did not reach ready barrier".to_owned());
        }
        if fixture.path.join(format!("late-{index}")).exists() == timeout {
            return Ok(false);
        }
    }
    Ok(report["checks"][0]["status"] == expected)
}

#[cfg(test)]
mod tests {
    #[test]
    fn absolute_binary_runs() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn relative_alias_runs() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn canonical_path_recorded() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn digest_recorded() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
