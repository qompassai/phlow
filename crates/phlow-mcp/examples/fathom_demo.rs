//! Demo: phlow drives fathom's reverse-engineering tools
//! through [`phlow_mcp::client`] (Phase E, 2026-10-18).
//!
//! ```sh
//! FATHOM_BIN=/path/to/fathom \
//!   cargo run -p phlow-mcp --example fathom_demo -- <target-binary>
//! ```
//!
//! The demo loads the checked-in profile
//! (`crates/phlow-mcp/tests/fixtures/fathom-demo-profile.toml`),
//! prepares a scratch root containing a copy of the target plus
//! a generated fathom config admitting exactly that root
//! (fathom is fail-closed; the demo widens nothing), spawns
//! fathom as the profile registers it, and calls three distinct
//! tools. The transcript it prints is the integration proof:
//! negotiated protocol version, tool inventory, and one evidence
//! summary per call.

use std::path::{Path, PathBuf};
use std::time::Duration;

use phlow_mcp::client::{McpClient, load_profile};
use serde_json::{Value, json};

fn main() {
    if let Err(error) = run() {
        eprintln!("fathom_demo: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let target_arg = std::env::args()
        .nth(1)
        .ok_or_else(|| "usage: fathom_demo <target-binary>".to_string())?;
    let fathom_bin = std::env::var("FATHOM_BIN")
        .map_err(|_| "FATHOM_BIN must name the fathom binary".to_string())?;
    let source = PathBuf::from(&target_arg);
    if !source.is_file() {
        return Err(format!("target {target_arg} is not a file").into());
    }

    let root = std::env::temp_dir().join(format!("phlow-fathom-demo-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    let target = root.join("target-bin");
    std::fs::copy(&source, &target)?;
    let fathom_config = root.join("fathom.toml");
    std::fs::write(
        &fathom_config,
        format!(
            "target_roots = [\"{}\"]\nstate_dir = \"{}\"\n",
            root.display(),
            root.join("state").display()
        ),
    )?;

    let profile_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fathom-demo-profile.toml");
    let servers = load_profile(&profile_path)?;
    let mut server = servers
        .into_iter()
        .find(|s| s.name == "fathom")
        .ok_or("profile registers no 'fathom' server")?;
    server.command = fathom_bin;
    server.args = server
        .args
        .iter()
        .map(|arg| arg.replace("{fathom_config}", &fathom_config.to_string_lossy()))
        .collect();

    println!("== phlow -> fathom demo ==");
    println!("profile: {}", profile_path.display());
    println!("target:  {} (copied into a scratch root)", source.display());
    let mut client = McpClient::connect(&server, "phlow-fathom-demo")?;
    println!(
        "negotiated protocol: {} (server: {})",
        client.negotiated_version,
        client.server_name.as_deref().unwrap_or("unnamed")
    );
    let tools = client.list_tools()?;
    println!("fathom offers {} tools", tools.len());

    let target_text = target.to_string_lossy().into_owned();
    for (tool, timeout) in [
        ("binary_overview", 600_u64),
        ("inspect_artifact", 60),
        ("list_strings", 600),
    ] {
        let envelope = client.call_tool(
            tool,
            json!({"target": target_text}),
            Duration::from_secs(timeout),
        )?;
        println!("--- {tool} ---");
        print_summary(&envelope);
    }
    println!("== demo complete: 3 fathom tools answered through phlow ==");
    std::fs::remove_dir_all(&root).ok();
    Ok(())
}

fn print_summary(envelope: &Value) {
    println!("status:   {}", envelope["status"].as_str().unwrap_or("?"));
    println!(
        "subject:  {} bytes, sha256 {}…",
        envelope["subject"]["size_bytes"].as_u64().unwrap_or(0),
        envelope["subject"]["sha256"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(16)
            .collect::<String>()
    );
    if let Some(findings) = envelope["findings"].as_array() {
        for finding in findings {
            println!(
                "finding:  {} ({})",
                finding["kind"].as_str().unwrap_or("?"),
                finding["confidence"].as_str().unwrap_or("?")
            );
        }
    }
    if let Some(unknowns) = envelope["unknowns"].as_array() {
        for unknown in unknowns {
            println!(
                "unknown:  {} — {}",
                unknown["aspect"].as_str().unwrap_or("?"),
                unknown["reason"].as_str().unwrap_or("?")
            );
        }
    }
}
