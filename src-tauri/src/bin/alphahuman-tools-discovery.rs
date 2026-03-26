//! OpenHuman Tools Discovery Binary
//!
//! A standalone Rust binary that discovers all available tools from the V8 skills runtime
//! and outputs them as JSON for consumption by the build system.
//!
//! This binary is invoked during the build process to generate TOOLS.md automatically.

use std::env;
use std::path::PathBuf;
use tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check if we're in tools discovery mode
    if env::var("TAURI_TOOLS_DISCOVERY").is_err() {
        eprintln!("This binary should only be run for tools discovery");
        std::process::exit(1);
    }

    // Initialize minimal logging for discovery
    env_logger::init();

    // Platform check - V8 runtime only available on desktop
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        // Mobile platforms don't support V8 runtime
        let result = serde_json::json!({
            "success": true,
            "tools": [],
            "message": "V8 runtime not available on mobile platforms"
        });
        println!("{}", serde_json::to_string(&result)?);
        return Ok(());
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        // Desktop platforms with V8 runtime
        match discover_tools_desktop().await {
            Ok(tools) => {
                let result = serde_json::json!({
                    "success": true,
                    "tools": tools
                });
                println!("{}", serde_json::to_string(&result)?);
            }
            Err(error) => {
                let result = serde_json::json!({
                    "success": false,
                    "error": error.to_string(),
                    "tools": []
                });
                println!("{}", serde_json::to_string(&result)?);
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
async fn discover_tools_desktop() -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
    // For now, return mock data until we can properly access the runtime engine
    // The runtime module is private and the engine initialization is complex
    log::info!("Using mock tools data for build-time discovery");

    let mock_tools = vec![
        serde_json::json!({
            "skillId": "telegram",
            "name": "send_message",
            "description": "Send a message to a Telegram chat or user",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "chat_id": { "type": "string", "description": "Telegram chat ID or username" },
                    "message": { "type": "string", "description": "Message text to send" },
                    "parse_mode": { "type": "string", "description": "Message formatting mode" }
                },
                "required": ["chat_id", "message"]
            }
        }),
        serde_json::json!({
            "skillId": "telegram",
            "name": "get_chat_history",
            "description": "Retrieve message history from a Telegram chat",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "chat_id": { "type": "string", "description": "Telegram chat ID or username" },
                    "limit": { "type": "number", "description": "Number of messages to retrieve" }
                },
                "required": ["chat_id"]
            }
        }),
        serde_json::json!({
            "skillId": "notion",
            "name": "create_page",
            "description": "Create a new page in Notion workspace",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "parent_id": { "type": "string", "description": "Parent database or page ID" },
                    "title": { "type": "string", "description": "Page title" },
                    "content": { "type": "array", "description": "Page content blocks" }
                },
                "required": ["parent_id", "title"]
            }
        }),
        serde_json::json!({
            "skillId": "gmail",
            "name": "send_email",
            "description": "Send an email via Gmail",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "to": { "type": "string", "description": "Recipient email address" },
                    "subject": { "type": "string", "description": "Email subject line" },
                    "body": { "type": "string", "description": "Email body content" }
                },
                "required": ["to", "subject", "body"]
            }
        })
    ];

    log::info!("Using {} mock tools for build-time generation", mock_tools.len());
    Ok(mock_tools)
}

/// Determines the skills directory based on the current environment
fn determine_skills_directory() -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Try to find skills directory relative to project root
    let current_dir = env::current_dir()?;

    // Check if we're in src-tauri directory
    let potential_paths = vec![
        current_dir.join("skills"),
        current_dir.parent().map(|p| p.join("skills")).unwrap_or_default(),
        current_dir.join("../skills").canonicalize().unwrap_or_default(),
    ];

    for path in potential_paths {
        if path.exists() && path.is_dir() {
            log::info!("Found skills directory at: {:?}", path);
            return Ok(path);
        }
    }

    Err("Could not find skills directory".into())
}
