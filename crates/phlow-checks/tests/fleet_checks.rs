//! Integration tests for the PH-12 fleet-check scripts
//! (`tools/fleet-checks/`): each script is run as the operator's
//! named check would run it — a plain argv, no shell — against
//! fixture trees built under the system temp dir.
//!
//! The split is half validation, half adversarial. Validation: a
//! well-formed seat passes (installed pin resolves, clean files
//! report zero, a recorded freeze verifies, excluded trees do not
//! drift the hash). Adversarial: every way the seat can be wrong is
//! refused with the script's documented exit code (uninstalled or
//! malformed pin, resolution that disagrees with the pin, drift,
//! a foreign freeze record, a tree changing mid-read, malformed
//! targets, a Rust target with no producer output).
//!
//! Environment-gated cases skip (print and pass) when their
//! prerequisite is absent — python3, rustup, or the theodolite
//! bridge — so the suite stays green on machines without the fleet's
//! tooling. On primo all prerequisites exist and every case runs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn scripts_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/fleet-checks")
}

/// A fixture tree under the temp dir, removed on drop.
struct TempTree {
    root: PathBuf,
}

impl TempTree {
    fn new(tag: &str) -> TempTree {
        let serial = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("phlow-fleet-{tag}-{}-{serial}", std::process::id()));
        fs::create_dir_all(&root).expect("create fixture tree");
        TempTree { root }
    }

    fn write(&self, relative: &str, body: &str) {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().expect("fixture path has a parent"))
            .expect("create fixture parent");
        fs::write(&path, body).expect("write fixture file");
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn path_string(&self) -> String {
        self.root.display().to_string()
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn tool_on_path(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn cargo_home_tool(name: &str) -> Option<PathBuf> {
    let home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")))?;
    let candidate = home.join("bin").join(name);
    candidate.is_file().then_some(candidate)
}

/// Run one fleet-check script; `None` means "skip" (no python3).
/// `RUSTUP_TOOLCHAIN` is scrubbed unless a case sets it explicitly,
/// so directory pins — not the test shell's environment — decide
/// resolution.
fn run_script(script: &str, args: &[&str], envs: &[(&str, &str)]) -> Option<Output> {
    if !tool_on_path("python3") {
        eprintln!("SKIP: python3 is unavailable");
        return None;
    }
    let mut command = Command::new("python3");
    command
        .arg(scripts_dir().join(script))
        .args(args)
        .env_remove("RUSTUP_TOOLCHAIN");
    for (key, value) in envs {
        command.env(key, value);
    }
    Some(command.output().expect("run fleet-check script"))
}

fn exit_code(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// An installed channel usable as a fixture pin, when one is known:
/// the stable channel, present on any rustup machine that has it.
fn stable_channel_installed() -> bool {
    let Some(rustup) = cargo_home_tool("rustup")
        .or_else(|| tool_on_path("rustup").then(|| PathBuf::from("rustup")))
    else {
        return false;
    };
    Command::new(rustup)
        .args(["which", "--toolchain", "stable", "rustc"])
        .env("RUSTUP_AUTO_INSTALL", "0")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn channel_installed(channel: &str) -> bool {
    let Some(rustup) = cargo_home_tool("rustup") else {
        return false;
    };
    Command::new(rustup)
        .args(["which", "--toolchain", channel, "rustc"])
        .env("RUSTUP_AUTO_INSTALL", "0")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn bridge_available() -> bool {
    let bin = std::env::var("FLEET_THEODOLITE_BIN")
        .unwrap_or_else(|_| "/home/phaedrus/.local/bin/theodolite".to_owned());
    let config = std::env::var("FLEET_THEODOLITE_CONFIG")
        .unwrap_or_else(|_| "/home/phaedrus/.local/share/theodolite/bridge.toml".to_owned());
    Path::new(&bin).is_file() && Path::new(&config).is_file()
}

// ----------------------------------------------------------------
// Validation: a well-formed seat passes.
// ----------------------------------------------------------------

#[test]
fn toolchain_accepts_installed_stable_pin() {
    if !stable_channel_installed() {
        eprintln!("SKIP: no installed stable toolchain");
        return;
    }
    let tree = TempTree::new("pin-ok");
    tree.write("rust-toolchain.toml", "[toolchain]\nchannel = \"stable\"\n");
    let Some(output) = run_script(
        "toolchain_pin.py",
        &["--workspace", &tree.path_string()],
        &[],
    ) else {
        return;
    };
    assert_eq!(exit_code(&output), 0, "{}", stdout_text(&output));
    assert!(stdout_text(&output).contains("toolchain_pin: PASS"));
}

#[test]
fn toolchain_accepts_legacy_pin_file() {
    if !stable_channel_installed() {
        eprintln!("SKIP: no installed stable toolchain");
        return;
    }
    let tree = TempTree::new("pin-legacy");
    tree.write("rust-toolchain", "stable\n");
    let Some(output) = run_script(
        "toolchain_pin.py",
        &["--workspace", &tree.path_string()],
        &[],
    ) else {
        return;
    };
    assert_eq!(exit_code(&output), 0, "{}", stdout_text(&output));
}

#[test]
fn freeze_records_then_verifies() {
    let tree = TempTree::new("freeze-ok");
    tree.write("a.txt", "alpha\n");
    tree.write("sub/b.txt", "beta\n");
    let freeze = tree.path().join("record.json");
    let freeze_arg = freeze.display().to_string();
    let args = [
        "--workspace",
        &tree.path_string(),
        "--freeze-file",
        &freeze_arg,
    ];
    // The record lives outside the hashed set only by exclusion of
    // the freeze file itself; place it in the tree to prove that too.
    let Some(first) = run_script("tree_freeze.py", &args, &[]) else {
        return;
    };
    assert_eq!(exit_code(&first), 0, "{}", stdout_text(&first));
    assert!(stdout_text(&first).contains("RECORDED"));
    let Some(second) = run_script("tree_freeze.py", &args, &[]) else {
        return;
    };
    assert_eq!(exit_code(&second), 0, "{}", stdout_text(&second));
    assert!(stdout_text(&second).contains("PASS"));
}

#[test]
fn freeze_ignores_excluded_trees() {
    let tree = TempTree::new("freeze-excl");
    tree.write("a.txt", "alpha\n");
    tree.write("target/junk.txt", "one\n");
    let outside = TempTree::new("freeze-excl-record");
    let freeze = outside.path().join("record.json");
    let freeze_arg = freeze.display().to_string();
    let args = [
        "--workspace",
        &tree.path_string(),
        "--freeze-file",
        &freeze_arg,
    ];
    let Some(first) = run_script("tree_freeze.py", &args, &[]) else {
        return;
    };
    assert_eq!(exit_code(&first), 0, "{}", stdout_text(&first));
    tree.write("target/junk.txt", "two\n");
    let Some(second) = run_script("tree_freeze.py", &args, &[]) else {
        return;
    };
    assert_eq!(
        exit_code(&second),
        0,
        "excluded target/ churn must not drift the freeze: {}",
        stdout_text(&second)
    );
}

#[test]
fn theodolite_zero_passes_clean_toml() {
    if !bridge_available() {
        eprintln!("SKIP: theodolite bridge is unavailable");
        return;
    }
    let tree = TempTree::new("theo-toml-ok");
    tree.write("Cargo.toml", "[package]\nname = \"fixture\"\n");
    let Some(output) = run_script(
        "theodolite_zero.py",
        &["--workspace", &tree.path_string(), "Cargo.toml"],
        &[],
    ) else {
        return;
    };
    assert_eq!(exit_code(&output), 0, "{}", stdout_text(&output));
    assert!(stdout_text(&output).contains("control ok for taplo"));
}

#[test]
fn theodolite_zero_passes_clean_python() {
    if !bridge_available() {
        eprintln!("SKIP: theodolite bridge is unavailable");
        return;
    }
    let tree = TempTree::new("theo-py-ok");
    tree.write("ok.py", "def answer():\n    return 42\n");
    let Some(output) = run_script(
        "theodolite_zero.py",
        &["--workspace", &tree.path_string(), "ok.py"],
        &[],
    ) else {
        return;
    };
    assert_eq!(exit_code(&output), 0, "{}", stdout_text(&output));
}

// ----------------------------------------------------------------
// Adversarial: every way the seat can be wrong is refused.
// ----------------------------------------------------------------

#[test]
fn toolchain_rejects_uninstalled_channel() {
    if cargo_home_tool("rustup").is_none() {
        eprintln!("SKIP: rustup is unavailable");
        return;
    }
    let tree = TempTree::new("pin-uninstalled");
    tree.write(
        "rust-toolchain.toml",
        "[toolchain]\nchannel = \"nightly-1999-01-01\"\n",
    );
    let Some(output) = run_script(
        "toolchain_pin.py",
        &["--workspace", &tree.path_string()],
        &[],
    ) else {
        return;
    };
    assert_eq!(exit_code(&output), 1, "{}", stdout_text(&output));
}

#[test]
fn toolchain_rejects_missing_pin() {
    let tree = TempTree::new("pin-missing");
    let Some(output) = run_script(
        "toolchain_pin.py",
        &["--workspace", &tree.path_string()],
        &[],
    ) else {
        return;
    };
    assert_eq!(exit_code(&output), 2, "{}", stdout_text(&output));
}

#[test]
fn toolchain_rejects_malformed_pin() {
    let tree = TempTree::new("pin-malformed");
    tree.write("rust-toolchain.toml", "[toolchain\n");
    let Some(output) = run_script(
        "toolchain_pin.py",
        &["--workspace", &tree.path_string()],
        &[],
    ) else {
        return;
    };
    assert_eq!(exit_code(&output), 2, "{}", stdout_text(&output));
}

#[test]
fn toolchain_rejects_resolution_mismatch() {
    // The pin file names one installed channel while the rustup
    // environment override resolves another: the workspace does not
    // resolve to its pin, and the gate must say so.
    if !channel_installed("nightly-2026-10-30") || !channel_installed("nightly-2026-09-25") {
        eprintln!("SKIP: the two evidence toolchains are not both installed");
        return;
    }
    let tree = TempTree::new("pin-mismatch");
    tree.write(
        "rust-toolchain.toml",
        "[toolchain]\nchannel = \"nightly-2026-10-30\"\n",
    );
    let Some(output) = run_script(
        "toolchain_pin.py",
        &["--workspace", &tree.path_string()],
        &[("RUSTUP_TOOLCHAIN", "nightly-2026-09-25")],
    ) else {
        return;
    };
    assert_eq!(exit_code(&output), 1, "{}", stdout_text(&output));
}

#[test]
fn freeze_detects_drift() {
    let tree = TempTree::new("freeze-drift");
    tree.write("a.txt", "alpha\n");
    let outside = TempTree::new("freeze-drift-record");
    let freeze = outside.path().join("record.json");
    let freeze_arg = freeze.display().to_string();
    let args = [
        "--workspace",
        &tree.path_string(),
        "--freeze-file",
        &freeze_arg,
    ];
    let Some(first) = run_script("tree_freeze.py", &args, &[]) else {
        return;
    };
    assert_eq!(exit_code(&first), 0, "{}", stdout_text(&first));
    tree.write("a.txt", "alpha changed\n");
    let Some(second) = run_script("tree_freeze.py", &args, &[]) else {
        return;
    };
    assert_eq!(exit_code(&second), 1, "{}", stdout_text(&second));
}

#[test]
fn freeze_rejects_foreign_record() {
    let tree_a = TempTree::new("freeze-foreign-a");
    tree_a.write("a.txt", "alpha\n");
    let tree_b = TempTree::new("freeze-foreign-b");
    tree_b.write("a.txt", "alpha\n");
    let outside = TempTree::new("freeze-foreign-record");
    let freeze = outside.path().join("record.json");
    let freeze_arg = freeze.display().to_string();
    let Some(first) = run_script(
        "tree_freeze.py",
        &[
            "--workspace",
            &tree_a.path_string(),
            "--freeze-file",
            &freeze_arg,
        ],
        &[],
    ) else {
        return;
    };
    assert_eq!(exit_code(&first), 0, "{}", stdout_text(&first));
    let Some(second) = run_script(
        "tree_freeze.py",
        &[
            "--workspace",
            &tree_b.path_string(),
            "--freeze-file",
            &freeze_arg,
        ],
        &[],
    ) else {
        return;
    };
    assert_eq!(exit_code(&second), 2, "{}", stdout_text(&second));
}

#[test]
fn freeze_detects_mid_read_mutation() {
    let tree = TempTree::new("freeze-mutating");
    tree.write("a.txt", "alpha\n");
    let outside = TempTree::new("freeze-mutating-record");
    let freeze = outside.path().join("record.json");
    let freeze_arg = freeze.display().to_string();
    let Some(output) = run_script(
        "tree_freeze.py",
        &[
            "--workspace",
            &tree.path_string(),
            "--freeze-file",
            &freeze_arg,
        ],
        &[("FLEET_FREEZE_TEST_MUTATE", "a.txt")],
    ) else {
        return;
    };
    assert_eq!(exit_code(&output), 1, "{}", stdout_text(&output));
    assert!(stdout_text(&output).contains("changed while it was read"));
}

#[test]
fn theodolite_zero_flags_malformed_toml() {
    if !bridge_available() {
        eprintln!("SKIP: theodolite bridge is unavailable");
        return;
    }
    let tree = TempTree::new("theo-toml-bad");
    tree.write("bad.toml", "[workspace]\nmembers = [\n");
    let Some(output) = run_script(
        "theodolite_zero.py",
        &["--workspace", &tree.path_string(), "bad.toml"],
        &[],
    ) else {
        return;
    };
    assert_eq!(exit_code(&output), 1, "{}", stdout_text(&output));
}

#[test]
fn theodolite_zero_flags_python_syntax_error() {
    if !bridge_available() {
        eprintln!("SKIP: theodolite bridge is unavailable");
        return;
    }
    let tree = TempTree::new("theo-py-bad");
    tree.write("bad.py", "def broken(:\n");
    let Some(output) = run_script(
        "theodolite_zero.py",
        &["--workspace", &tree.path_string(), "bad.py"],
        &[],
    ) else {
        return;
    };
    assert_eq!(exit_code(&output), 1, "{}", stdout_text(&output));
}

#[test]
fn theodolite_zero_rust_target_requires_producer() {
    // No .bacon-locations in the fixture: the Rust route must fail
    // closed as unproven, never read a zero that was never produced.
    let tree = TempTree::new("theo-rs-noprod");
    tree.write("lib.rs", "pub fn answer() -> u32 { 42 }\n");
    let Some(output) = run_script(
        "theodolite_zero.py",
        &["--workspace", &tree.path_string(), "lib.rs"],
        &[],
    ) else {
        return;
    };
    assert_eq!(exit_code(&output), 2, "{}", stdout_text(&output));
    assert!(stdout_text(&output).contains("producer precondition unmet"));
}
