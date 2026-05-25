//! Tiny MCP stdio server used by `tests/mcp_registry_e2e.rs`.
//!
//! Speaks just enough of MCP 2024-11-05 to satisfy `initialize`,
//! `tools/list`, and `tools/call` for one toy tool (`echo`). Reads
//! newline-delimited JSON-RPC from stdin, writes responses to stdout,
//! exits when stdin closes.
//!
//! Intentionally dependency-free beyond serde_json so the binary builds
//! fast and is reliable in CI.

use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2025-11-25";

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();

    let _ = writeln!(stderr, "[test_mcp_stub] ready");

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(err) => {
                let _ = writeln!(stderr, "[test_mcp_stub] stdin read error: {err}");
                break;
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(err) => {
                let _ = writeln!(stderr, "[test_mcp_stub] invalid JSON: {err}");
                continue;
            }
        };

        let id = req.get("id").cloned();
        let method = req
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let params = req.get("params").cloned().unwrap_or(Value::Null);

        // Notifications (no id) are accepted silently — MCP sends
        // `notifications/initialized` after the initialize handshake.
        if id.is_none() {
            let _ = writeln!(stderr, "[test_mcp_stub] notification: {method}");
            continue;
        }

        let response = match method.as_str() {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "test_mcp_stub", "version": "0.0.1" }
                }
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": [
                        {
                            "name": "echo",
                            "description": "Returns the `message` argument verbatim.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "message": { "type": "string" }
                                },
                                "required": ["message"]
                            }
                        }
                    ]
                }
            }),
            "tools/call" => {
                let tool = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                if tool == "echo" {
                    let msg = args.get("message").and_then(Value::as_str).unwrap_or("");
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [
                                { "type": "text", "text": msg }
                            ],
                            "isError": false
                        }
                    })
                } else {
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32601,
                            "message": format!("unknown tool `{tool}`")
                        }
                    })
                }
            }
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("method `{method}` not implemented in stub")
                }
            }),
        };

        let line = serde_json::to_string(&response).unwrap();
        let _ = writeln!(stdout, "{line}");
        let _ = stdout.flush();
    }

    let _ = writeln!(stderr, "[test_mcp_stub] stdin closed, exiting");
}
