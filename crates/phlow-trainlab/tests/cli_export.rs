//! CLI-level tests for `run --export-groups`: the flag produces an
//! export tied to the receipt, its absence leaves the run exactly
//! as before, and a pre-existing export path refuses the run before
//! anything is sampled or written.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn python3_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "phlow-trainlab-cli-test-{}-{tag}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test dir");
    dir
}

fn run_cli(dir: &Path, export: Option<&Path>) -> std::process::Output {
    let receipt = dir.join("receipt.json");
    let mut command = Command::new(env!("CARGO_BIN_EXE_phlow-trainlab"));
    command
        .args(["run", "--split", "dev", "--per-family", "1"])
        .args([
            "--groups",
            "2",
            "--group-size",
            "2",
            "--families",
            "increment",
        ])
        .args(["--sampler", "stub", "--execute-rewards"])
        .args(["--receipt", receipt.to_str().expect("receipt path")]);
    if let Some(path) = export {
        command.args(["--export-groups", path.to_str().expect("export path")]);
    }
    command.output().expect("run CLI")
}

#[test]
fn cli_run_with_export_groups_writes_tied_export() {
    if !python3_available() {
        eprintln!("SKIP: python3 not available for CLI export test");
        return;
    }
    let dir = test_dir("with-export");
    let export_path = dir.join("groups.json");
    let output = run_cli(&dir, Some(export_path.as_path()));
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("receipt.json")).expect("receipt"))
            .expect("receipt json");
    let exported: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&export_path).expect("export"))
            .expect("export json");
    assert_eq!(exported["schema"], "phlow.trainlab.groups/v1");
    assert_eq!(exported["run_id"], receipt["run_id"]);
    assert_eq!(exported["config_sha256"], receipt["config_sha256"]);
    assert_eq!(exported["split"], receipt["split"]);
    assert_eq!(exported["sampler_id"], receipt["sampler_id"]);
    let groups = exported["groups"].as_array().expect("export groups");
    let receipt_groups = receipt["groups"].as_array().expect("receipt groups");
    assert_eq!(groups.len(), receipt_groups.len());
    for (group, recorded) in groups.iter().zip(receipt_groups.iter()) {
        assert_eq!(group["rewards"], recorded["rewards"]);
        assert_eq!(group["advantages"], recorded["advantages"]);
        assert_eq!(group["task_id"], recorded["task_id"]);
        assert!(group["prompt"].as_str().is_some_and(|p| !p.is_empty()));
        assert_eq!(
            group["completions"].as_array().expect("completions").len(),
            group["rewards"].as_array().expect("rewards").len()
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cli_run_without_flag_writes_receipt_only() {
    if !python3_available() {
        eprintln!("SKIP: python3 not available for CLI export test");
        return;
    }
    let dir = test_dir("no-export");
    let output = run_cli(&dir, None);
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(dir.join("receipt.json").exists());
    let stdout: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("stdout json");
    assert_eq!(stdout["export_groups"], serde_json::Value::Null);
    // Exactly one file in the run directory: the receipt.
    let entries: Vec<String> = fs::read_dir(&dir)
        .expect("read dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec!["receipt.json".to_string()]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cli_run_refuses_existing_export_before_writing_anything() {
    // Adversarial: a pre-existing export path is another run's
    // evidence. The CLI must refuse up front — the sentinel file
    // stays byte-identical and no receipt is written.
    if !python3_available() {
        eprintln!("SKIP: python3 not available for CLI export test");
        return;
    }
    let dir = test_dir("refuses");
    let export_path = dir.join("groups.json");
    fs::write(&export_path, "previous run's export").expect("sentinel");
    let output = run_cli(&dir, Some(export_path.as_path()));
    assert!(!output.status.success(), "CLI must refuse");
    assert_eq!(
        fs::read_to_string(&export_path).expect("sentinel read"),
        "previous run's export"
    );
    assert!(
        !dir.join("receipt.json").exists(),
        "a refused run must not write a receipt"
    );
    let _ = fs::remove_dir_all(&dir);
}
