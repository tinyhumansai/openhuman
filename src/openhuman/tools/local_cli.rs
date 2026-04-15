//! Local CLI helpers for running tools with workspace config (no `core_server`).

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::security::SecurityPolicy;

use super::traits::Tool;
use super::ScreenshotTool;
use super::{decode_data_url_bytes, extract_data_url, extract_saved_path, write_bytes_to_path};

#[derive(Debug, Default)]
pub struct CliScreenshotArgs {
    pub filename: Option<String>,
    pub region: Option<String>,
    pub output: Option<PathBuf>,
    pub print_data_url: bool,
}

#[derive(Debug, Default)]
pub struct CliScreenshotRefArgs {
    pub output: Option<PathBuf>,
    pub print_data_url: bool,
}

pub fn tools_wrappers_list_json() -> serde_json::Value {
    json!({
        "result": {
            "wrappers": [
                {
                    "name": "screenshot",
                    "description": "Capture a screenshot with screenshot tool wrapper."
                },
                {
                    "name": "screenshot-ref",
                    "description": "Capture data URL from screen intelligence capture_image_ref."
                }
            ]
        },
        "logs": ["tools wrappers listed"]
    })
}

pub async fn run_cli_screenshot(args: CliScreenshotArgs) -> Result<serde_json::Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
    ));
    let tool = ScreenshotTool::new(security);

    let mut payload = serde_json::Map::new();
    if let Some(filename) = args.filename {
        payload.insert("filename".to_string(), json!(filename));
    }
    if let Some(region) = args.region {
        payload.insert("region".to_string(), json!(region));
    }

    let tool_result = tool
        .execute(serde_json::Value::Object(payload))
        .await
        .map_err(|e| format!("screenshot tool failed to execute: {e}"))?;

    let mut logs = vec!["tools.screenshot executed".to_string()];

    if let Some(output_path) = args.output.as_ref() {
        if let Some(saved_path) = extract_saved_path(&tool_result.output()) {
            std::fs::copy(&saved_path, output_path).map_err(|e| {
                format!(
                    "failed to copy screenshot from {} to {}: {e}",
                    saved_path.display(),
                    output_path.display()
                )
            })?;
            logs.push(format!("copied screenshot to {}", output_path.display()));
        } else if let Some(data_url) = extract_data_url(&tool_result.output()) {
            let bytes = decode_data_url_bytes(&data_url)?;
            write_bytes_to_path(output_path, &bytes)?;
            logs.push(format!(
                "decoded data URL and wrote {} bytes to {}",
                bytes.len(),
                output_path.display()
            ));
        } else {
            return Err(
                "screenshot tool response did not contain a saved path or image data URL"
                    .to_string(),
            );
        }
    }

    let data_url = extract_data_url(&tool_result.output());
    Ok(json!({
        "result": {
            "success": !tool_result.is_error,
            "error": if tool_result.is_error { Some(tool_result.output()) } else { None::<String> },
            "output_path": args.output.as_ref().map(|p| p.display().to_string()),
            "tool_output": tool_result.output(),
            "data_url": if args.print_data_url { data_url } else { None::<String> },
        },
        "logs": logs
    }))
}

pub async fn run_cli_screenshot_ref(
    args: CliScreenshotRefArgs,
) -> Result<serde_json::Value, String> {
    let crate::rpc::RpcOutcome {
        value: payload,
        mut logs,
    } = crate::openhuman::screen_intelligence::rpc::accessibility_capture_image_ref().await?;
    logs.push("tools.screenshot-ref executed".to_string());

    if let Some(output_path) = args.output.as_ref() {
        if let Some(data_url) = payload.image_ref.as_deref() {
            let bytes = decode_data_url_bytes(data_url)?;
            write_bytes_to_path(output_path, &bytes)?;
            logs.push(format!(
                "decoded image_ref and wrote {} bytes to {}",
                bytes.len(),
                output_path.display()
            ));
        } else {
            return Err(
                "screen intelligence capture_image_ref did not return image_ref".to_string(),
            );
        }
    }

    Ok(json!({
        "result": {
            "ok": payload.ok,
            "mime_type": payload.mime_type,
            "bytes_estimate": payload.bytes_estimate,
            "message": payload.message,
            "output_path": args.output.as_ref().map(|p| p.display().to_string()),
            "image_ref": if args.print_data_url { payload.image_ref } else { None::<String> },
        },
        "logs": logs
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── CliScreenshotArgs ─────────────────────────────────────────────────────

    #[test]
    fn cli_screenshot_args_default_fields() {
        let args = CliScreenshotArgs::default();
        assert!(args.filename.is_none());
        assert!(args.region.is_none());
        assert!(args.output.is_none());
        assert!(!args.print_data_url);
    }

    #[test]
    fn cli_screenshot_args_debug_does_not_panic() {
        let args = CliScreenshotArgs {
            filename: Some("shot.png".into()),
            region: Some("selection".into()),
            output: Some(PathBuf::from("/tmp/out.png")),
            print_data_url: true,
        };
        let dbg = format!("{args:?}");
        assert!(dbg.contains("shot.png"));
        assert!(dbg.contains("selection"));
        assert!(dbg.contains("print_data_url: true"));
    }

    // ── CliScreenshotRefArgs ──────────────────────────────────────────────────

    #[test]
    fn cli_screenshot_ref_args_default_fields() {
        let args = CliScreenshotRefArgs::default();
        assert!(args.output.is_none());
        assert!(!args.print_data_url);
    }

    #[test]
    fn cli_screenshot_ref_args_debug_does_not_panic() {
        let args = CliScreenshotRefArgs {
            output: Some(PathBuf::from("/tmp/ref.png")),
            print_data_url: false,
        };
        let dbg = format!("{args:?}");
        assert!(dbg.contains("print_data_url: false"));
    }

    // ── tools_wrappers_list_json ──────────────────────────────────────────────

    #[test]
    fn tools_wrappers_list_json_shape() {
        let v = tools_wrappers_list_json();

        // Top-level keys
        assert!(v["result"].is_object(), "should have a 'result' key");
        assert!(v["logs"].is_array(), "should have a 'logs' array");

        // Wrappers array
        let wrappers = v["result"]["wrappers"]
            .as_array()
            .expect("wrappers is array");
        assert_eq!(wrappers.len(), 2, "should list exactly 2 wrappers");

        // First wrapper
        assert_eq!(wrappers[0]["name"].as_str(), Some("screenshot"));
        assert!(
            wrappers[0]["description"]
                .as_str()
                .unwrap()
                .contains("screenshot"),
            "screenshot description should mention screenshot"
        );

        // Second wrapper
        assert_eq!(wrappers[1]["name"].as_str(), Some("screenshot-ref"));
        assert!(
            wrappers[1]["description"]
                .as_str()
                .unwrap()
                .contains("capture_image_ref"),
            "screenshot-ref description should mention capture_image_ref"
        );
    }

    #[test]
    fn tools_wrappers_list_json_logs_populated() {
        let v = tools_wrappers_list_json();
        let logs = v["logs"].as_array().unwrap();
        assert!(!logs.is_empty(), "logs should not be empty");
        let first = logs[0].as_str().unwrap();
        assert!(
            first.contains("listed"),
            "log entry should mention 'listed'"
        );
    }

    #[test]
    fn tools_wrappers_list_json_is_deterministic() {
        let v1 = tools_wrappers_list_json();
        let v2 = tools_wrappers_list_json();
        assert_eq!(v1, v2);
    }
}
