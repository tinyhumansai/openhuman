use super::traits::{Tool, ToolResult};
use crate::openhuman::security::SecurityPolicy;
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

const PUSHOVER_API_URL: &str = "https://api.pushover.net/1/messages.json";
const PUSHOVER_REQUEST_TIMEOUT_SECS: u64 = 15;

pub struct PushoverTool {
    security: Arc<SecurityPolicy>,
    workspace_dir: PathBuf,
}

impl PushoverTool {
    pub fn new(security: Arc<SecurityPolicy>, workspace_dir: PathBuf) -> Self {
        Self {
            security,
            workspace_dir,
        }
    }

    fn parse_env_value(raw: &str) -> String {
        let raw = raw.trim();

        let unquoted = if raw.len() >= 2
            && ((raw.starts_with('"') && raw.ends_with('"'))
                || (raw.starts_with('\'') && raw.ends_with('\'')))
        {
            &raw[1..raw.len() - 1]
        } else {
            raw
        };

        // Keep support for inline comments in unquoted values:
        // KEY=value # comment
        unquoted.split_once(" #").map_or_else(
            || unquoted.trim().to_string(),
            |(value, _)| value.trim().to_string(),
        )
    }

    async fn get_credentials(&self) -> anyhow::Result<(String, String)> {
        let env_path = self.workspace_dir.join(".env");
        let content = tokio::fs::read_to_string(&env_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", env_path.display(), e))?;

        let mut token = None;
        let mut user_key = None;

        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let line = line.strip_prefix("export ").map(str::trim).unwrap_or(line);
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = Self::parse_env_value(value);

                if key.eq_ignore_ascii_case("PUSHOVER_TOKEN") {
                    token = Some(value);
                } else if key.eq_ignore_ascii_case("PUSHOVER_USER_KEY") {
                    user_key = Some(value);
                }
            }
        }

        let token = token.ok_or_else(|| anyhow::anyhow!("PUSHOVER_TOKEN not found in .env"))?;
        let user_key =
            user_key.ok_or_else(|| anyhow::anyhow!("PUSHOVER_USER_KEY not found in .env"))?;

        Ok((token, user_key))
    }
}

#[async_trait]
impl Tool for PushoverTool {
    fn name(&self) -> &str {
        "pushover"
    }

