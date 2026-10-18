//! Fathom <-> phlow integration test (Phase E, 2026-10-18).
//!
//! Drives fathom's stdio MCP server through phlow-mcp's client
//! exactly as the demo profile registers it, against a real ELF
//! target (this test binary itself, copied into a scratch root so
//! fathom's fail-closed target policy admits it).
//!
//! Gating: when `FATHOM_BIN` is unset the test reports the skip
//! and passes — a machine without a fathom build has nothing to
//! integrate with. When `FATHOM_BIN` is set, every assertion is
//! hard: a broken fathom or a broken client fails the suite.

use std::path::{Path, PathBuf};
use std::time::Duration;

use phlow_mcp::client::{McpClient, load_profile};
use serde_json::json;

fn scratch_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("phlow-fathom-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch root");
    dir
}

/// Copy the test binary into the scratch root and write the
/// fathom config that admits exactly that root.
fn prepare_target(root: &Path) -> (PathBuf, PathBuf) {
    let target = root.join("target-bin");
    std::fs::copy(std::env::current_exe().expect("current exe"), &target)
        .expect("copy test binary as target");
    let config = root.join("fathom.toml");
    std::fs::write(
        &config,
        format!(
            "target_roots = [\"{}\"]\nstate_dir = \"{}\"\n",
            root.display(),
            root.join("state").display()
        ),
    )
    .expect("write fathom config");
    (target, config)
}

#[test]
fn fathom_tools_answer_through_phlow_client() {
    let Ok(fathom_bin) = std::env::var("FATHOM_BIN") else {
        eprintln!("FATHOM_BIN unset: fathom integration test skipped");
        return;
    };
    let root = scratch_root("integration");
    let (target, fathom_config) = prepare_target(&root);

    // Registration goes through the checked-in demo profile: the
    // same TOML a phlow operator profile would carry.
    let profile_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fathom-demo-profile.toml");
    let servers = load_profile(&profile_path).expect("demo profile parses");
    let mut server = servers
        .into_iter()
        .find(|s| s.name == "fathom")
        .expect("profile registers fathom");
    server.command = fathom_bin;
    server.args = server
        .args
        .iter()
        .map(|arg| arg.replace("{fathom_config}", &fathom_config.to_string_lossy()))
        .collect();

    let mut client =
        McpClient::connect(&server, "phlow-integration-test").expect("fathom handshake succeeds");
    // Fathom speaks the classic initialize handshake at
    // 2025-06-18, inside phlow's supported set: negotiation must
    // land there, not on phlow's newer offer.
    assert_eq!(client.negotiated_version, "2025-06-18");
    assert_eq!(client.server_name.as_deref(), Some("fathom"));

    let tools = client.list_tools().expect("tools/list answers");
    for expected in ["binary_overview", "inspect_artifact", "list_strings"] {
        assert!(
            tools.iter().any(|t| t == expected),
            "fathom offers {expected}"
        );
    }

    let target_arg = target.to_string_lossy().into_owned();
    let target_size = std::fs::metadata(&target).expect("target metadata").len();

    let overview = client
        .call_tool(
            "binary_overview",
            json!({"target": target_arg}),
            Duration::from_secs(600),
        )
        .expect("binary_overview answers");
    assert_eq!(overview["tool"], "binary_overview");
    assert_eq!(overview["provider"], "ghidra");
    assert_eq!(overview["subject"]["size_bytes"], target_size);
    assert!(
        overview["findings"]
            .as_array()
            .is_some_and(|findings| !findings.is_empty()),
        "overview carries findings"
    );

    let artifact = client
        .call_tool(
            "inspect_artifact",
            json!({"target": target_arg}),
            Duration::from_secs(60),
        )
        .expect("inspect_artifact answers");
    assert_eq!(artifact["tool"], "inspect_artifact");
    assert_eq!(artifact["subject"]["size_bytes"], target_size);
    // Two independent tools agree on the subject identity.
    assert_eq!(artifact["subject"]["sha256"], overview["subject"]["sha256"]);

    let strings = client
        .call_tool(
            "list_strings",
            json!({"target": target_arg}),
            Duration::from_secs(600),
        )
        .expect("list_strings answers");
    assert_eq!(strings["tool"], "list_strings");
    let kinds: Vec<&str> = strings["findings"]
        .as_array()
        .expect("strings findings")
        .iter()
        .filter_map(|f| f["kind"].as_str())
        .collect();
    assert!(
        kinds.contains(&"strings"),
        "defined strings finding present"
    );
    assert!(
        kinds.contains(&"data_strings"),
        "byte-run data_strings finding present"
    );

    std::fs::remove_dir_all(&root).ok();
}
