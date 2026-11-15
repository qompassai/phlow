//! PH-9 — self-improvement safety case, `phlow-checks` suite.
//! The checks half of the case: the loop may never change its own
//! checks, and the removed implicit-execution tools stay denied.
//! Full case: `docs/SELF_IMPROVEMENT_SAFETY.md`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use phlow_checks::{CheckRunner, CheckStatus, DisabledError, DisabledTool};
use phlow_config::{LoadOptions, load_config};
use phlow_workspace::Workspace;

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "phlow-checks-safety-test-{prefix}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).expect("test setup: create temp dir");
    dir
}

#[test]
fn shell_exec_denial_is_exact_and_verifies_nothing() {
    let denial = DisabledError::shell_exec();
    assert_eq!(denial.tool(), DisabledTool::ShellExec);
    assert_eq!(denial.status(), "error");
    assert_eq!(
        denial.message(),
        "Arbitrary commands and cwd overrides are disabled. \
         Configure exact named check argv and use flow_check."
    );
    assert!(!denial.clean());
    assert!(!denial.verified());
}

#[test]
fn lsp_check_denial_is_exact_and_verifies_nothing() {
    let denial = DisabledError::lsp_check("rust");
    assert_eq!(denial.tool(), DisabledTool::LspCheck);
    assert_eq!(denial.status(), "unavailable");
    assert_eq!(
        denial.message(),
        "No implicit language command execution. \
         Configure a named lint/typecheck in operator TOML or attach \
         Rose's native editor tools."
    );
    assert!(!denial.clean());
    assert!(!denial.verified());
}

#[test]
fn the_loop_cannot_invent_a_check() {
    // Denial 2: checks exist only because the operator configured
    // them. With an empty operator map, any name the loop asks for —
    // including one that would be convenient to have — is
    // unavailable, and nothing runs.
    let dir = temp_dir("noinvention");
    let config_path = dir.join("flow-test.toml");
    std::fs::write(&config_path, "").expect("test setup: write config");
    let config = load_config(&LoadOptions {
        config_path: Some(config_path),
        workspace: Some(dir.clone()),
        trusted: true,
        model: None,
    })
    .expect("test setup: load config");
    assert!(config.checks().is_empty());
    let workspace = Workspace::open(&dir, true, &[]).expect("test setup: open workspace");
    let runner = CheckRunner::new(&workspace, BTreeMap::new());
    for invented in ["self_approved_lint", "cargo-test", "rm-rf"] {
        let report = runner.run(invented);
        assert_eq!(report.status, CheckStatus::Unavailable, "{invented}");
        assert_eq!(report.error.as_deref(), Some("No such configured check"));
        assert!(report.pid.is_none(), "no process may run for {invented}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