    fn description(&self) -> &str {
        "Send a Pushover notification to your device. Requires PUSHOVER_TOKEN and PUSHOVER_USER_KEY in .env file."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The notification message to send"
                },
                "title": {
                    "type": "string",
                    "description": "Optional notification title"
                },
                "priority": {
                    "type": "integer",
                    "description": "Message priority: -2 (lowest/silent), -1 (low/no sound), 0 (normal), 1 (high), 2 (emergency/repeating)"
                },
                "sound": {
                    "type": "string",
                    "description": "Notification sound override (e.g., 'pushover', 'bike', 'bugle', 'cashregister', etc.)"
                }
            },
            "required": ["message"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        if !self.security.can_act() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: autonomy is read-only".into()),
            });
        }

        if !self.security.record_action() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: rate limit exceeded".into()),
            });
        }

        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'message' parameter"))?
            .to_string();

        let title = args.get("title").and_then(|v| v.as_str()).map(String::from);

        let priority = match args.get("priority").and_then(|v| v.as_i64()) {
            Some(value) if (-2..=2).contains(&value) => Some(value),
            Some(value) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!(
                        "Invalid 'priority': {value}. Expected integer in range -2..=2"
                    )),
                })
            }
            None => None,
        };

        let sound = args.get("sound").and_then(|v| v.as_str()).map(String::from);

        let (token, user_key) = self.get_credentials().await?;

        let mut form = reqwest::multipart::Form::new()
            .text("token", token)
            .text("user", user_key)
            .text("message", message);

        if let Some(title) = title {
            form = form.text("title", title);
        }

        if let Some(priority) = priority {
            form = form.text("priority", priority.to_string());
        }

        if let Some(sound) = sound {
            form = form.text("sound", sound);
        }

        let client = crate::openhuman::config::build_runtime_proxy_client_with_timeouts(
            "tool.pushover",
            PUSHOVER_REQUEST_TIMEOUT_SECS,
            10,
        );
        let response = client.post(PUSHOVER_API_URL).multipart(form).send().await?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Ok(ToolResult {
                success: false,
                output: body,
                error: Some(format!("Pushover API returned status {}", status)),
            });
        }

        let api_status = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|json| json.get("status").and_then(|value| value.as_i64()));

        if api_status == Some(1) {
            Ok(ToolResult {
                success: true,
                output: format!(
                    "Pushover notification sent successfully. Response: {}",
                    body
                ),
                error: None,
            })
        } else {
            Ok(ToolResult {
                success: false,
                output: body,
                error: Some("Pushover API returned an application-level error".into()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::security::AutonomyLevel;
    use std::fs;
    use tempfile::TempDir;

    fn test_security(level: AutonomyLevel, max_actions_per_hour: u32) -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: level,
            max_actions_per_hour,
            workspace_dir: std::env::temp_dir(),
            ..SecurityPolicy::default()
        })
    }

    #[test]
    fn pushover_tool_name() {
        let tool = PushoverTool::new(
            test_security(AutonomyLevel::Full, 100),
            PathBuf::from("/tmp"),
        );
        assert_eq!(tool.name(), "pushover");
    }

    #[test]
    fn pushover_tool_description() {
        let tool = PushoverTool::new(
            test_security(AutonomyLevel::Full, 100),
            PathBuf::from("/tmp"),
        );
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn pushover_tool_has_parameters_schema() {
        let tool = PushoverTool::new(
            test_security(AutonomyLevel::Full, 100),
            PathBuf::from("/tmp"),
        );
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].get("message").is_some());
    }

    #[test]
    fn pushover_tool_requires_message() {
        let tool = PushoverTool::new(
            test_security(AutonomyLevel::Full, 100),
            PathBuf::from("/tmp"),
        );
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::Value::String("message".to_string())));
    }

    #[tokio::test]
    async fn credentials_parsed_from_env_file() {
        let tmp = TempDir::new().unwrap();
        let env_path = tmp.path().join(".env");
        fs::write(
            &env_path,
            "PUSHOVER_TOKEN=testtoken123\nPUSHOVER_USER_KEY=userkey456\n",
        )
        .unwrap();

        let tool = PushoverTool::new(
            test_security(AutonomyLevel::Full, 100),
            tmp.path().to_path_buf(),
        );
        let result = tool.get_credentials().await;

        assert!(result.is_ok());
        let (token, user_key) = result.unwrap();
        assert_eq!(token, "testtoken123");
        assert_eq!(user_key, "userkey456");
    }

    #[tokio::test]
    async fn credentials_fail_without_env_file() {
        let tmp = TempDir::new().unwrap();
        let tool = PushoverTool::new(
            test_security(AutonomyLevel::Full, 100),
            tmp.path().to_path_buf(),
        );
        let result = tool.get_credentials().await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn credentials_fail_without_token() {
        let tmp = TempDir::new().unwrap();
        let env_path = tmp.path().join(".env");
        fs::write(&env_path, "PUSHOVER_USER_KEY=userkey456\n").unwrap();

        let tool = PushoverTool::new(
            test_security(AutonomyLevel::Full, 100),
            tmp.path().to_path_buf(),
        );
        let result = tool.get_credentials().await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn credentials_fail_without_user_key() {
        let tmp = TempDir::new().unwrap();
        let env_path = tmp.path().join(".env");
        fs::write(&env_path, "PUSHOVER_TOKEN=testtoken123\n").unwrap();

        let tool = PushoverTool::new(
            test_security(AutonomyLevel::Full, 100),
            tmp.path().to_path_buf(),
        );
        let result = tool.get_credentials().await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn credentials_ignore_comments() {
        let tmp = TempDir::new().unwrap();
        let env_path = tmp.path().join(".env");
        fs::write(&env_path, "# This is a comment\nPUSHOVER_TOKEN=realtoken\n# Another comment\nPUSHOVER_USER_KEY=realuser\n").unwrap();

        let tool = PushoverTool::new(
            test_security(AutonomyLevel::Full, 100),
            tmp.path().to_path_buf(),
        );
        let result = tool.get_credentials().await;

        assert!(result.is_ok());
        let (token, user_key) = result.unwrap();
        assert_eq!(token, "realtoken");
        assert_eq!(user_key, "realuser");
    }

    #[test]
    fn pushover_tool_supports_priority() {
        let tool = PushoverTool::new(
            test_security(AutonomyLevel::Full, 100),
            PathBuf::from("/tmp"),
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("priority").is_some());
    }

    #[test]
    fn pushover_tool_supports_sound() {
        let tool = PushoverTool::new(
            test_security(AutonomyLevel::Full, 100),
            PathBuf::from("/tmp"),
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("sound").is_some());
    }

    #[tokio::test]
    async fn credentials_support_export_and_quoted_values() {
        let tmp = TempDir::new().unwrap();
        let env_path = tmp.path().join(".env");
        fs::write(
            &env_path,
            "export PUSHOVER_TOKEN=\"quotedtoken\"\nPUSHOVER_USER_KEY='quoteduser'\n",
        )
        .unwrap();

        let tool = PushoverTool::new(
            test_security(AutonomyLevel::Full, 100),
            tmp.path().to_path_buf(),
        );
        let result = tool.get_credentials().await;

        assert!(result.is_ok());
        let (token, user_key) = result.unwrap();
        assert_eq!(token, "quotedtoken");
        assert_eq!(user_key, "quoteduser");
    }

    #[tokio::test]
    async fn execute_blocks_readonly_mode() {
        let tool = PushoverTool::new(
            test_security(AutonomyLevel::ReadOnly, 100),
            PathBuf::from("/tmp"),
        );

        let result = tool.execute(json!({"message": "hello"})).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("read-only"));
    }

    #[tokio::test]
    async fn execute_blocks_rate_limit() {
        let tool = PushoverTool::new(test_security(AutonomyLevel::Full, 0), PathBuf::from("/tmp"));

        let result = tool.execute(json!({"message": "hello"})).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("rate limit"));
    }

    #[tokio::test]
    async fn execute_rejects_priority_out_of_range() {
        let tool = PushoverTool::new(
            test_security(AutonomyLevel::Full, 100),
            PathBuf::from("/tmp"),
        );

        let result = tool
            .execute(json!({"message": "hello", "priority": 5}))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.error.unwrap().contains("-2..=2"));
    }
}
