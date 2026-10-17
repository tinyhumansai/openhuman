//! Wire-level integration coverage for OpenHuman's stdio MCP server.
//!
//! The server is a subprocess and the test acts as an external MCP client over
//! newline-delimited JSON-RPC. This keeps the host test independent of MCP
//! implementation crates while exercising the real CLI transport.

#![cfg(feature = "mcp")]

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use tinymcp_bus::LATEST_PROTOCOL_VERSION;

fn send_message(writer: &mut impl Write, message: &Value) {
    serde_json::to_writer(&mut *writer, message).expect("serialize MCP message");
    writer.write_all(b"\n").expect("write MCP message");
    writer.flush().expect("flush MCP message");
}

fn wait_for_response(receiver: &Receiver<Result<Value, String>>, request_id: u64) -> Value {
    loop {
        let message = receiver
            .recv_timeout(Duration::from_secs(60))
            .expect("stdio MCP response arrives before the deadline")
            .expect("stdio MCP server writes valid JSON-RPC to stdout");
        if message.get("id") == Some(&json!(request_id)) {
            return message;
        }
    }
}

#[test]
fn stdio_server_initializes_and_lists_core_tools() {
    let workspace = tempfile::tempdir().expect("temporary MCP workspace");
    let workspace_path =
        std::fs::canonicalize(workspace.path()).expect("canonicalize temporary MCP workspace");
    let mut child = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .arg("mcp")
        .current_dir(&workspace_path)
        .env("OPENHUMAN_WORKSPACE", &workspace_path)
        .env("OPENHUMAN_KEYRING_BACKEND", "encrypted_file")
        .env("OPENHUMAN_KEYRING_MASTER_KEY", "42".repeat(32))
        .env_remove("OPENHUMAN_KEYRING_MASTER_KEY_FILE")
        .env_remove("OPENHUMAN_APP_ENV")
        .env_remove("OPENHUMAN_MODE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn OpenHuman's stdio MCP server");
    let mut stdin = child.stdin.take().expect("child stdin");
    let stdout = child.stdout.take().expect("child stdout");
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let message = line.map_err(|error| error.to_string()).and_then(|line| {
                serde_json::from_str(&line)
                    .map_err(|error| format!("{error}: stdout line {line:?}"))
            });
            if sender.send(message).is_err() {
                break;
            }
        }
    });

    send_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": LATEST_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "openhuman-stdio-integration",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            },
        }),
    );
    let initialized = wait_for_response(&receiver, 1);
    assert!(initialized.get("error").is_none(), "{initialized}");
    assert_eq!(
        initialized["result"]["protocolVersion"],
        LATEST_PROTOCOL_VERSION
    );

    send_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {},
        }),
    );
    send_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {},
        }),
    );
    let listed = wait_for_response(&receiver, 2);
    assert!(listed.get("error").is_none(), "{listed}");
    assert!(
        listed["result"]["tools"]
            .as_array()
            .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == "memory.recall")),
        "the server exposes the read-only memory tool: {listed}"
    );

    send_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "notifications/exit",
            "params": {},
        }),
    );
    drop(stdin);
    let status = child.wait().expect("wait for stdio server shutdown");
    reader.join().expect("join stdout reader");
    assert!(status.success(), "stdio MCP server exited with {status}");
}
