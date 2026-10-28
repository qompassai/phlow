//! Probe a live MCP server with phlow's own client: connect
//! (modern discovery), list its tools, and call one tool.
//!
//! ```sh
//! cargo run -p phlow-mcp --example mcp_probe -- <command> [args...] [--call <tool> [json-args]]
//! ```
//!
//! Examples: `mcp_probe -- phlow serve --call flow_status`;
//! `mcp_probe -- fathom --config fathom.toml serve`.
//! This is the C2.4 gate harness: the client is proven against
//! real migrated servers (phlow itself, fathom), not only
//! against scripted fakes.

use std::time::Duration;

use phlow_mcp::{McpClient, McpServerConfig};
use serde_json::json;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let split = argv.iter().position(|arg| arg == "--").unwrap_or(0);
    let mut server_args: Vec<String> = argv.iter().skip(split + 1).cloned().collect();
    let mut call: Option<(String, serde_json::Value)> = None;
    if let Some(pos) = server_args.iter().position(|arg| arg == "--call") {
        let tool = server_args
            .get(pos + 1)
            .cloned()
            .unwrap_or_else(|| "flow_status".to_string());
        let args = server_args
            .get(pos + 2)
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_else(|| json!({}));
        call = Some((tool, args));
        server_args.truncate(pos);
    }
    let Some((command, args)) = server_args.split_first() else {
        eprintln!("usage: mcp_probe -- <command> [args...] [--call <tool> [json-args]]");
        std::process::exit(2);
    };
    let config = McpServerConfig {
        name: "probe-target".to_string(),
        command: command.clone(),
        args: args.to_vec(),
        env: Vec::new(),
    };
    let mut client = match McpClient::connect(&config, "phlow-mcp-probe") {
        Ok(client) => client,
        Err(error) => {
            eprintln!("connect failed: {error}");
            std::process::exit(1);
        }
    };
    println!("negotiated_version: {}", client.negotiated_version);
    println!("server_name: {:?}", client.server_name);
    match client.list_tools() {
        Ok(tools) => println!("tools: {}", tools.join(", ")),
        Err(error) => {
            eprintln!("tools/list failed: {error}");
            std::process::exit(1);
        }
    }
    if let Some((tool, args)) = call {
        match client.call_tool(&tool, args, Duration::from_secs(600)) {
            Ok(value) => println!("call {tool}: {value}"),
            Err(error) => {
                eprintln!("tools/call {tool} failed: {error}");
                std::process::exit(1);
            }
        }
    }
}
